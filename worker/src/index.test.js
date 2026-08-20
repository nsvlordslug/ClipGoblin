import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import worker from "./index.js";

const {
  privateKey: testGitHubAppPrivateKey,
  publicKey: testGitHubAppPublicKey,
} = generateKeyPairSync("rsa", { modulusLength: 2048 });
const TEST_GITHUB_APP_PRIVATE_KEY = testGitHubAppPrivateKey.export({
  type: "pkcs1",
  format: "pem",
}).toString();

function env(overrides = {}) {
  const allow = { limit: async () => ({ success: true }) };
  return {
    TWITCH_CLIENT_ID: "twitch-client",
    TWITCH_CLIENT_SECRET: "twitch-secret",
    YOUTUBE_CLIENT_ID: "youtube-client",
    YOUTUBE_CLIENT_SECRET: "youtube-secret",
    TIKTOK_CLIENT_KEY: "tiktok-key",
    TIKTOK_CLIENT_SECRET: "tiktok-secret",
    GITHUB_APP_ID: "123456",
    GITHUB_APP_INSTALLATION_ID: "789012",
    GITHUB_APP_PRIVATE_KEY: TEST_GITHUB_APP_PRIVATE_KEY,
    OAUTH_RATE_LIMITER: allow,
    OAUTH_GLOBAL_RATE_LIMITER: allow,
    BUG_REPORT_RATE_LIMITER: allow,
    BUG_REPORT_GLOBAL_RATE_LIMITER: allow,
    ...overrides,
  };
}

function request(path, body, init = {}) {
  return new Request(`https://proxy.example${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(init.headers || {}) },
    body: JSON.stringify(body),
    ...init,
  });
}

test("rejects token exchanges with an unregistered redirect URI", async () => {
  const response = await worker.fetch(
    request("/auth/youtube/token", {
      code: "code",
      redirect_uri: "https://attacker.example/callback",
    }),
    env(),
  );
  assert.equal(response.status, 400);
  assert.deepEqual(await response.json(), { error: "invalid_redirect_uri" });
});

test("rejects a request when either rate limit is exhausted", async () => {
  const denied = { limit: async () => ({ success: false }) };
  const response = await worker.fetch(
    request("/auth/twitch/refresh", { refresh_token: "token" }),
    env({ OAUTH_RATE_LIMITER: denied }),
  );
  assert.equal(response.status, 429);
  assert.equal(response.headers.get("retry-after"), "60");
});

test("forwards only validated fields and Worker-held credentials", async (t) => {
  const originalFetch = globalThis.fetch;
  t.after(() => { globalThis.fetch = originalFetch; });

  let upstreamRequest;
  globalThis.fetch = async (url, init) => {
    upstreamRequest = { url, init };
    return new Response(JSON.stringify({ access_token: "access" }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  };

  const response = await worker.fetch(
    request("/auth/twitch/token", {
      code: "one-time-code",
      redirect_uri: "http://localhost:17385",
      ignored: "not-forwarded",
    }),
    env(),
  );

  assert.equal(response.status, 200);
  assert.equal(upstreamRequest.url, "https://id.twitch.tv/oauth2/token");
  const params = new URLSearchParams(upstreamRequest.init.body);
  assert.equal(params.get("client_secret"), "twitch-secret");
  assert.equal(params.get("code"), "one-time-code");
  assert.equal(params.has("ignored"), false);
});

test("requires a valid TikTok PKCE verifier", async () => {
  const response = await worker.fetch(
    request("/auth/tiktok/token", {
      code: "code",
      redirect_uri: "https://nsvlordslug.github.io/ClipGoblin/callback/",
      code_verifier: "short",
    }),
    env(),
  );
  assert.equal(response.status, 400);
  assert.deepEqual(await response.json(), { error: "invalid_code_verifier" });
});

test("creates bug reports with Worker-held credentials and fixed labels", async (t) => {
  const originalFetch = globalThis.fetch;
  t.after(() => { globalThis.fetch = originalFetch; });

  const upstreamRequests = [];
  globalThis.fetch = async (url, init) => {
    const upstreamRequest = { url: String(url), init };
    upstreamRequests.push(upstreamRequest);
    if (upstreamRequest.url.endsWith("/app/installations/789012/access_tokens")) {
      return new Response(JSON.stringify({
        token: "github-installation-token",
        expires_at: "2026-08-20T04:00:00Z",
      }), {
        status: 201,
        headers: { "Content-Type": "application/json" },
      });
    }
    return new Response(JSON.stringify({
      html_url: "https://github.com/nsvlordslug/ClipGoblin/issues/123",
    }), {
      status: 201,
      headers: { "Content-Type": "application/json" },
    });
  };

  const response = await worker.fetch(
    request("/reports/bug", {
      title: "Playback breaks @everyone",
      description: "The preview stays blank.",
      steps: "Open a clip.",
      expected: "The preview plays.",
      page: "Editor",
      severity: "Broken Feature",
      reporterUsername: "tester",
      reporterUserId: "1234",
      appVersion: "1.6.9",
      os: "windows",
      arch: "x86_64",
      logs: "scrubbed logs",
      labels: ["security"],
    }),
    env(),
  );

  assert.equal(response.status, 201);
  assert.deepEqual(await response.json(), {
    success: true,
    issueUrl: "https://github.com/nsvlordslug/ClipGoblin/issues/123",
    error: null,
  });

  assert.equal(upstreamRequests.length, 2);
  const [tokenRequest, issueRequest] = upstreamRequests;
  assert.equal(
    tokenRequest.url,
    "https://api.github.com/app/installations/789012/access_tokens",
  );
  const tokenScope = JSON.parse(tokenRequest.init.body);
  assert.deepEqual(tokenScope, {
    repositories: ["ClipGoblin"],
    permissions: { issues: "write" },
  });

  const jwt = tokenRequest.init.headers.Authorization.replace(/^Bearer /, "");
  const [encodedHeader, encodedPayload, encodedSignature] = jwt.split(".");
  assert.deepEqual(
    JSON.parse(Buffer.from(encodedHeader, "base64url").toString("utf8")),
    { alg: "RS256", typ: "JWT" },
  );
  const jwtPayload = JSON.parse(Buffer.from(encodedPayload, "base64url").toString("utf8"));
  const now = Math.floor(Date.now() / 1000);
  assert.equal(jwtPayload.iss, "123456");
  assert.ok(jwtPayload.iat <= now);
  assert.ok(jwtPayload.exp > now);
  assert.ok(jwtPayload.exp - jwtPayload.iat <= 600);
  assert.equal(
    verify(
      "RSA-SHA256",
      Buffer.from(`${encodedHeader}.${encodedPayload}`),
      testGitHubAppPublicKey,
      Buffer.from(encodedSignature, "base64url"),
    ),
    true,
  );

  assert.equal(
    issueRequest.url,
    "https://api.github.com/repos/nsvlordslug/ClipGoblin/issues",
  );
  assert.equal(issueRequest.init.headers.Authorization, "Bearer github-installation-token");
  const payload = JSON.parse(issueRequest.init.body);
  assert.deepEqual(payload.labels, ["bug", "auto-reported", "severity:high"]);
  assert.equal(payload.title.includes("@everyone"), false);
  assert.equal(payload.body.includes("@everyone"), false);
});

test("rejects malformed GitHub App identifiers before contacting GitHub", async (t) => {
  const originalFetch = globalThis.fetch;
  t.after(() => { globalThis.fetch = originalFetch; });

  let called = false;
  globalThis.fetch = async () => {
    called = true;
    throw new Error("must not be called");
  };

  const response = await worker.fetch(
    request("/reports/bug", {
      title: "Playback issue",
      description: "The preview stays blank.",
      steps: "Open a clip.",
      expected: "The preview plays.",
      page: "Editor",
      severity: "Broken Feature",
      reporterUsername: "tester",
      reporterUserId: "1234",
      appVersion: "1.6.19",
      os: "windows",
      arch: "x86_64",
      logs: "scrubbed logs",
    }),
    env({ GITHUB_APP_INSTALLATION_ID: "../another-installation" }),
  );

  assert.equal(response.status, 502);
  assert.deepEqual(await response.json(), { error: "report_unavailable" });
  assert.equal(called, false);
});

test("rejects invalid bug-report fields before calling GitHub", async (t) => {
  const originalFetch = globalThis.fetch;
  t.after(() => { globalThis.fetch = originalFetch; });

  let called = false;
  globalThis.fetch = async () => {
    called = true;
    throw new Error("must not be called");
  };

  const response = await worker.fetch(
    request("/reports/bug", {
      title: "Bad report",
      description: "Description",
      steps: "Steps",
      expected: "Expected",
      page: "Admin",
      severity: "Critical",
      reporterUsername: "tester",
      reporterUserId: "1234",
      appVersion: "1.6.9",
      os: "windows",
      arch: "x86_64",
      logs: "logs",
    }),
    env(),
  );

  assert.equal(response.status, 400);
  assert.deepEqual(await response.json(), { error: "invalid_page" });
  assert.equal(called, false);
});

test("release builds do not receive bug-report or proxy credentials", () => {
  const releaseWorkflow = readFileSync(
    new URL("../../.github/workflows/release.yml", import.meta.url),
    "utf8",
  );
  const desktopReporter = readFileSync(
    new URL("../../src-tauri/src/commands/bug_report.rs", import.meta.url),
    "utf8",
  );
  const workerSource = readFileSync(new URL("./index.js", import.meta.url), "utf8");

  for (const secretName of [
    "GITHUB_BUG_TOKEN",
    "GITHUB_APP_ID",
    "GITHUB_APP_INSTALLATION_ID",
    "GITHUB_APP_PRIVATE_KEY",
    "DISCORD_WEBHOOK_URL",
    "PROXY_API_KEY",
  ]) {
    assert.equal(releaseWorkflow.includes(secretName), false, `${secretName} reached release build`);
    assert.equal(desktopReporter.includes(secretName), false, `${secretName} reached desktop code`);
  }
  assert.equal(workerSource.includes("GITHUB_BUG_TOKEN"), false, "Worker still accepts a PAT");
});
