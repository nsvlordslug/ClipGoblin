const MAX_BODY_BYTES = 16 * 1024;
const UPSTREAM_TIMEOUT_MS = 15_000;

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
  async fetch(request, env) {
    const url = new URL(request.url);
    const route = ROUTES[url.pathname];
    if (!route) return jsonResponse({ error: "not_found" }, 404);
    if (request.method !== "POST") {
      return jsonResponse({ error: "method_not_allowed" }, 405, { Allow: "POST" });
    }
    if (!request.headers.get("content-type")?.toLowerCase().startsWith("application/json")) {
      return jsonResponse({ error: "content_type_must_be_json" }, 415);
    }

    const contentLength = Number(request.headers.get("content-length") || 0);
    if (contentLength > MAX_BODY_BYTES) {
      return jsonResponse({ error: "request_too_large" }, 413);
    }

    const ip = request.headers.get("CF-Connecting-IP") || "unknown";
    const limited = await enforceRateLimits(env, `${url.pathname}:${ip}`, url.pathname);
    if (limited) return limited;

    try {
      const body = await readJsonBody(request);
      const fields = validateBody(route, body);
      const response = await exchangeToken(route, fields, env);
      return jsonResponse(response.data, response.status);
    } catch (error) {
      if (error instanceof ClientError) {
        return jsonResponse({ error: error.code }, error.status);
      }
      console.error("OAuth proxy request failed", {
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

async function enforceRateLimits(env, clientKey, globalKey) {
  if (!env.OAUTH_RATE_LIMITER || !env.OAUTH_GLOBAL_RATE_LIMITER) {
    console.error("OAuth rate-limit bindings are missing");
    return jsonResponse({ error: "service_unavailable" }, 503);
  }
  const [client, global] = await Promise.all([
    env.OAUTH_RATE_LIMITER.limit({ key: clientKey }),
    env.OAUTH_GLOBAL_RATE_LIMITER.limit({ key: globalKey }),
  ]);
  if (!client.success || !global.success) {
    return jsonResponse({ error: "rate_limited" }, 429, { "Retry-After": "60" });
  }
  return null;
}

async function readJsonBody(request) {
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_BODY_BYTES) {
    throw new ClientError("request_too_large", 413);
  }
  try {
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

function requireSecret(env, key) {
  const value = env[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Missing Worker secret or variable: ${key}`);
  }
  return value;
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
