import test from "node:test";
import assert from "node:assert/strict";
import worker from "./index.js";

function env(overrides = {}) {
  const allow = { limit: async () => ({ success: true }) };
  return {
    TWITCH_CLIENT_ID: "twitch-client",
    TWITCH_CLIENT_SECRET: "twitch-secret",
    YOUTUBE_CLIENT_ID: "youtube-client",
    YOUTUBE_CLIENT_SECRET: "youtube-secret",
    TIKTOK_CLIENT_KEY: "tiktok-key",
    TIKTOK_CLIENT_SECRET: "tiktok-secret",
    OAUTH_RATE_LIMITER: allow,
    OAUTH_GLOBAL_RATE_LIMITER: allow,
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
