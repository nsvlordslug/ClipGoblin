export default {
  async fetch(request, env) {
    if (request.method === "OPTIONS") {
      return new Response(null, { headers: corsHeaders(env) });
    }

    const apiKey = request.headers.get("X-Proxy-Key");
    if (apiKey !== env.PROXY_API_KEY) {
      return jsonResponse({ error: "unauthorized" }, 401, env);
    }

    const url = new URL(request.url);
    const path = url.pathname;

    try {
      if (path === "/auth/twitch/token") return await twitchToken(request, env);
      if (path === "/auth/twitch/refresh") return await twitchRefresh(request, env);
      if (path === "/auth/youtube/token") return await youtubeToken(request, env);
      if (path === "/auth/youtube/refresh") return await youtubeRefresh(request, env);
      if (path === "/auth/tiktok/token") return await tiktokToken(request, env);
      if (path === "/auth/tiktok/refresh") return await tiktokRefresh(request, env);
      return jsonResponse({ error: "not found" }, 404, env);
    } catch (err) {
      return jsonResponse({ error: "internal error" }, 500, env);
    }
  },
};

async function twitchToken(request, env) {
  const { code, redirect_uri } = await request.json();
  const resp = await fetch("https://id.twitch.tv/oauth2/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: env.TWITCH_CLIENT_ID,
      client_secret: env.TWITCH_CLIENT_SECRET,
      code,
      grant_type: "authorization_code",
      redirect_uri,
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

async function twitchRefresh(request, env) {
  const { refresh_token } = await request.json();
  const resp = await fetch("https://id.twitch.tv/oauth2/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: env.TWITCH_CLIENT_ID,
      client_secret: env.TWITCH_CLIENT_SECRET,
      refresh_token,
      grant_type: "refresh_token",
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

async function youtubeToken(request, env) {
  const { code, redirect_uri } = await request.json();
  const resp = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: env.YOUTUBE_CLIENT_ID,
      client_secret: env.YOUTUBE_CLIENT_SECRET,
      code,
      grant_type: "authorization_code",
      redirect_uri,
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

async function youtubeRefresh(request, env) {
  const { refresh_token } = await request.json();
  const resp = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: env.YOUTUBE_CLIENT_ID,
      client_secret: env.YOUTUBE_CLIENT_SECRET,
      refresh_token,
      grant_type: "refresh_token",
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

async function tiktokToken(request, env) {
  const { code, redirect_uri, code_verifier } = await request.json();
  const resp = await fetch("https://open.tiktokapis.com/v2/oauth/token/", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_key: env.TIKTOK_CLIENT_KEY,
      client_secret: env.TIKTOK_CLIENT_SECRET,
      code,
      grant_type: "authorization_code",
      redirect_uri,
      code_verifier,
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

async function tiktokRefresh(request, env) {
  const { refresh_token } = await request.json();
  const resp = await fetch("https://open.tiktokapis.com/v2/oauth/token/", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_key: env.TIKTOK_CLIENT_KEY,
      client_secret: env.TIKTOK_CLIENT_SECRET,
      refresh_token,
      grant_type: "refresh_token",
    }),
  });
  return jsonResponse(await resp.json(), resp.status, env);
}

function corsHeaders(env) {
  return {
    "Access-Control-Allow-Origin": env.ALLOWED_ORIGIN || "*",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, X-Proxy-Key",
    "Access-Control-Max-Age": "86400",
  };
}

function jsonResponse(data, status, env) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json",
      ...corsHeaders(env),
    },
  });
}
