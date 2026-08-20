const MAX_BODY_BYTES = 16 * 1024;
const MAX_REPORT_BODY_BYTES = 64 * 1024;
const UPSTREAM_TIMEOUT_MS = 15_000;
const BUG_REPORT_PATH = "/reports/bug";
const BUG_REPORT_REPOSITORY = "nsvlordslug/ClipGoblin";
const BUG_REPORT_REPOSITORY_NAME = "ClipGoblin";
const GITHUB_API_VERSION = "2022-11-28";
const GITHUB_APP_JWT_TTL_SECONDS = 9 * 60;

const REPORT_PAGES = new Set(["VODs", "Clips", "Editor", "Publishing", "Settings", "Other"]);
const REPORT_SEVERITY_LABELS = new Map([
  ["Crash", "severity:critical"],
  ["Broken Feature", "severity:high"],
  ["Cosmetic", "severity:low"],
]);

const ROUTES = {
  "/auth/twitch/token": {
    provider: "twitch",
    grant: "authorization_code",
    redirectUri: "http://localhost:17385",
  },
  "/auth/twitch/refresh": { provider: "twitch", grant: "refresh_token" },
  "/auth/youtube/token": {
    provider: "youtube",
    grant: "authorization_code",
    redirectUri: "http://localhost:17386",
  },
  "/auth/youtube/refresh": { provider: "youtube", grant: "refresh_token" },
  "/auth/tiktok/token": {
    provider: "tiktok",
    grant: "authorization_code",
    redirectUri: "https://nsvlordslug.github.io/ClipGoblin/callback/",
    pkce: true,
  },
  "/auth/tiktok/refresh": { provider: "tiktok", grant: "refresh_token" },
};

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const route = ROUTES[url.pathname];
    const isBugReport = url.pathname === BUG_REPORT_PATH;
    if (!route && !isBugReport) return jsonResponse({ error: "not_found" }, 404);
    if (request.method !== "POST") {
      return jsonResponse({ error: "method_not_allowed" }, 405, { Allow: "POST" });
    }
    if (!request.headers.get("content-type")?.toLowerCase().startsWith("application/json")) {
      return jsonResponse({ error: "content_type_must_be_json" }, 415);
    }

    const contentLength = Number(request.headers.get("content-length") || 0);
    const maxBodyBytes = isBugReport ? MAX_REPORT_BODY_BYTES : MAX_BODY_BYTES;
    if (contentLength > maxBodyBytes) {
      return jsonResponse({ error: "request_too_large" }, 413);
    }

    const ip = request.headers.get("CF-Connecting-IP") || "unknown";
    const limiterPrefix = isBugReport ? "BUG_REPORT" : "OAUTH";
    const limited = await enforceRateLimits(
      env,
      limiterPrefix,
      `${url.pathname}:${ip}`,
      url.pathname,
    );
    if (limited) return limited;

    try {
      const body = await readJsonBody(request, maxBodyBytes);
      if (isBugReport) {
        const fields = validateBugReport(body);
        const response = await createBugReport(fields, env, ctx);
        return jsonResponse(response, 201);
      }
      const fields = validateBody(route, body);
      const response = await exchangeToken(route, fields, env);
      return jsonResponse(response.data, response.status);
    } catch (error) {
      if (error instanceof ClientError) {
        return jsonResponse({ error: error.code }, error.status);
      }
      console.error("Proxy request failed", {
        route: url.pathname,
        error: error instanceof Error ? error.message : String(error),
      });
      return jsonResponse({ error: "internal_error" }, 500);
    }
  },
};

class ClientError extends Error {
  constructor(code, status = 400) {
    super(code);
    this.code = code;
    this.status = status;
  }
}

async function enforceRateLimits(env, prefix, clientKey, globalKey) {
  const clientLimiter = env[`${prefix}_RATE_LIMITER`];
  const globalLimiter = env[`${prefix}_GLOBAL_RATE_LIMITER`];
  if (!clientLimiter || !globalLimiter) {
    console.error(`${prefix} rate-limit bindings are missing`);
    return jsonResponse({ error: "service_unavailable" }, 503);
  }
  const [client, global] = await Promise.all([
    clientLimiter.limit({ key: clientKey }),
    globalLimiter.limit({ key: globalKey }),
  ]);
  if (!client.success || !global.success) {
    return jsonResponse({ error: "rate_limited" }, 429, { "Retry-After": "60" });
  }
  return null;
}

async function readJsonBody(request, maxBodyBytes) {
  if (!request.body) throw new ClientError("invalid_json");

  const reader = request.body.getReader();
  const chunks = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > maxBodyBytes) {
        await reader.cancel();
        throw new ClientError("request_too_large", 413);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }

  try {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    const value = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("not an object");
    }
    return value;
  } catch {
    throw new ClientError("invalid_json");
  }
}

function requiredString(body, key, maxLength) {
  const value = body[key];
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength) {
    throw new ClientError(`invalid_${key}`);
  }
  return value;
}

function validateBody(route, body) {
  if (route.grant === "refresh_token") {
    return { refreshToken: requiredString(body, "refresh_token", 8192) };
  }

  const code = requiredString(body, "code", 4096);
  const redirectUri = requiredString(body, "redirect_uri", 512);
  if (redirectUri !== route.redirectUri) {
    throw new ClientError("invalid_redirect_uri");
  }
  const fields = { code, redirectUri };
  if (route.pkce) {
    const verifier = requiredString(body, "code_verifier", 128);
    if (verifier.length < 43 || !/^[A-Za-z0-9._~-]+$/.test(verifier)) {
      throw new ClientError("invalid_code_verifier");
    }
    fields.codeVerifier = verifier;
  }
  return fields;
}

function optionalString(body, key, maxLength) {
  const value = body[key];
  if (typeof value !== "string" || value.length > maxLength) {
    throw new ClientError(`invalid_${key}`);
  }
  return value;
}

function validateBugReport(body) {
  const page = requiredString(body, "page", 32);
  if (!REPORT_PAGES.has(page)) throw new ClientError("invalid_page");

  const severity = requiredString(body, "severity", 32);
  const severityLabel = REPORT_SEVERITY_LABELS.get(severity);
  if (!severityLabel) throw new ClientError("invalid_severity");

  return {
    title: requiredString(body, "title", 180),
    description: requiredString(body, "description", 10_000),
    steps: requiredString(body, "steps", 10_000),
    expected: optionalString(body, "expected", 5_000),
    page,
    severity,
    severityLabel,
    reporterUsername: requiredString(body, "reporterUsername", 100),
    reporterUserId: requiredString(body, "reporterUserId", 128),
    appVersion: requiredString(body, "appVersion", 32),
    os: requiredString(body, "os", 32),
    arch: requiredString(body, "arch", 32),
    logs: optionalString(body, "logs", 30_000),
  };
}

function requireSecret(env, key) {
  const value = env[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Missing Worker secret or variable: ${key}`);
  }
  return value;
}

function requireNumericSecret(env, key) {
  const value = requireSecret(env, key);
  if (!/^\d+$/.test(value)) {
    throw new Error(`Invalid Worker secret or variable: ${key}`);
  }
  return value;
}

function concatBytes(...arrays) {
  const result = new Uint8Array(arrays.reduce((total, value) => total + value.length, 0));
  let offset = 0;
  for (const value of arrays) {
    result.set(value, offset);
    offset += value.length;
  }
  return result;
}

function encodeAsn1Length(length) {
  if (length < 0x80) return Uint8Array.of(length);
  const bytes = [];
  let remaining = length;
  while (remaining > 0) {
    bytes.unshift(remaining & 0xff);
    remaining >>>= 8;
  }
  return Uint8Array.of(0x80 | bytes.length, ...bytes);
}

function decodePemBlock(pem, label) {
  const match = pem.match(new RegExp(
    `-----BEGIN ${label}-----([\\s\\S]+?)-----END ${label}-----`,
  ));
  if (!match) return null;
  const encoded = match[1].replace(/\s/g, "");
  if (!encoded || !/^[A-Za-z0-9+/]+={0,2}$/.test(encoded)) {
    throw new Error("Invalid GitHub App private key encoding");
  }
  const binary = atob(encoded);
  return Uint8Array.from(binary, character => character.charCodeAt(0));
}

function githubPrivateKeyToPkcs8(pem) {
  const pkcs8 = decodePemBlock(pem, "PRIVATE KEY");
  if (pkcs8) return pkcs8;

  const pkcs1 = decodePemBlock(pem, "RSA PRIVATE KEY");
  if (!pkcs1) throw new Error("Unsupported GitHub App private key format");

  const version = Uint8Array.of(0x02, 0x01, 0x00);
  const rsaAlgorithmIdentifier = Uint8Array.of(
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86,
    0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
  );
  const privateKey = concatBytes(Uint8Array.of(0x04), encodeAsn1Length(pkcs1.length), pkcs1);
  const body = concatBytes(version, rsaAlgorithmIdentifier, privateKey);
  return concatBytes(Uint8Array.of(0x30), encodeAsn1Length(body.length), body);
}

function base64UrlEncode(value) {
  const bytes = typeof value === "string" ? new TextEncoder().encode(value) : value;
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

async function createGitHubAppJwt(env) {
  const appId = requireNumericSecret(env, "GITHUB_APP_ID");
  const privateKeyPem = requireSecret(env, "GITHUB_APP_PRIVATE_KEY");
  const now = Math.floor(Date.now() / 1000);
  const header = base64UrlEncode(JSON.stringify({ alg: "RS256", typ: "JWT" }));
  const payload = base64UrlEncode(JSON.stringify({
    iat: now - 60,
    exp: now + GITHUB_APP_JWT_TTL_SECONDS,
    iss: appId,
  }));
  const signingInput = `${header}.${payload}`;
  const key = await globalThis.crypto.subtle.importKey(
    "pkcs8",
    githubPrivateKeyToPkcs8(privateKeyPem),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await globalThis.crypto.subtle.sign(
    "RSASSA-PKCS1-v1_5",
    key,
    new TextEncoder().encode(signingInput),
  );
  return `${signingInput}.${base64UrlEncode(new Uint8Array(signature))}`;
}

async function createGitHubInstallationToken(env) {
  const installationId = requireNumericSecret(env, "GITHUB_APP_INSTALLATION_ID");
  const appJwt = await createGitHubAppJwt(env);
  const upstream = await fetch(
    `https://api.github.com/app/installations/${installationId}/access_tokens`,
    {
      method: "POST",
      headers: {
        Authorization: `Bearer ${appJwt}`,
        Accept: "application/vnd.github+json",
        "Content-Type": "application/json",
        "User-Agent": "ClipGoblin-Report-Worker",
        "X-GitHub-Api-Version": GITHUB_API_VERSION,
      },
      body: JSON.stringify({
        repositories: [BUG_REPORT_REPOSITORY_NAME],
        permissions: { issues: "write" },
      }),
      signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
    },
  );

  if (!upstream.ok) {
    console.error("GitHub App installation token request was rejected", {
      status: upstream.status,
    });
    throw new Error("GitHub App installation token was rejected");
  }

  let data;
  try {
    data = await upstream.json();
  } catch {
    throw new Error("GitHub App installation token response was invalid");
  }
  if (typeof data.token !== "string" || data.token.length === 0) {
    throw new Error("GitHub App installation token response was incomplete");
  }
  return data.token;
}

async function exchangeToken(route, fields, env) {
  let endpoint;
  let params;

  if (route.provider === "twitch") {
    endpoint = "https://id.twitch.tv/oauth2/token";
    params = {
      client_id: requireSecret(env, "TWITCH_CLIENT_ID"),
      client_secret: requireSecret(env, "TWITCH_CLIENT_SECRET"),
    };
  } else if (route.provider === "youtube") {
    endpoint = "https://oauth2.googleapis.com/token";
    params = {
      client_id: requireSecret(env, "YOUTUBE_CLIENT_ID"),
      client_secret: requireSecret(env, "YOUTUBE_CLIENT_SECRET"),
    };
  } else {
    endpoint = "https://open.tiktokapis.com/v2/oauth/token/";
    params = {
      client_key: requireSecret(env, "TIKTOK_CLIENT_KEY"),
      client_secret: requireSecret(env, "TIKTOK_CLIENT_SECRET"),
    };
  }

  if (route.grant === "refresh_token") {
    params.refresh_token = fields.refreshToken;
    params.grant_type = "refresh_token";
  } else {
    params.code = fields.code;
    params.grant_type = "authorization_code";
    params.redirect_uri = fields.redirectUri;
    if (fields.codeVerifier) params.code_verifier = fields.codeVerifier;
  }

  const upstream = await fetch(endpoint, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(params),
    signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
  });
  const text = await upstream.text();
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    data = { error: "invalid_upstream_response" };
  }
  return { data, status: upstream.status };
}

function safeMarkdown(value) {
  return value
    .replaceAll("@", "@\u200b")
    .replaceAll("```", "` ` `");
}

function buildIssueBody(fields) {
  return `## Bug Report (auto-submitted)

**Reporter:** ${safeMarkdown(fields.reporterUsername)} (\`${safeMarkdown(fields.reporterUserId)}\`)
**Page:** ${fields.page}
**Severity:** ${fields.severity}
**App Version:** ${safeMarkdown(fields.appVersion)}
**OS:** ${safeMarkdown(fields.os)} (${safeMarkdown(fields.arch)})

### Description
${safeMarkdown(fields.description)}

### Steps to Reproduce
${safeMarkdown(fields.steps)}

### Expected Behavior
${safeMarkdown(fields.expected)}

### Recent Logs (scrubbed)
<details>
<summary>Last 100 log lines</summary>

\`\`\`
${safeMarkdown(fields.logs)}
\`\`\`

</details>`;
}

async function createBugReport(fields, env, ctx) {
  let githubToken;
  try {
    githubToken = await createGitHubInstallationToken(env);
  } catch (error) {
    console.error("GitHub App authentication failed", {
      error: error instanceof Error ? error.message : String(error),
    });
    throw new ClientError("report_unavailable", 502);
  }

  let upstream;
  try {
    upstream = await fetch(`https://api.github.com/repos/${BUG_REPORT_REPOSITORY}/issues`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${githubToken}`,
        Accept: "application/vnd.github+json",
        "Content-Type": "application/json",
        "User-Agent": `ClipGoblin-Report-Worker/${fields.appVersion}`,
        "X-GitHub-Api-Version": GITHUB_API_VERSION,
      },
      body: JSON.stringify({
        title: safeMarkdown(fields.title).replaceAll("\r", " ").replaceAll("\n", " "),
        body: buildIssueBody(fields),
        labels: ["bug", "auto-reported", fields.severityLabel],
      }),
      signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
    });
  } catch (error) {
    console.error("GitHub issue request failed", {
      error: error instanceof Error ? error.message : String(error),
    });
    throw new ClientError("report_unavailable", 502);
  }

  if (!upstream.ok) {
    console.error("GitHub issue request was rejected", { status: upstream.status });
    throw new ClientError("report_unavailable", 502);
  }

  let data;
  try {
    data = await upstream.json();
  } catch {
    throw new ClientError("report_unavailable", 502);
  }
  if (typeof data.html_url !== "string" || !data.html_url.startsWith(
    `https://github.com/${BUG_REPORT_REPOSITORY}/issues/`,
  )) {
    throw new ClientError("report_unavailable", 502);
  }

  const notification = notifyDiscordOfBugReport(fields.title, data.html_url, env);
  if (notification) {
    if (ctx?.waitUntil) ctx.waitUntil(notification);
    else await notification;
  }

  return { success: true, issueUrl: data.html_url, error: null };
}

function notifyDiscordOfBugReport(title, issueUrl, env) {
  if (typeof env.DISCORD_WEBHOOK_URL !== "string" || env.DISCORD_WEBHOOK_URL.length === 0) {
    return null;
  }
  return fetch(env.DISCORD_WEBHOOK_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      content: `New ClipGoblin bug report: ${safeMarkdown(title)}\n${issueUrl}`,
      allowed_mentions: { parse: [] },
    }),
    signal: AbortSignal.timeout(UPSTREAM_TIMEOUT_MS),
  }).catch((error) => {
    console.error("Discord bug-report notification failed", {
      error: error instanceof Error ? error.message : String(error),
    });
  });
}

function jsonResponse(data, status, extraHeaders = {}) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      ...extraHeaders,
    },
  });
}
