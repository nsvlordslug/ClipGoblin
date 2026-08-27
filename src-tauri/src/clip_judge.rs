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
use crate::commands::vod::{TranscriptResult, TranscriptSegment};
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

/// A creator-approved moment supplied as a compact example of personal taste.
/// The app preserves this exact moment separately; the judge uses only its
/// transcript to find additional moments with similar appeal.
#[derive(Debug, Clone)]
pub struct ApprovedMomentExample {
    pub start_sec: f64,
    pub end_sec: f64,
    pub transcript: String,
}

/// Normalize a segment's text for repetition/dup comparison: lowercased, trimmed,
/// inner whitespace collapsed, trailing punctuation stripped. So "Wait, wait!" and
/// "wait wait" compare equal when detecting a loop.
fn normalize_for_dedup(text: &str) -> String {
    let lowered = text.to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// True if a segment is a whisper artifact carrying no real dialogue: empty after
/// trimming, all-punctuation, or a SINGLE fully-wrapped non-speech token — the
/// whole segment bracketed/parenthesized/asterisked/music-noted (`[_TT_]`,
/// `[BLANK_AUDIO]`, `[Music]`, `(music)`, `(applause)`, `*laughs*`, `♪ … ♪`).
/// A line with dialogue OUTSIDE the brackets (e.g. `(laughing) that was great`)
/// is real and is kept — it doesn't end with the closing bracket.
fn is_artifact_segment(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // No alphanumerics at all → pure punctuation/symbols (e.g. "...", "??", "♪♪").
    if !t.chars().any(|c| c.is_alphanumeric()) {
        return true;
    }
    // The ENTIRE segment is one wrapped token → non-speech tag.
    let wrapped = (t.starts_with('[') && t.ends_with(']'))
        || (t.starts_with('(') && t.ends_with(')'))
        || (t.starts_with('*') && t.ends_with('*'))
        || (t.starts_with('♪') && t.ends_with('♪'));
    wrapped
}

/// Clean whisper output before it reaches the judge: drop empty/`[_TT_]`-style
/// artifacts, and collapse repetition-loops — a short phrase (≤ MAX_LOOP_WORDS)
/// repeated across many consecutive segments (whisper's classic "wait, wait,
/// wait…" / "I'm not sure." ×N stall) into ONE segment spanning the run. This
/// also covers simple adjacent-duplicate de-duplication. Cuts judge input tokens
/// ~20–40% and removes noise that skews the verdict.
///
/// A run of identical normalized text is only collapsed when the phrase is short
/// (a legitimately repeated long sentence is rare and informative); a 2× repeat
/// is left alone (`MIN_LOOP_RUN`), so real back-to-back callouts survive.
pub fn clean_segments(segs: &[TranscriptSegment]) -> Vec<TranscriptSegment> {
    /// Phrases longer than this (in words) are never treated as a loop.
    const MAX_LOOP_WORDS: usize = 6;
    /// Need at least this many identical consecutive segments to collapse.
    const MIN_LOOP_RUN: usize = 3;

    // Pass 1: drop artifacts, keeping original order/timing.
    let kept: Vec<&TranscriptSegment> = segs
        .iter()
        .filter(|s| !is_artifact_segment(&s.text))
        .collect();

    // Pass 2: collapse runs of identical normalized text.
    let mut out: Vec<TranscriptSegment> = Vec::with_capacity(kept.len());
    let mut i = 0usize;
    while i < kept.len() {
        let norm = normalize_for_dedup(&kept[i].text);
        // Extend the run while the next segment normalizes to the same text.
        let mut j = i + 1;
        while j < kept.len() && normalize_for_dedup(&kept[j].text) == norm {
            j += 1;
        }
        let run_len = j - i;
        let word_count = norm.split_whitespace().count();
        let is_loop = run_len >= MIN_LOOP_RUN && word_count <= MAX_LOOP_WORDS && !norm.is_empty();
        // An exact adjacent duplicate (run of 2 identical) also collapses — no
        // value in feeding the judge the same line twice in a row.
        let is_adjacent_dup = run_len >= 2 && !norm.is_empty();

        if is_loop || is_adjacent_dup {
            // Keep one segment spanning the whole run (first text, run's time span).
            let mut merged = kept[i].clone();
            merged.end = kept[j - 1].end.max(merged.end);
            merged.words = Vec::new(); // word timings no longer line up after a collapse
            out.push(merged);
        } else {
            for seg in &kept[i..j] {
                out.push((*seg).clone());
            }
        }
        i = j;
    }
    out
}

/// Build the timestamped transcript text the judge reads. Timestamps are whole
/// seconds from the start of the VOD, matching the `start`/`end` the model must
/// return — so it never has to convert m:ss → seconds (a common hallucination).
/// Segments are cleaned first (`clean_segments`): artifacts dropped, whisper
/// repetition-loops collapsed — fewer tokens, clearer signal.
pub fn build_transcript_text(t: &TranscriptResult) -> String {
    build_transcript_text_excluding(t, &[])
}

fn build_transcript_text_excluding(
    t: &TranscriptResult,
    reserved_windows: &[(f64, f64)],
) -> String {
    let cleaned = clean_segments(&t.segments);
    let mut s = String::with_capacity(t.full_text.len() + cleaned.len() * 8);
    for seg in &cleaned {
        if reserved_windows
            .iter()
            .any(|(start, end)| seg.end > *start && seg.start < *end)
        {
            continue;
        }
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
fn build_judge_prompt(
    transcript_text: &str,
    vod_title: &str,
    duration: f64,
    rejected_windows: &[(f64, f64)],
    approved_examples: &[ApprovedMomentExample],
) -> String {
    let exclusion_guidance = if rejected_windows.is_empty() {
        String::new()
    } else {
        let ranges = rejected_windows
            .iter()
            .take(50)
            .map(|(start, end)| format!("- {start:.0}-{end:.0}s"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nPREVIOUSLY REJECTED BY THIS CREATOR:\n\
Do NOT return these events again, including shifted, shortened, or widened versions that overlap them. Find genuinely different moments elsewhere in the transcript.\n\
{ranges}\n"
        )
    };
    let approved_guidance = if approved_examples.is_empty() {
        String::new()
    } else {
        let examples = approved_examples
            .iter()
            .take(12)
            .map(|example| {
                let transcript = example
                    .transcript
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "- {:.0}-{:.0}s: \"{}\"",
                    example.start_sec, example.end_sec, transcript
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nCREATOR-APPROVED TASTE EXAMPLES:\n\
These exact clips are already preserved by the app. Do NOT return the same events. Use their humor, energy, payoff, or gameplay value as evidence of this creator's taste, then find ADDITIONAL moments elsewhere with similar appeal.\n\
{examples}\n"
        )
    };
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
{exclusions}
{approved}

Pick a tight 15-45 second window for each moment, using ONLY timestamps that appear in the transcript below (they are seconds from the start). Score each 0-100 on how clip-worthy it is. Be SELECTIVE — a strong VOD has roughly 6-15 real moments, not dozens.

Return ONLY this JSON, no prose:
{{"moments":[{{"start":<seconds>,"end":<seconds>,"category":"banter|play|scare|hype","score":<0-100>,"reason":"<one short line>"}}]}}
If nothing qualifies: {{"moments":[]}}

TRANSCRIPT:
{transcript}"#,
        title = vod_title,
        dur = duration,
        exclusions = exclusion_guidance,
        approved = approved_guidance,
        transcript = transcript_text,
    )
}

fn meaningfully_overlaps_excluded_window(
    start: f64,
    end: f64,
    excluded_start: f64,
    excluded_end: f64,
) -> bool {
    if end <= start || excluded_end <= excluded_start {
        return false;
    }
    let overlap = (end.min(excluded_end) - start.max(excluded_start)).max(0.0);
    let shorter = (end - start).min(excluded_end - excluded_start).max(0.001);
    overlap >= 5.0 && overlap / shorter >= 0.20
}

fn validated_moment(value: &serde_json::Value, duration: f64) -> Option<JudgedMoment> {
    let start = value.get("start").and_then(|v| v.as_f64())?;
    let end = value.get("end").and_then(|v| v.as_f64())?;
    if !start.is_finite() || !end.is_finite() {
        return None;
    }
    let start = start.max(0.0);
    let end = end.min(duration);
    if start >= duration || end - start < 1.0 {
        return None;
    }
    let raw = value.get("score").and_then(|v| v.as_f64()).unwrap_or(50.0);
    let score = (raw / 100.0).clamp(0.0, 1.0);
    let category = value
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("other")
        .to_string();
    let reason = value
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(JudgedMoment {
        start_sec: start,
        end_sec: end,
        category,
        score,
        reason,
    })
}

fn moments_from_value(value: &serde_json::Value, duration: f64) -> Option<Vec<JudgedMoment>> {
    let arr = value
        .get("moments")
        .or_else(|| value.get("clips"))
        .or_else(|| value.get("highlights"))
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array())?;
    Some(
        arr.iter()
            .filter_map(|moment| validated_moment(moment, duration))
            .collect(),
    )
}

/// Return every balanced JSON object, including nested objects, while ignoring
/// braces inside quoted strings. This lets us retain valid sibling moments when
/// a model makes a syntax mistake in just one array item.
fn balanced_json_objects(text: &str) -> Vec<(usize, &str)> {
    let mut starts = Vec::new();
    let mut objects = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => starts.push(index),
            '}' => {
                if let Some(start) = starts.pop() {
                    objects.push((start, &text[start..index + ch.len_utf8()]));
                }
            }
            _ => {}
        }
    }
    objects.sort_by_key(|(start, _)| *start);
    objects
}

/// Parse the model's JSON into validated moments. Robust to markdown fences and
/// prose: extracts the first `{...}` block. If the wrapper is malformed, valid
/// sibling moment objects are salvaged individually rather than discarding the
/// whole paid response. Every timestamp is validated against the VOD duration.
fn parse_judge_response(text: &str, duration: f64) -> Result<Vec<JudgedMoment>, String> {
    let json_str = match (text.find('{'), text.rfind('}')) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => return Err("response did not contain a JSON object".to_string()),
    };

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(value) => {
            return moments_from_value(&value, duration)
                .ok_or_else(|| "response JSON did not contain a moments array".to_string());
        }
        Err(strict_error) => {
            let mut salvaged = Vec::new();
            for (_, object) in balanced_json_objects(json_str) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(object) else {
                    continue;
                };
                let Some(moment) = validated_moment(&value, duration) else {
                    continue;
                };
                let duplicate = salvaged.iter().any(|existing: &JudgedMoment| {
                    (existing.start_sec - moment.start_sec).abs() < 0.001
                        && (existing.end_sec - moment.end_sec).abs() < 0.001
                        && existing.category == moment.category
                });
                if !duplicate {
                    salvaged.push(moment);
                }
            }
            if salvaged.is_empty() {
                Err(format!("could not parse model JSON: {strict_error}"))
            } else {
                log::warn!(
                    "clip_judge: model JSON was malformed ({strict_error}); salvaged {} valid moment(s)",
                    salvaged.len(),
                );
                Ok(salvaged)
            }
        }
    }
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
            let tout = j["usage"]["output_tokens"]
                .as_u64()
                .unwrap_or((text.len() / 4) as u64);
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
            let text = j["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tin = j["usage"]["prompt_tokens"].as_u64().unwrap_or(est);
            let tout = j["usage"]["completion_tokens"]
                .as_u64()
                .unwrap_or((text.len() / 4) as u64);
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
            let tin = j["usageMetadata"]["promptTokenCount"]
                .as_u64()
                .unwrap_or(est);
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
    rejected_windows: &[(f64, f64)],
    approved_examples: &[ApprovedMomentExample],
) -> Result<(Vec<JudgedMoment>, u64, u64), AppError> {
    let mut reserved_windows = rejected_windows.to_vec();
    reserved_windows.extend(
        approved_examples
            .iter()
            .map(|example| (example.start_sec, example.end_sec)),
    );
    let transcript_text = build_transcript_text_excluding(transcript, &reserved_windows);
    if transcript_text.trim().is_empty() {
        return Ok((Vec::new(), 0, 0));
    }
    let prompt = build_judge_prompt(
        &transcript_text,
        vod_title,
        duration,
        rejected_windows,
        approved_examples,
    );
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
    let mut moments = parse_judge_response(&response, duration)
        .map_err(|error| AppError::Api(format!("AI clip judge returned invalid JSON: {error}")))?;
    let before_exclusions = moments.len();
    moments.retain(|moment| {
        !reserved_windows.iter().any(|(start, end)| {
            meaningfully_overlaps_excluded_window(moment.start_sec, moment.end_sec, *start, *end)
        })
    });
    if moments.len() < before_exclusions {
        log::info!(
            "clip_judge: dropped {} returned moment(s) that overlapped reserved review windows",
            before_exclusions - moments.len(),
        );
    }
    log::info!("clip_judge: {} clip-worthy moments returned", moments.len());
    Ok((moments, tin, tout))
}

// ═══════════════════════════════════════════════════════════════════
//  Sonnet final-pass — taste re-rank over the top survivors (cheap)
// ═══════════════════════════════════════════════════════════════════

/// Build the final-pass prompt: the bulk (Haiku) judge already found the
/// candidates; one stronger model now reads only their short snippets and
/// curates the final cut. Candidates are presented by index so the model
/// returns a compact ordering, not re-derived timestamps.
fn build_final_pass_prompt(candidates: &[(JudgedMoment, String)], vod_title: &str) -> String {
    let mut list = String::new();
    for (i, (m, snippet)) in candidates.iter().enumerate() {
        let snip = snippet.trim();
        // Cap snippet length on a CHAR boundary (byte-slicing can panic on UTF-8).
        let snip: String = snip.chars().take(600).collect();
        list.push_str(&format!(
            "{i}. [{start:.0}-{end:.0}s] ({cat}, judge {score:.0}) {snip}\n",
            i = i,
            start = m.start_sec,
            end = m.end_sec,
            cat = m.category,
            score = m.score * 100.0,
            snip = snip,
        ));
    }
    format!(
        r#"You are the senior editor making the FINAL clip cut for "{title}". A first-pass judge already shortlisted these moments; pick and ORDER the ones that will actually perform as TikTok / Shorts / Reels, best first. Drop weak or redundant ones. Do NOT invent new moments — only choose from the numbered list. Keep the strong ones; a tight final set is better than a long one.

CANDIDATES (index. [start-end] (category, first-pass score) snippet):
{list}
Return ONLY this JSON, best first, no prose:
{{"final":[{{"index":<n>,"score":<0-100>}}]}}"#,
        title = vod_title,
        list = list,
    )
}

/// Parse the final-pass response: ordered indices into the candidate list, with
/// an optional re-score. Out-of-range / duplicate indices are dropped. Returns
/// the curated moments in the model's order (the new score overrides the
/// first-pass one; start/end/category/reason are preserved from the candidate).
fn parse_final_pass(text: &str, candidates: &[(JudgedMoment, String)]) -> Vec<JudgedMoment> {
    let json_str = match (text.find('{'), text.rfind('}')) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("clip_judge final-pass: could not parse model JSON: {e}");
            return Vec::new();
        }
    };
    let arr = parsed
        .get("final")
        .or_else(|| parsed.get("selected"))
        .or_else(|| parsed.get("clips"))
        .and_then(|v| v.as_array());
    let Some(arr) = arr else { return Vec::new() };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in arr {
        // Accept either {"index":n,"score":s} or a bare index number.
        let idx = item
            .get("index")
            .and_then(|v| v.as_u64())
            .or_else(|| item.as_u64());
        let Some(idx) = idx else { continue };
        let idx = idx as usize;
        if idx >= candidates.len() || !seen.insert(idx) {
            continue;
        }
        let mut m = candidates[idx].0.clone();
        if let Some(s) = item.get("score").and_then(|v| v.as_f64()) {
            m.score = (s / 100.0).clamp(0.0, 1.0);
        }
        out.push(m);
    }
    out
}

/// Run ONE cheap Sonnet final-pass over the top survivors (their snippets only,
/// not the VOD) to pick/re-order the final clip set. Returns the curated moments
/// plus (input, output) token counts for cost logging. Errors propagate so the
/// caller can fall back to the first-pass ranking.
pub async fn final_pass(
    provider: Provider,
    api_key: &str,
    model: &str,
    candidates: &[(JudgedMoment, String)],
    vod_title: &str,
) -> Result<(Vec<JudgedMoment>, u64, u64), AppError> {
    if candidates.is_empty() {
        return Ok((Vec::new(), 0, 0));
    }
    let prompt = build_final_pass_prompt(candidates, vod_title);
    log::info!(
        "clip_judge: final-pass over {} candidates via {} ({})",
        candidates.len(),
        match provider {
            Provider::Claude => "Claude",
            Provider::OpenAI => "OpenAI",
            Provider::Gemini => "Gemini",
            Provider::Free => "Free",
        },
        model
    );
    let (response, tin, tout) = call_llm(provider, api_key, model, &prompt).await?;
    let moments = parse_final_pass(&response, candidates);
    log::info!(
        "clip_judge: final-pass selected {} of {} moments",
        moments.len(),
        candidates.len()
    );
    Ok((moments, tin, tout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::vod::TranscriptSegment;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.to_string(),
            words: Vec::new(),
        }
    }

    fn transcript(segs: Vec<TranscriptSegment>) -> TranscriptResult {
        let full = segs
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        TranscriptResult {
            segments: segs,
            full_text: full,
            language: "en".to_string(),
            keywords_found: Vec::new(),
            recognition: None,
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
        let p = build_judge_prompt("[10] hi", "My Stream", 3600.0, &[], &[]);
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
    fn prompt_tells_judge_to_find_new_events_after_creator_rejections() {
        let p = build_judge_prompt(
            "[1197] old event\n[1400] different event",
            "My Stream",
            3600.0,
            &[(1197.0, 1257.0), (3182.12, 3204.0)],
            &[],
        );
        assert!(p.contains("PREVIOUSLY REJECTED BY THIS CREATOR"));
        assert!(p.contains("1197-1257s"));
        assert!(p.contains("3182-3204s"));
        assert!(p.contains("shifted, shortened, or widened"));
        assert!(p.contains("genuinely different moments"));
    }

    #[test]
    fn reserved_review_ranges_are_removed_before_judging() {
        let t = transcript(vec![
            seg(10.0, 12.0, "rejected old moment"),
            seg(20.0, 22.0, "approved old moment"),
            seg(30.0, 32.0, "new candidate moment"),
        ]);
        let out = build_transcript_text_excluding(&t, &[(9.0, 13.0), (19.0, 23.0)]);
        assert!(!out.contains("rejected old moment"));
        assert!(!out.contains("approved old moment"));
        assert!(out.contains("[30] new candidate moment"));
    }

    #[test]
    fn prompt_uses_good_moments_as_taste_examples_without_reselecting_them() {
        let examples = vec![ApprovedMomentExample {
            start_sec: 941.0,
            end_sec: 1002.0,
            transcript: "the joke lands and everyone loses it".to_string(),
        }];
        let p = build_judge_prompt(
            "[1200] another possible moment",
            "My Stream",
            3600.0,
            &[],
            &examples,
        );
        assert!(p.contains("CREATOR-APPROVED TASTE EXAMPLES"));
        assert!(p.contains("941-1002s"));
        assert!(p.contains("the joke lands"));
        assert!(p.contains("Do NOT return the same events"));
        assert!(p.contains("ADDITIONAL moments"));
    }

    #[test]
    fn rejected_tail_overlap_is_meaningful_even_when_centers_are_far_apart() {
        assert!(meaningfully_overlaps_excluded_window(
            1243.12, 1298.75, 1197.0, 1257.0,
        ));
        assert!(!meaningfully_overlaps_excluded_window(
            1258.0, 1298.75, 1197.0, 1257.0,
        ));
    }

    #[test]
    fn parses_valid_moments() {
        let json = r#"{"moments":[
            {"start":37.0,"end":62.0,"category":"banter","score":86,"reason":"savage roast"},
            {"start":120.0,"end":150.0,"category":"play","score":74,"reason":"clutch save"}
        ]}"#;
        let m = parse_judge_response(json, 5938.0).unwrap();
        assert_eq!(m.len(), 2);
        assert!((m[0].start_sec - 37.0).abs() < 1e-6);
        assert!((m[0].score - 0.86).abs() < 1e-6);
        assert_eq!(m[0].category, "banter");
        assert_eq!(m[0].reason, "savage roast");
    }

    #[test]
    fn parse_strips_markdown_and_prose() {
        let text = "Here you go:\n```json\n{\"moments\":[{\"start\":5,\"end\":25,\"score\":70,\"category\":\"hype\",\"reason\":\"x\"}]}\n```\nThanks!";
        let m = parse_judge_response(text, 600.0).unwrap();
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
        let m = parse_judge_response(json, 600.0).unwrap();
        assert_eq!(m.len(), 1, "only the clamped-end moment survives");
        assert!(
            (m[0].end_sec - 600.0).abs() < 1e-6,
            "end clamped to duration"
        );
    }

    #[test]
    fn parse_empty_and_garbage_are_safe() {
        assert!(parse_judge_response(r#"{"moments":[]}"#, 600.0)
            .unwrap()
            .is_empty());
        assert!(parse_judge_response("no json here", 600.0).is_err());
        assert!(parse_judge_response("{bad json", 600.0).is_err());
    }

    #[test]
    fn parse_salvages_valid_siblings_from_malformed_array() {
        let text = r#"{"moments":[
            {"start":37,"end":62,"category":"banter","score":86,"reason":"real joke"},
            {"start":120,"end":150,"category":"play","score" 74,"reason":"missing colon"},
            {"start":210,"end":235,"category":"scare","score":91,"reason":"real scare"}
        ]}"#;
        let moments = parse_judge_response(text, 600.0).unwrap();
        assert_eq!(moments.len(), 2);
        assert_eq!(moments[0].category, "banter");
        assert_eq!(moments[1].category, "scare");
    }

    #[test]
    fn parse_reports_fully_malformed_paid_response() {
        let text = r#"{"moments":[{"start":37,"end" 62,"score" 86}]}"#;
        assert!(parse_judge_response(text, 600.0).is_err());
    }

    // ── Transcript cleanup ──

    #[test]
    fn clean_collapses_whisper_repetition_loop() {
        // "wait, wait, wait" ×6 (whisper stall) collapses to ONE segment that
        // spans the whole run.
        let mut segs = vec![seg(10.0, 11.0, "okay here we go")];
        for k in 0..6 {
            let t = 12.0 + k as f64;
            segs.push(seg(t, t + 1.0, "wait, wait"));
        }
        segs.push(seg(30.0, 31.0, "that was wild"));
        let out = clean_segments(&segs);
        let waits = out.iter().filter(|s| s.text.starts_with("wait")).count();
        assert_eq!(waits, 1, "loop collapses to a single segment");
        // The kept loop segment spans the run (12 → 18).
        let loop_seg = out.iter().find(|s| s.text.starts_with("wait")).unwrap();
        assert!((loop_seg.start - 12.0).abs() < 1e-6);
        assert!(loop_seg.end >= 18.0, "collapsed segment spans the run end");
        assert_eq!(out.len(), 3, "intro + collapsed loop + outro");
    }

    #[test]
    fn clean_drops_artifacts_and_adjacent_dupes() {
        let segs = vec![
            seg(0.0, 1.0, "[_TT_]"),
            seg(1.0, 2.0, "[BLANK_AUDIO]"),
            seg(2.0, 3.0, "(music)"),
            seg(3.0, 4.0, "   "),
            seg(4.0, 5.0, "real line"),
            seg(5.0, 6.0, "real line"), // adjacent exact dup
            seg(6.0, 7.0, "different line"),
        ];
        let out = clean_segments(&segs);
        assert_eq!(out.len(), 2, "artifacts dropped, adjacent dup collapsed");
        assert_eq!(out[0].text, "real line");
        assert_eq!(out[1].text, "different line");
    }

    #[test]
    fn clean_keeps_distinct_lines() {
        // Distinct consecutive lines are never collapsed (no loop, no dup).
        let segs = vec![
            seg(0.0, 1.0, "nice shot"),
            seg(1.0, 2.0, "let's go"),
            seg(2.0, 3.0, "clutch"),
        ];
        let out = clean_segments(&segs);
        assert_eq!(out.len(), 3, "three distinct lines all survive");
    }

    // ── Sonnet final-pass parsing ──

    fn jm(start: f64, end: f64, score: f64) -> JudgedMoment {
        JudgedMoment {
            start_sec: start,
            end_sec: end,
            category: "banter".into(),
            score,
            reason: "r".into(),
        }
    }

    #[test]
    fn final_pass_reorders_and_rescores_by_index() {
        let cands = vec![
            (jm(10.0, 30.0, 0.60), "a".to_string()),
            (jm(50.0, 70.0, 0.55), "b".to_string()),
            (jm(90.0, 110.0, 0.50), "c".to_string()),
        ];
        // Model picks index 2 then 0, drops 1, re-scores.
        let resp = r#"{"final":[{"index":2,"score":95},{"index":0,"score":80}]}"#;
        let out = parse_final_pass(resp, &cands);
        assert_eq!(out.len(), 2, "only the two chosen survive");
        assert!((out[0].start_sec - 90.0).abs() < 1e-6, "index 2 first");
        assert!((out[0].score - 0.95).abs() < 1e-6, "re-scored");
        assert!((out[1].start_sec - 10.0).abs() < 1e-6, "index 0 second");
    }

    #[test]
    fn final_pass_drops_bad_and_duplicate_indices() {
        let cands = vec![(jm(10.0, 30.0, 0.6), "a".to_string())];
        // Out-of-range index 9 dropped; duplicate index 0 deduped.
        let resp = r#"{"final":[{"index":9},{"index":0},{"index":0}]}"#;
        let out = parse_final_pass(resp, &cands);
        assert_eq!(out.len(), 1);
        assert!((out[0].start_sec - 10.0).abs() < 1e-6);
    }

    #[test]
    fn final_pass_parse_garbage_is_safe() {
        let cands = vec![(jm(10.0, 30.0, 0.6), "a".to_string())];
        assert!(parse_final_pass("no json", &cands).is_empty());
        assert!(parse_final_pass(r#"{"final":[]}"#, &cands).is_empty());
    }

    #[test]
    fn final_pass_prompt_lists_indexed_candidates() {
        let cands = vec![
            (jm(10.0, 30.0, 0.6), "savage roast here".to_string()),
            (jm(50.0, 70.0, 0.5), "clutch escape".to_string()),
        ];
        let p = build_final_pass_prompt(&cands, "My Stream");
        assert!(p.contains("My Stream"));
        assert!(p.contains("0. [10-30s]"));
        assert!(p.contains("1. [50-70s]"));
        assert!(p.contains("savage roast here"));
        assert!(p.contains("\"final\""));
    }
}
