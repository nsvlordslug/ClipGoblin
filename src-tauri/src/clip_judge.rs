//! AI clip-worthiness judge (opt-in, BYOK).
//!
//! Reads the VOD's timestamped transcript and rates moments by whether they are
//! actually clip-worthy — banter/roasts, plays/clutches, scares, hype — catching
//! quiet moments the loudness/chat signals miss and vetoing loud-but-empty ones
//! (e.g. laughing while explaining OBS). The signal pipeline hears volume; this
//! reads the words. Mirrors `vision_signal.rs`'s provider-abstraction pattern,
//! but for text. See `.claude/specs/2026-06-14-detection-ai-clip-judge-design.md`.
//!
//! Pure logic (prompt build, response parse, timestamp validation) is unit-
//! tested with no network. The HTTP call lives in `call_llm` / `judge`.

use crate::ai_provider::Provider;
use crate::commands::vod::TranscriptResult;
use crate::error::AppError;

/// One clip-worthy moment the AI identified in the transcript.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JudgedMoment {
    pub start_sec: f64,
    pub end_sec: f64,
    /// banter | play | scare | hype | other  (the AI's own label).
    pub category: String,
    /// Clip-worthiness, 0.0–1.0 (the AI's 0–100 normalized).
    pub score: f64,
    /// One-line justification — for the gate-A log and future display.
    pub reason: String,
}

/// Build the timestamped transcript text the judge reads. Timestamps are whole
/// seconds from the start of the VOD, matching the `start`/`end` the model must
/// return — so it never has to convert m:ss → seconds (a common hallucination).
pub fn build_transcript_text(t: &TranscriptResult) -> String {
    let mut s = String::with_capacity(t.full_text.len() + t.segments.len() * 8);
    for seg in &t.segments {
        let line = seg.text.trim();
        if line.is_empty() {
            continue;
        }
        s.push_str(&format!("[{:.0}] {}\n", seg.start, line));
    }
    s
}

/// The judge prompt. Encodes the creator-confirmed criteria (all four) and the
/// anti-criteria (logistics/dead air), and forbids loudness-alone reasoning.
fn build_judge_prompt(transcript_text: &str, vod_title: &str, duration: f64) -> String {
    format!(
        r#"You are an elite stream-clip editor reviewing a VOD transcript to find the moments worth clipping for TikTok / Shorts / Reels.

Stream: "{title}" | Duration: {dur:.0}s

CLIP-WORTHY — find these:
- BANTER / ROASTS: friends ribbing each other, jokes landing, savage one-liners — INCLUDING quiet, deadpan delivery that would NOT spike audio or chat.
- PLAYS / CLUTCHES: skillful gameplay, clutch escapes or saves, wins, outplays (or a hilarious failure).
- SCARES / REACTIONS: jumpscares, genuine shock, panic, rage.
- HYPE: collective excitement, celebrations, "OH MY GOD" energy.

NOT CLIP-WORTHY — reject, score these LOW:
- Explaining settings / OBS / co-streaming logistics, mic checks, "what video are you watching", general housekeeping.
- Dead air, mundane chat, transitions.
- Loudness or laughter ALONE is NOT clip-worthy. There must be an actual moment — a joke that lands, a play, a scare. A loud laugh while explaining a menu is NOT a clip.

Pick a tight 15-45 second window for each moment, using ONLY timestamps that appear in the transcript below (they are seconds from the start). Score each 0-100 on how clip-worthy it is. Be SELECTIVE — a strong VOD has roughly 6-15 real moments, not dozens.

Return ONLY this JSON, no prose:
{{"moments":[{{"start":<seconds>,"end":<seconds>,"category":"banter|play|scare|hype","score":<0-100>,"reason":"<one short line>"}}]}}
If nothing qualifies: {{"moments":[]}}

TRANSCRIPT:
{transcript}"#,
        title = vod_title,
        dur = duration,
        transcript = transcript_text,
    )
}

/// Parse the model's JSON into validated moments. Robust to markdown fences and
/// prose: extracts the first `{...}` block. Every timestamp is validated against
/// the VOD duration — hallucinated / out-of-range / degenerate windows are
/// dropped, never trusted blindly.
fn parse_judge_response(text: &str, duration: f64) -> Vec<JudgedMoment> {
    let json_str = match (text.find('{'), text.rfind('}')) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("clip_judge: could not parse model JSON: {e}");
            return Vec::new();
        }
    };
    let arr = parsed
        .get("moments")
        .or_else(|| parsed.get("clips"))
        .or_else(|| parsed.get("highlights"))
        .and_then(|v| v.as_array());
    let Some(arr) = arr else { return Vec::new() };

    arr.iter()
        .filter_map(|m| {
            let start = m.get("start").and_then(|v| v.as_f64())?;
            let end = m.get("end").and_then(|v| v.as_f64())?;
            if !start.is_finite() || !end.is_finite() {
                return None;
            }
            // Clamp into the VOD, drop degenerate / out-of-range windows.
            let start = start.max(0.0);
            let end = end.min(duration);
            if start >= duration || end - start < 1.0 {
                return None;
            }
            let raw = m.get("score").and_then(|v| v.as_f64()).unwrap_or(50.0);
            let score = (raw / 100.0).clamp(0.0, 1.0);
            let category = m
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("other")
                .to_string();
            let reason = m
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(JudgedMoment { start_sec: start, end_sec: end, category, score, reason })
        })
        .collect()
}

/// Call the resolved BYOK provider with a text prompt. Returns the raw response
/// text plus the provider-reported (input, output) token counts for cost
/// logging — falling back to a length estimate when usage isn't returned.
async fn call_llm(
    provider: Provider,
    api_key: &str,
    model: &str,
    prompt: &str,
) -> Result<(String, u64, u64), AppError> {
    let client = reqwest::Client::new();
    let est = (prompt.len() / 4) as u64;
    match provider {
        Provider::Claude => {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 4096,
                "messages": [{ "role": "user", "content": prompt }]
            });
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::Api(format!("Claude request failed: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let b = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("Claude API {status}: {b}")));
            }
            let j: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Api(format!("Claude response parse: {e}")))?;
            let text = j["content"][0]["text"].as_str().unwrap_or("").to_string();
            let tin = j["usage"]["input_tokens"].as_u64().unwrap_or(est);
            let tout = j["usage"]["output_tokens"].as_u64().unwrap_or((text.len() / 4) as u64);
            Ok((text, tin, tout))
        }
        Provider::OpenAI => {
            let body = serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": prompt }],
                "response_format": { "type": "json_object" }
            });
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::Api(format!("OpenAI request failed: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let b = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("OpenAI API {status}: {b}")));
            }
            let j: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Api(format!("OpenAI response parse: {e}")))?;
            let text = j["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
            let tin = j["usage"]["prompt_tokens"].as_u64().unwrap_or(est);
            let tout = j["usage"]["completion_tokens"].as_u64().unwrap_or((text.len() / 4) as u64);
            Ok((text, tin, tout))
        }
        Provider::Gemini => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
            );
            let body = serde_json::json!({
                "contents": [{ "parts": [{ "text": prompt }] }],
                "generationConfig": { "temperature": 0.3, "responseMimeType": "application/json" }
            });
            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .header("x-goog-api-key", api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::Api(format!("Gemini request failed: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let b = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("Gemini API {status}: {b}")));
            }
            let j: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Api(format!("Gemini response parse: {e}")))?;
            let text = j["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tin = j["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(est);
            let tout = j["usageMetadata"]["candidatesTokenCount"]
                .as_u64()
                .unwrap_or((text.len() / 4) as u64);
            Ok((text, tin, tout))
        }
        Provider::Free => Err(AppError::Api(
            "AI clip detection requires a configured AI provider (BYOK)".into(),
        )),
    }
}

/// Run the judge over a VOD transcript. Returns the validated clip-worthy
/// moments plus (input, output) token counts for cost logging. Errors (no key,
/// API failure) propagate so the caller can fall back to the signal-only path.
pub async fn judge(
    provider: Provider,
    api_key: &str,
    model: &str,
    transcript: &TranscriptResult,
    vod_title: &str,
    duration: f64,
) -> Result<(Vec<JudgedMoment>, u64, u64), AppError> {
    let transcript_text = build_transcript_text(transcript);
    if transcript_text.trim().is_empty() {
        return Ok((Vec::new(), 0, 0));
    }
    let prompt = build_judge_prompt(&transcript_text, vod_title, duration);
    log::info!(
        "clip_judge: judging {} transcript chars via {} ({})",
        transcript_text.len(),
        match provider {
            Provider::Claude => "Claude",
            Provider::OpenAI => "OpenAI",
            Provider::Gemini => "Gemini",
            Provider::Free => "Free",
        },
        model
    );
    let (response, tin, tout) = call_llm(provider, api_key, model, &prompt).await?;
    let moments = parse_judge_response(&response, duration);
    log::info!("clip_judge: {} clip-worthy moments returned", moments.len());
    Ok((moments, tin, tout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::vod::TranscriptSegment;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment { start, end, text: text.to_string(), words: Vec::new() }
    }

    fn transcript(segs: Vec<TranscriptSegment>) -> TranscriptResult {
        let full = segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
        TranscriptResult {
            segments: segs,
            full_text: full,
            language: "en".to_string(),
            keywords_found: Vec::new(),
        }
    }

    #[test]
    fn transcript_text_is_seconds_stamped_and_skips_blanks() {
        let t = transcript(vec![
            seg(12.0, 14.0, "what video are you watching"),
            seg(20.0, 21.0, "   "),
            seg(37.4, 40.0, "that was a savage roast"),
        ]);
        let out = build_transcript_text(&t);
        assert!(out.contains("[12] what video are you watching"));
        assert!(out.contains("[37] that was a savage roast"));
        assert!(!out.contains("[20]"), "blank segment should be skipped");
    }

    #[test]
    fn prompt_carries_criteria_and_anti_criteria() {
        let p = build_judge_prompt("[10] hi", "My Stream", 3600.0);
        assert!(p.contains("My Stream"));
        assert!(p.contains("3600"));
        assert!(p.contains("BANTER"));
        assert!(p.contains("deadpan"));
        assert!(p.contains("NOT CLIP-WORTHY"));
        assert!(p.contains("OBS"));
        assert!(p.contains("[10] hi"));
        assert!(p.contains("\"moments\""));
    }

    #[test]
    fn parses_valid_moments() {
        let json = r#"{"moments":[
            {"start":37.0,"end":62.0,"category":"banter","score":86,"reason":"savage roast"},
            {"start":120.0,"end":150.0,"category":"play","score":74,"reason":"clutch save"}
        ]}"#;
        let m = parse_judge_response(json, 5938.0);
        assert_eq!(m.len(), 2);
        assert!((m[0].start_sec - 37.0).abs() < 1e-6);
        assert!((m[0].score - 0.86).abs() < 1e-6);
        assert_eq!(m[0].category, "banter");
        assert_eq!(m[0].reason, "savage roast");
    }

    #[test]
    fn parse_strips_markdown_and_prose() {
        let text = "Here you go:\n```json\n{\"moments\":[{\"start\":5,\"end\":25,\"score\":70,\"category\":\"hype\",\"reason\":\"x\"}]}\n```\nThanks!";
        let m = parse_judge_response(text, 600.0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn parse_drops_hallucinated_and_degenerate_windows() {
        // end past duration is clamped; start past duration and zero-length drop.
        let json = r#"{"moments":[
            {"start":10.0,"end":99999.0,"score":80,"category":"play","reason":"clamped end"},
            {"start":9000.0,"end":9020.0,"score":80,"category":"play","reason":"out of range"},
            {"start":50.0,"end":50.2,"score":80,"category":"play","reason":"too short"}
        ]}"#;
        let m = parse_judge_response(json, 600.0);
        assert_eq!(m.len(), 1, "only the clamped-end moment survives");
        assert!((m[0].end_sec - 600.0).abs() < 1e-6, "end clamped to duration");
    }

    #[test]
    fn parse_empty_and_garbage_are_safe() {
        assert!(parse_judge_response(r#"{"moments":[]}"#, 600.0).is_empty());
        assert!(parse_judge_response("no json here", 600.0).is_empty());
        assert!(parse_judge_response("{bad json", 600.0).is_empty());
    }
}
