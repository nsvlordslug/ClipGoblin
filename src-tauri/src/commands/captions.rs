//! Caption generation, AI title, and clip naming commands.

use tauri::State;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use crate::db;
use crate::ai_provider;
use crate::post_captions;
use crate::DbConn;

/// In-memory regenerate history keyed by clip_id. Cleared on app restart.
/// Each clip keeps the last ~10 titles produced so the anti-repeat rule in
/// `generate_llm_titles` sees the full regen chain, not just the current title.
/// Without this, regenerates spaced 3-5 clicks apart can produce duplicates
/// because only the immediately-prior title is in the DB.
static REGEN_TITLE_HISTORY: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();

fn title_history() -> &'static Mutex<HashMap<String, Vec<String>>> {
    REGEN_TITLE_HISTORY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn push_title_history(clip_id: &str, title: &str) {
    const MAX_HISTORY_PER_CLIP: usize = 10;
    if let Ok(mut map) = title_history().lock() {
        let entry = map.entry(clip_id.to_string()).or_default();
        // Skip duplicates at the head (same as last pushed).
        if entry.last().map(String::as_str) != Some(title) {
            entry.push(title.to_string());
        }
        if entry.len() > MAX_HISTORY_PER_CLIP {
            let drop_n = entry.len() - MAX_HISTORY_PER_CLIP;
            entry.drain(..drop_n);
        }
    }
}

fn read_title_history(clip_id: &str) -> Vec<String> {
    title_history()
        .lock()
        .map(|map| map.get(clip_id).cloned().unwrap_or_default())
        .unwrap_or_default()
}

// ── Clip title generation ──
// Mirrors the TypeScript module at src/lib/clipNaming.ts.
// Generates context-aware titles from analysis signals.

/// Event vocabulary — maps tag substrings to readable action labels.
/// These describe WHAT HAPPENED.
pub(crate) const EVENTS: &[(&str, &str)] = &[
    ("kill", "Kill"), ("death", "Death"), ("clutch", "Clutch Play"), ("save", "Save"),
    ("escape", "Escape"), ("chase", "Chase"), ("fight", "Fight"), ("ambush", "Ambush"),
    ("snipe", "Snipe"), ("headshot", "Headshot"), ("combo", "Combo"), ("dodge", "Dodge"),
    ("block", "Block"), ("counter", "Counter"), ("gank", "Gank"), ("wipe", "Team Wipe"),
    ("ace", "Ace"), ("steal", "Steal"), ("grab", "Grab"), ("explosion", "Explosion"),
    ("jumpscare", "Jumpscare"), ("scare", "Scare"),
    ("generator", "Generator"), ("repair", "Repair"),
    ("hook", "Hook"), ("interrupt", "Interrupt"), ("down", "Down"),
    ("rescue", "Rescue"), ("loop", "Loop"), ("mindgame", "Mind Game"),
    ("juke", "Juke"), ("bait", "Bait"), ("outplay", "Outplay"),
    ("miss", "Missed Hit"), ("whiff", "Whiff"),
    ("encounter", "Encounter"), ("skirmish", "Skirmish"),
    ("scream", "Scream"),
];

pub(crate) fn parse_tags(tags: Option<&str>) -> Vec<String> {
    tags.map(|t| t.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

pub(crate) fn classify(tags: &[String], vocab: &[(&str, &str)]) -> Vec<String> {
    let mut found = Vec::new();
    for tag in tags {
        for &(key, label) in vocab {
            if tag.contains(key) && !found.contains(&label.to_string()) {
                found.push(label.to_string());
                break;
            }
        }
    }
    found
}

// ═══════════════════════════════════════════════════════════════════
//  Grounded title / confidence / explanation for the save path.
//  These replace the hype-title generator for newly saved highlights.
// ═══════════════════════════════════════════════════════════════════

/// 2-stage title: ground truth → hook formatting.
///
/// Same logic as clip_labeler but adapted for the save path which
/// receives raw tag strings instead of structured CandidateClip data.
pub(crate) fn grounded_highlight_title(
    transcript_snippet: Option<&str>,
    tags: Option<&str>,
    start_seconds: f64,
) -> String {
    let phrase = extract_title_phrase(transcript_snippet.unwrap_or(""));
    let event = primary_event_from_tags(tags);
    let idx = start_seconds as usize;

    // 1. Reaction + Context (transcript + event)
    if let (Some(ref p), Some(ev)) = (&phrase, event) {
        if !p.to_lowercase().contains(&ev.to_lowercase()) {
            let ctx = save_context_tag(ev);
            let words: Vec<&str> = p.split_whitespace().take(5).collect();
            let q = words.join(" ");
            let formatted = if p.split_whitespace().count() > 5 {
                format!("{}...", q)
            } else {
                save_punctuate(&q)
            };
            return format!("\"{}\" {}", formatted, ctx);
        }
    }

    // 2. Transcript alone (if specific enough)
    if let Some(ref p) = phrase {
        if !is_vague_phrase(p) {
            let words: Vec<&str> = p.split_whitespace().collect();
            if words.len() >= 7 {
                let short: Vec<&str> = words[..6].to_vec();
                return format!("\"{}...\"", short.join(" "));
            }
            return format!("\"{}\"", p);
        }
    }

    // 3. Outcome-based (compound tags)
    if let Some(tag_str) = tags {
        let tag_list = parse_tags(Some(tag_str));
        if let Some(title) = save_outcome_title(&tag_list) {
            return title;
        }
    }

    // 4. Event + Tension (verb-forward, timing-aware)
    if let Some(ev) = event {
        let phrases: &[&str] = match ev {
            "jumpscare" | "ambush" => &["Ambush comes out of nowhere", "Caught off guard instantly", "Jumpscare hits with no warning"],
            "fight" => &["Fight breaks out instantly", "Fight goes wrong fast", "Fight starts and it gets bad"],
            "explosion" => &["Explosion hits out of nowhere", "Blows up with no warning"],
            "panic" => &["Panic hits instantly", "Everything goes wrong at once", "Panic sets in right away"],
            "celebration" => &["Clutches it at the last second", "Barely survives then celebrates"],
            "frustration" => &["Nothing goes right", "Loses it after that play"],
            "shock" | "disbelief" => &["Didn't see that coming", "Shock hits out of nowhere"],
            "hype" => &["Hype hits out of nowhere", "Goes off at the perfect time"],
            "reaction" => &["Reaction says it all", "Reacts instantly"],
            _ => &["Happens out of nowhere"],
        };
        return phrases[idx % phrases.len()].to_string();
    }

    // 5. Tag summary fallback
    if let Some(tag_str) = tags {
        let tag_list = parse_tags(Some(tag_str));
        let events = classify(&tag_list, EVENTS);
        if !events.is_empty() {
            return events[..events.len().min(2)].join(" + ");
        }
    }

    // 6. Timestamp
    let mins = (start_seconds as u32) / 60;
    let secs = (start_seconds as u32) % 60;
    format!("Highlight at {}:{:02}", mins, secs)
}

fn save_punctuate(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with('.') || t.ends_with('!') || t.ends_with('?') { t.to_string() }
    else { format!("{}.", t) }
}

fn save_context_tag(event: &str) -> &'static str {
    match event {
        "jumpscare" | "ambush" => "caught off guard",
        "fight" => "mid-fight",
        "explosion" => "right before it blows up",
        "celebration" => "clutches it",
        "panic" => "instant panic",
        "frustration" => "loses it",
        "disbelief" => "didn't see that coming",
        "shock" => "instant reaction",
        "hype" => "peak hype",
        "reaction" => "the reaction",
        _ => "out of nowhere",
    }
}

fn save_outcome_title(tag_list: &[String]) -> Option<String> {
    let has = |t: &str| tag_list.iter().any(|x| x.contains(t));
    if has("fight") && has("celebration") { return Some("Fight breaks out and they clutch it".into()); }
    if has("fight") && has("frustration") { return Some("Fight goes wrong and they lose it".into()); }
    if has("fight") && has("panic") { return Some("Fight turns bad fast".into()); }
    if (has("ambush") || has("jumpscare")) && has("panic") { return Some("Ambush hits and panic sets in".into()); }
    if (has("ambush") || has("jumpscare")) && has("shock") { return Some("Ambush out of nowhere".into()); }
    if has("panic") && has("celebration") { return Some("Almost dies then clutches it".into()); }
    if has("hype") && has("celebration") { return Some("Clutch play at the last second".into()); }
    None
}

fn extract_title_phrase(excerpt: &str) -> Option<String> {
    let trimmed = excerpt.trim();
    if trimmed.len() < 3 { return None; }
    let filler = ["like", "so", "um", "uh", "okay", "ok", "well", "and", "but"];
    let words: Vec<&str> = trimmed.split_whitespace()
        .skip_while(|w| filler.iter().any(|f| w.to_lowercase() == *f))
        .take(8)
        .collect();
    if words.len() < 2 { return None; }
    Some(words.join(" "))
}

fn is_vague_phrase(s: &str) -> bool {
    let wc = s.split_whitespace().count();
    if wc < 4 { return true; }
    let lower = s.to_lowercase();
    let vague = ["oh my god", "oh my gosh", "what the hell", "what the fuck",
                  "no way dude", "are you serious", "holy shit"];
    wc <= 4 && vague.iter().any(|v| lower.contains(v))
}

fn primary_event_from_tags(tags: Option<&str>) -> Option<&'static str> {
    let tag_str = tags?;
    let tag_list = parse_tags(Some(tag_str));
    let lower: Vec<String> = tag_list.iter().map(|t| t.to_lowercase()).collect();
    if lower.iter().any(|t| t.contains("jumpscare") || t.contains("ambush")) { return Some("jumpscare"); }
    if lower.iter().any(|t| t.contains("fight"))     { return Some("fight"); }
    if lower.iter().any(|t| t.contains("explosion")) { return Some("explosion"); }
    if lower.iter().any(|t| t.contains("celebration")){ return Some("celebration"); }
    if lower.iter().any(|t| t.contains("panic"))     { return Some("panic"); }
    if lower.iter().any(|t| t.contains("frustration")){ return Some("frustration"); }
    if lower.iter().any(|t| t.contains("disbelief")) { return Some("disbelief"); }
    if lower.iter().any(|t| t.contains("shock"))     { return Some("shock"); }
    if lower.iter().any(|t| t.contains("hype"))      { return Some("hype"); }
    if lower.iter().any(|t| t.contains("reaction"))  { return Some("reaction"); }
    if lower.iter().any(|t| t.contains("rapid"))     { return Some("rapid cuts"); }
    None
}

// ═══════════════════════════════════════════════════════════════════
//  Save-path title generation (analyze-time, runs synchronously)
// ═══════════════════════════════════════════════════════════════════
//
//  Called from `vod.rs` inside `run_analysis_signals` for each candidate
//  clip. This is the title users see immediately after analysis, BEFORE
//  any LLM upgrade pass runs. Two layers:
//
//  1. Wave 3-shaped templates (QuietFlex from a punchy transcript phrase,
//     AftermathConfession from event tags + game name)
//  2. Fall back to `grounded_highlight_title` for cases neither matches
//
//  After all clips are produced, `upgrade_titles_with_llm` runs in async
//  context and replaces these heuristic titles with LLM-generated ones
//  when the user has BYOK + the titles toggle on.

/// Save-path heuristic title. Tries Wave 3-shaped templates first, then
/// falls back to the legacy grounded heuristic for unmatched cases.
pub fn save_path_heuristic_title(
    transcript_excerpt: Option<&str>,
    tags_str: Option<&str>,
    game_name: Option<&str>,
    start_seconds: f64,
) -> String {
    let tags = parse_tags(tags_str);

    // Layer 1: QuietFlex via a short transcript phrase. If the transcript has
    // a 2-5 word non-vague fragment, return it standalone. Maps to real top
    // performers like "actually clean" / "rip mouse" — short, voice, post-clip.
    if let Some(excerpt) = transcript_excerpt {
        if let Some(phrase) = extract_title_phrase(excerpt) {
            let wc = phrase.split_whitespace().count();
            if wc >= 2 && wc <= 5 && !is_vague_phrase(&phrase) {
                return phrase;
            }
        }
    }

    // Layer 2: AftermathConfession from event tags + game name. First-person
    // past-tense templated lines, anchored on the game when available.
    if let Some(line) = aftermath_from_tags(&tags, game_name) {
        return line;
    }

    // Layer 3: Fall back to the legacy grounded heuristic. Still used when
    // neither of the above produces something Wave 3-shaped.
    grounded_highlight_title(transcript_excerpt, tags_str, start_seconds)
}

/// Pick a Wave 3 AftermathConfession-style template based on event tags.
/// Anchors on the game name when supplied so titles read "Elden Ring broke me"
/// rather than the abstract "broke me" version.
fn aftermath_from_tags(tags: &[String], game_name: Option<&str>) -> Option<String> {
    let has = |needle: &str| tags.iter().any(|t| t.contains(needle));
    let game = game_name
        .map(str::trim)
        .filter(|g| !g.is_empty());

    if has("ambush") || has("jumpscare") {
        return Some(match game {
            Some(g) => format!("{} ambushed me before i could move", g.to_lowercase()),
            None => "ambushed before i could move".into(),
        });
    }
    if has("fight") && has("panic") {
        return Some(match game {
            Some(g) => format!("panicked mid-fight in {}", g.to_lowercase()),
            None => "panicked mid-fight".into(),
        });
    }
    if has("fight") && has("frustration") {
        return Some("couldn't survive that fight".into());
    }
    if has("celebration") && has("hype") {
        return Some("clutched it at the last second".into());
    }
    if has("death") {
        return Some(match game {
            Some(g) => format!("{} broke me before i blinked", g.to_lowercase()),
            None => "broke me before i blinked".into(),
        });
    }
    if has("explosion") {
        return Some("blew up before i could react".into());
    }
    if has("disbelief") || has("shock") {
        return Some("didn't see that one coming".into());
    }
    None
}

/// Async LLM upgrade pass. Iterates highlights and replaces each title with
/// a Wave 3 LLM-generated one when BYOK + titles toggle is on. Per-clip
/// failures keep the existing heuristic title (not fatal).
///
/// Called from `analyze_vod` AFTER `run_analysis_signals` returns highlights
/// but BEFORE inserting them into the DB.
pub async fn upgrade_titles_with_llm(
    highlights: &mut [db::HighlightRow],
    resolved: &ai_provider::ResolvedProvider,
    vod_game: Option<&str>,
) {
    if !resolved.is_llm() {
        log::debug!(
            "Save-path Wave 3 upgrade skipped — provider resolved to Free for Scope::Titles"
        );
        return;
    }

    log::info!(
        "Save-path Wave 3: upgrading {} title(s) with {:?} (model: {})",
        highlights.len(),
        resolved.provider,
        resolved.model,
    );

    for h in highlights.iter_mut() {
        let tags: Vec<String> = h
            .tags
            .as_deref()
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let event_summary = h.event_summary.clone().unwrap_or_default();

        // Use the highlight's stored transcript snippet — it's the relevant
        // excerpt for this clip range, set during signal fusion.
        let transcript_for_clip = h.transcript_snippet.as_deref();

        // No money-quote extraction at analyze time — keeps the cost down to
        // one API call per clip. Money quote still extracts on regenerate.
        match post_captions::generate_llm_titles(
            &resolved.api_key,
            &resolved.model,
            &event_summary,
            None,                  // money_quote — skip at analyze
            transcript_for_clip,
            &tags,
            vod_game,
            None,                  // streamer_history — fresh analyze, no prior titles
            None,                  // target_platform — defaults to TikTok
        )
        .await
        {
            Ok(candidates) => {
                if let Some(top) = candidates.first() {
                    log::info!(
                        "Save-path Wave 3 title for highlight {}: \"{}\" (pattern {:?}, score {:.2})",
                        h.id, top.text, top.pattern, top.score,
                    );
                    h.description = Some(top.text.clone());
                } else {
                    log::warn!(
                        "Save-path Wave 3: zero candidates for highlight {} — keeping heuristic",
                        h.id
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "Save-path Wave 3 failed for highlight {}: {} — keeping heuristic",
                    h.id, e
                );
            }
        }
    }
}

/// Compute confidence from raw score and signal count,
/// then applies a piecewise curve matching the pipeline calibration.
///
/// Target distribution:
///   most clips: 55–80%   strong: 80–90%   exceptional: 90–95%
pub(crate) fn compute_confidence(raw_score: f64, signal_count: usize) -> f64 {
    // Step 1: de-inflate — strip bonus stacking headroom
    let normalized = (raw_score * 0.85 - 0.10).clamp(0.0, 0.99);

    // Step 2: piecewise curve (same shape as clip_ranker::rescale_confidence)
    const ANCHORS: [(f64, f64); 8] = [
        (0.00, 0.00),
        (0.25, 0.25),
        (0.40, 0.55),
        (0.50, 0.65),
        (0.60, 0.77),
        (0.70, 0.84),
        (0.80, 0.89),
        (0.90, 0.93),
    ];

    let base = if normalized >= 0.90 {
        (0.93 + (normalized - 0.90) * 0.20).min(0.95)
    } else {
        let mut out = 0.0;
        for i in 1..ANCHORS.len() {
            if normalized <= ANCHORS[i].0 {
                let (x0, y0) = ANCHORS[i - 1];
                let (x1, y1) = ANCHORS[i];
                let t = (normalized - x0) / (x1 - x0);
                out = y0 + t * (y1 - y0);
                break;
            }
        }
        out
    };

    // Minimal signal nudge
    let nudge = if signal_count >= 4 { 0.01 } else { 0.0 };
    (base + nudge).min(0.96)
}

/// Count how many of the score channels are meaningfully active.
pub(crate) fn count_active_signals(audio: f64, visual: f64, chat: f64, has_transcript: bool) -> usize {
    let mut n = 0;
    if audio > 0.1 { n += 1; }
    if visual > 0.1 { n += 1; }
    if chat > 0.1 { n += 1; }
    if has_transcript { n += 1; }
    n
}

/// Build a factual explanation: signal values + count.
pub(crate) fn build_highlight_explanation(audio: f64, visual: f64, chat: f64, has_transcript: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if audio > 0.0 { parts.push(format!("audio {:.0}%", audio * 100.0)); }
    if visual > 0.0 { parts.push(format!("visual {:.0}%", visual * 100.0)); }
    if chat > 0.0 { parts.push(format!("chat {:.0}%", chat * 100.0)); }
    if has_transcript { parts.push("transcript match".into()); }

    let count = parts.len();
    if parts.is_empty() {
        "No signal data".into()
    } else {
        format!("{} signal{} — {}", count, if count != 1 { "s" } else { "" }, parts.join(", "))
    }
}

/// Generate TikTok-style post captions on demand from a clip's highlight data.
///
/// If a Claude API key is configured, uses the LLM for fresh generation.
/// Otherwise falls back to the pattern-based system.
#[tauri::command]
pub async fn generate_post_captions(
    clip_id: String,
    seed: Option<u32>,
    transcript_text: Option<String>,
    current_title: Option<String>,
    current_game: Option<String>,
    selected_mode: Option<String>,
    db: State<'_, DbConn>,
) -> Result<post_captions::PostCaptions, String> {
    let (clip, tags, transcript, highlight_scores, resolved) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.iter().find(|h| h.id == clip.highlight_id);

        let tags: Vec<String> = highlight
            .and_then(|h| h.tags.as_ref())
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        // Prefer full subtitle transcript from frontend; fall back to highlight snippet
        let transcript = transcript_text
            .filter(|t| !t.trim().is_empty())
            .or_else(|| highlight.and_then(|h| h.transcript_snippet.clone()));
        let scores = (
            highlight.map(|h| h.audio_score).unwrap_or(0.0),
            highlight.map(|h| h.visual_score).unwrap_or(0.0),
            highlight.map(|h| h.chat_score).unwrap_or(0.0),
        );

        // Resolve provider for captions scope
        let resolved = ai_provider::resolve(&conn, ai_provider::Scope::Captions);

        (clip, tags, transcript, scores, resolved)
    };

    // Use frontend title if provided, otherwise fall back to clip title
    let title = current_title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| clip.title.clone());

    let (audio, visual, chat) = highlight_scores;

    // Default to direct_quote if no mode specified
    let mode = selected_mode.unwrap_or_else(|| "direct_quote".into());

    // ── Try LLM generation if provider is configured ──
    if resolved.is_llm() {
        let tone = post_captions::classify_tone_pub(
            &tags, transcript.as_deref(), audio, visual, chat,
        );
        let event = post_captions::primary_event_pub(&tags);
        let event_summary = post_captions::synthesize_event_pub(
            event, tone, &tags, seed.unwrap_or(0) as usize,
        );
        let tone_label = tone.label();
        let quote = post_captions::strong_quote_pub(transcript.as_deref());

        // Prefer live game value from frontend; fall back to DB value
        let game_name = current_game.as_deref()
            .filter(|s| !s.is_empty())
            .or(clip.game.as_deref());

        log::info!("Caption generation: using {:?} (model: {})", resolved.provider, resolved.model);
        log::info!("Caption generation: mode = {}, game = {:?}", mode, game_name);

        // Wave 3: extract a money-quote first (tiny, separate API call). Non-fatal
        // if it fails — captions still generate without one, just less punchy.
        let money_quote: Option<String> = match transcript.as_deref().filter(|t| !t.trim().is_empty()) {
            Some(ft) => match post_captions::extract_money_quote_llm(
                &resolved.api_key, &resolved.model, &event_summary, ft, &tags,
            ).await {
                Ok(q) => q,
                Err(e) => { log::debug!("Money-quote extraction skipped: {}", e); None }
            },
            None => None,
        };

        match post_captions::generate_llm_caption(
            &resolved.api_key, &resolved.model, &mode,
            None, // platform — defaults to TikTok; a future frontend selector can override
            &event_summary, money_quote.as_deref(), quote.as_deref(), tone_label,
            &tags, transcript.as_deref(), &title, game_name,
            &[], // streamer_niche_tags — surface from settings in future work
            clip.publish_description.as_deref(), // avoid_caption — breaks regen determinism
        ).await {
            Ok(candidates) if !candidates.is_empty() => {
                let top = &candidates[0];
                log::info!(
                    "Wave 3: {} caption candidate(s) for clip {} (mode: {}, top score {:.2}, hook \"{}\")",
                    candidates.len(), clip_id, mode, top.score, top.hook_line,
                );
                // Top-scored only. Candidates are pre-sorted descending by score.
                let llm_captions = vec![post_captions::caption_candidate_to_variant(top, &mode)];
                let hashtags = post_captions::build_hashtags_pub(&tags, tone);
                let casual = llm_captions.first().map(|c| c.text.clone()).unwrap_or_default();
                let funny  = llm_captions.get(1).map(|c| c.text.clone()).unwrap_or_default();
                let hype   = llm_captions.get(2).map(|c| c.text.clone()).unwrap_or_default();
                return Ok(post_captions::PostCaptions {
                    captions: llm_captions,
                    hashtags,
                    source: "llm".into(),
                    casual, funny, hype,
                });
            }
            Ok(_) => {
                log::warn!("LLM returned zero caption candidates");
                if !resolved.fallback_to_free {
                    return Err("Caption generation returned no candidates".into());
                }
                log::info!("Falling back to Free mode (pattern-based)");
            }
            Err(e) => {
                log::warn!("LLM caption generation failed: {}", e);
                if !resolved.fallback_to_free {
                    return Err(format!("Caption generation failed: {}", e));
                }
                log::info!("Falling back to Free mode (pattern-based)");
            }
        }
    }

    // ── Fallback: pattern-based generation ──
    Ok(post_captions::generate_from_parts(
        &tags,
        transcript.as_deref(),
        &title,
        clip.start_seconds,
        audio, visual, chat,
        seed.unwrap_or(0),
    ))
}

/// Generate an AI-powered clip title.
///
/// Uses the configured BYOK provider (Titles scope) to generate a short,
/// punchy title for the clip.  Returns the local heuristic title as fallback.
#[tauri::command]
pub async fn generate_ai_title(
    clip_id: String,
    transcript_text: Option<String>,
    current_game: Option<String>,
    current_title: Option<String>,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let (clip, tags, transcript, highlight_scores, resolved) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.iter().find(|h| h.id == clip.highlight_id);

        let tags: Vec<String> = highlight
            .and_then(|h| h.tags.as_ref())
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        let transcript = transcript_text
            .filter(|t| !t.trim().is_empty())
            .or_else(|| highlight.and_then(|h| h.transcript_snippet.clone()));
        let scores = (
            highlight.map(|h| h.audio_score).unwrap_or(0.0),
            highlight.map(|h| h.visual_score).unwrap_or(0.0),
            highlight.map(|h| h.chat_score).unwrap_or(0.0),
        );

        let resolved = ai_provider::resolve(&conn, ai_provider::Scope::Titles);

        (clip, tags, transcript, scores, resolved)
    };

    let (audio, visual, chat) = highlight_scores;

    if resolved.is_llm() {
        let tone = post_captions::classify_tone_pub(
            &tags, transcript.as_deref(), audio, visual, chat,
        );
        let event = post_captions::primary_event_pub(&tags);
        let event_summary = post_captions::synthesize_event_pub(
            event, tone, &tags, 0,
        );

        let game_name = current_game.as_deref()
            .filter(|s| !s.is_empty())
            .or(clip.game.as_deref());

        log::info!("AI title generation: using {:?} (model: {})", resolved.provider, resolved.model);

        // Wave 3: extract a money-quote first so the titles prompt can inherit it
        // (enables QuoteTwist pattern). Non-fatal on failure.
        let money_quote: Option<String> = match transcript.as_deref().filter(|t| !t.trim().is_empty()) {
            Some(ft) => match post_captions::extract_money_quote_llm(
                &resolved.api_key, &resolved.model, &event_summary, ft, &tags,
            ).await {
                Ok(q) => q,
                Err(e) => { log::debug!("Money-quote extraction skipped: {}", e); None }
            },
            None => None,
        };

        // Regenerate anti-repeat: build a history of all titles the model has
        // already produced for this clip in this session (REGEN_TITLE_HISTORY),
        // plus the title the UI is currently showing (may be stale relative to
        // DB, which is why frontend passes it explicitly). The ">50% token
        // overlap" prompt rule and the ranker's history check both consume this
        // list. Without session history, regens spaced N clicks apart can
        // duplicate each other.
        let effective_current = current_title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(clip.title.as_str())
            .to_string();
        let mut title_avoid: Vec<String> = read_title_history(&clip_id);
        if !effective_current.trim().is_empty()
            && !title_avoid.iter().any(|t| t == &effective_current)
        {
            title_avoid.push(effective_current);
        }
        let history_slice: Option<&[String]> = if title_avoid.is_empty() {
            None
        } else {
            Some(title_avoid.as_slice())
        };

        match post_captions::generate_llm_titles(
            &resolved.api_key, &resolved.model, &event_summary,
            money_quote.as_deref(), transcript.as_deref(),
            &tags, game_name,
            history_slice,
            None, // target_platform — defaults to TikTok
        ).await {
            Ok(candidates) => {
                if let Some(top) = candidates.first() {
                    log::info!(
                        "Wave 3 title for clip {}: \"{}\" (pattern {:?}, score {:.2}, {} candidates)",
                        clip_id, top.text, top.pattern, top.score, candidates.len(),
                    );
                    // Record this regen so subsequent calls on the same clip see
                    // the full session history, not just the current UI title.
                    push_title_history(&clip_id, &top.text);
                    return Ok(top.text.clone());
                }
                log::warn!("LLM returned zero title candidates");
                if !resolved.fallback_to_free {
                    return Err("Title generation returned no candidates".into());
                }
            }
            Err(e) => {
                log::warn!("AI title generation failed: {}", e);
                if !resolved.fallback_to_free {
                    return Err(format!("Title generation failed: {}", e));
                }
            }
        }
    }

    // Fallback: return the existing clip title
    Ok(clip.title)
}

/// Test an AI provider connection with a minimal API call.
/// Returns a status string: "connected", or an error description.
#[tauri::command]
pub async fn test_ai_connection(
    provider: String,
    api_key: String,
    model: String,
) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("No API key provided".into());
    }

    let client = reqwest::Client::new();

    match provider.as_str() {
        "claude" => {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 5,
                "messages": [{"role": "user", "content": "Say ok"}]
            });
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                401 => Err("Invalid API key".into()),
                403 => Err("API key lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        "openai" => {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 5,
                "messages": [{"role": "user", "content": "Say ok"}]
            });
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                401 => Err("Invalid API key".into()),
                403 => Err("API key lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        "gemini" => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                model
            );
            let body = serde_json::json!({
                "contents": [{"parts": [{"text": "Say ok"}]}],
                "generationConfig": {"maxOutputTokens": 5}
            });
            let resp = client
                .post(&url)
                .header("x-goog-api-key", api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                400 => Err("Invalid request — check API key".into()),
                403 => Err("API key invalid or lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        _ => Err(format!("Unknown provider: {}", provider)),
    }
}
