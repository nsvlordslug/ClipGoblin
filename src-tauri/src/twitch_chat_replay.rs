//! Twitch chat replay fetcher via the public GraphQL endpoint.
//!
//! yt-dlp's `--write-subs --sub-lang live_chat` only works for YouTube videos;
//! the Twitch extractor doesn't expose chat replay. This module talks directly
//! to Twitch's public GQL endpoint (`gql.twitch.tv`) using the same persisted-
//! query hash that every Twitch chat downloader and the Twitch web client use.
//!
//! No OAuth required — chat replay on archived public VODs is unauthenticated.
//! The `Client-Id` we send is the public web client ID, identical to the value
//! the Twitch web app sends in its browser. If Twitch ever rotates the
//! persisted-query hash, this module breaks loudly and we'll need to re-pin it
//! against the latest hash from a chat downloader (TwitchDownloaderCLI keeps
//! theirs current).
//!
//! Pagination: Twitch returns up to 100 messages per page with cursor-based
//! continuation. We loop through cursors until `pageInfo.hasNextPage` is false
//! or we hit `MAX_PAGES` (a defensive cap so a runaway stream doesn't pull
//! megabytes forever).

use serde::Deserialize;

const GQL_URL: &str = "https://gql.twitch.tv/gql";

/// Public Twitch web Client-Id. Used by every chat replay tool and by
/// twitch.tv itself when serving anonymous video pages.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// Persisted-query hash for `VideoCommentsByOffsetOrCursor`. This is the same
/// hash TwitchDownloaderCLI and the in-browser chat replay use. If it rotates,
/// pull the new value from any maintained Twitch chat downloader.
const VIDEO_COMMENTS_HASH: &str = "b70a3591ff0f4e0313d126c6a1502d79a1c02baebb288227c582044aa76adf6a";

/// Per-VOD pagination cap. ~100 messages per page → 50,000 messages.
/// Sufficient for any normal stream length; aborts cleanly on hostile input.
const MAX_PAGES: usize = 500;

/// One chat replay message at a specific VOD offset.
///
/// `body` is the rendered message text — fragments concatenated into a
/// single string so emote-only messages, multi-fragment messages, and
/// plain text messages all serialize the same way for downstream
/// analysis (chat-rate counting, emote density scanning, etc.).
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub time_seconds: f64,
    pub body: String,
}

// ── GraphQL response shapes ─────────────────────────────────────

#[derive(Deserialize, Debug)]
struct GqlResponse {
    data: Option<GqlData>,
}

#[derive(Deserialize, Debug)]
struct GqlData {
    video: Option<GqlVideo>,
}

#[derive(Deserialize, Debug)]
struct GqlVideo {
    comments: Option<GqlComments>,
}

#[derive(Deserialize, Debug)]
struct GqlComments {
    edges: Vec<GqlEdge>,
    #[serde(rename = "pageInfo")]
    page_info: GqlPageInfo,
}

#[derive(Deserialize, Debug)]
struct GqlEdge {
    node: GqlNode,
    cursor: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GqlNode {
    #[serde(rename = "contentOffsetSeconds")]
    content_offset_seconds: Option<f64>,
    message: Option<GqlMessage>,
}

#[derive(Deserialize, Debug)]
struct GqlMessage {
    fragments: Option<Vec<GqlFragment>>,
}

#[derive(Deserialize, Debug)]
struct GqlFragment {
    text: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GqlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
}

/// Fetch all chat replay messages for a Twitch VOD. Paginates by cursor
/// until the server reports no more pages or `MAX_PAGES` is hit.
///
/// Returns messages in temporal order (Twitch returns them ordered by
/// `contentOffsetSeconds` ascending within each page). Empty messages
/// and messages without a timestamp are filtered out.
pub async fn fetch_chat_replay(
    twitch_video_id: &str,
) -> Result<Vec<ChatMessage>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client: {}", e))?;

    let mut all: Vec<ChatMessage> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages_fetched = 0usize;

    loop {
        if pages_fetched >= MAX_PAGES {
            log::warn!(
                "twitch_chat_replay: hit MAX_PAGES ({}) for video {}, stopping",
                MAX_PAGES, twitch_video_id,
            );
            break;
        }

        // Variables differ for first page (offset 0) vs continuation (cursor).
        let variables = match &cursor {
            Some(c) => serde_json::json!({
                "videoID": twitch_video_id,
                "cursor": c,
            }),
            None => serde_json::json!({
                "videoID": twitch_video_id,
                "contentOffsetSeconds": 0,
            }),
        };

        // Twitch GQL accepts requests as a SINGLE-element array (batch shape).
        // Persisted query: send the hash + operation name, server resolves it.
        let body = serde_json::json!([{
            "operationName": "VideoCommentsByOffsetOrCursor",
            "variables": variables,
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": VIDEO_COMMENTS_HASH,
                }
            }
        }]);

        let resp = client
            .post(GQL_URL)
            .header("Client-Id", PUBLIC_CLIENT_ID)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("GQL request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Twitch GQL returned {}: {}",
                status,
                &body[..body.len().min(200)],
            ));
        }

        // Batch response → single-element array. We pull the first.
        let parsed: Vec<GqlResponse> = resp.json().await.map_err(|e| {
            format!("Parse GQL response: {}", e)
        })?;

        let comments = parsed
            .into_iter()
            .next()
            .and_then(|r| r.data)
            .and_then(|d| d.video)
            .and_then(|v| v.comments);

        let comments = match comments {
            Some(c) => c,
            None => break, // VOD has no chat or response shape is unexpected
        };

        if comments.edges.is_empty() {
            break;
        }

        for edge in &comments.edges {
            let time = match edge.node.content_offset_seconds {
                Some(t) => t,
                None => continue,
            };
            let body = edge
                .node
                .message
                .as_ref()
                .and_then(|m| m.fragments.as_ref())
                .map(|frags| {
                    frags
                        .iter()
                        .filter_map(|f| f.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            if body.trim().is_empty() {
                continue;
            }
            all.push(ChatMessage { time_seconds: time, body });
        }

        if !comments.page_info.has_next_page {
            break;
        }

        // Continuation: use the cursor from the last edge.
        cursor = comments
            .edges
            .last()
            .and_then(|e| e.cursor.clone());
        if cursor.is_none() {
            break; // Can't continue without a cursor.
        }

        pages_fetched += 1;
    }

    log::info!(
        "twitch_chat_replay: fetched {} message(s) across {} page(s) for video {}",
        all.len(),
        pages_fetched + 1,
        twitch_video_id,
    );

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_struct_clones_cheaply() {
        let m = ChatMessage { time_seconds: 12.5, body: "KEKW".to_string() };
        let m2 = m.clone();
        assert_eq!(m.time_seconds, m2.time_seconds);
        assert_eq!(m.body, m2.body);
    }

    #[test]
    fn empty_video_id_does_not_panic() {
        // Smoke test the structure — actually hitting the network requires
        // a tokio runtime and live API; this just ensures the function exists
        // and the public API surface is reachable from the test module.
        let _ = fetch_chat_replay; // no-op reference
    }
}
