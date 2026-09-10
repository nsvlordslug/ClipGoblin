//! Caption generation, AI title, and clip naming commands.

use crate::ai_provider;
use crate::db;
use crate::moment_brief::{CopySuggestion, MomentBrief, MomentEvidence, MomentScores};
use crate::post_captions;
use crate::DbConn;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::State;

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

#[derive(Debug, Clone, serde::Serialize)]
pub struct MomentCopySuggestion {
    pub text: String,
    pub strategy: String,
    pub feedback_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MomentCaptionSuggestion {
    pub mode: String,
    pub label: String,
    pub text: String,
    pub strategy: String,
    pub feedback_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MomentCopyResponse {
    pub title: MomentCopySuggestion,
    pub captions: Vec<MomentCaptionSuggestion>,
    pub hashtags: Vec<String>,
    pub source: String,
    pub title_source: String,
    pub brief: MomentBrief,
}

fn feedback_suggestion(suggestion: CopySuggestion) -> MomentCopySuggestion {
    MomentCopySuggestion {
        text: suggestion.text,
        strategy: suggestion.strategy,
        feedback_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn caption_mode_label(mode: &str) -> &'static str {
    match mode {
        "direct_quote" => "Quote",
        "blame" => "Blame",
        "internal_thought" => "Thought",
        "observation" => "Observe",
        "punchy" => "Punchy",
        "clean" => "Clean",
        "funny" => "Funny",
        "hype" => "Hype",
        "search" => "SEO",
        "minimal" => "Minimal",
        _ => "Caption",
    }
}

fn parse_signal_sources(value: Option<&str>) -> Vec<String> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
        raw.split(',')
            .map(|source| source.trim().to_lowercase())
            .filter(|source| !source.is_empty())
            .collect()
    })
}

fn scoring_value(value: Option<&str>, key: &str) -> f64 {
    value
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|json| json.get(key).and_then(serde_json::Value::as_f64))
        .unwrap_or(0.0)
}

fn build_moment_brief(
    highlight: Option<&db::HighlightRow>,
    transcript: Option<&str>,
    detector_title: Option<&str>,
    game: Option<&str>,
    stream_style: Option<&str>,
) -> MomentBrief {
    let tags = parse_tags(highlight.and_then(|row| row.tags.as_deref()));
    let signal_sources =
        parse_signal_sources(highlight.and_then(|row| row.signal_sources.as_deref()));
    let dimensions = highlight.and_then(|row| row.scoring_dimensions.as_deref());
    MomentBrief::build(MomentEvidence {
        transcript: transcript
            .or_else(|| highlight.and_then(|row| row.transcript_snippet.as_deref())),
        detector_summary: highlight.and_then(|row| row.event_summary.as_deref()),
        detector_title: detector_title
            .or_else(|| highlight.and_then(|row| row.description.as_deref())),
        payoff_summary: highlight.and_then(|row| row.event_summary.as_deref()),
        outcome_label: None,
        tags: &tags,
        game,
        stream_style,
        signal_sources: &signal_sources,
        scores: MomentScores {
            hook: highlight.map(|row| row.audio_score).unwrap_or(0.0),
            emotion: highlight.map(|row| row.visual_score).unwrap_or(0.0),
            payoff: scoring_value(dimensions, "payoff"),
            alignment: scoring_value(dimensions, "align"),
            context: scoring_value(dimensions, "context"),
            confidence: highlight
                .and_then(|row| row.confidence_score)
                .unwrap_or_default(),
        },
    })
}

fn detection_platform(value: Option<&str>) -> crate::detection::Platform {
    match value.unwrap_or_default().to_lowercase().as_str() {
        "youtube" | "shorts" => crate::detection::Platform::YouTubeShorts,
        "instagram" | "reels" => crate::detection::Platform::InstagramReels,
        "tiktok" => crate::detection::Platform::TikTok,
        _ => crate::detection::Platform::Generic,
    }
}

// ── Clip title generation ──
// Mirrors the TypeScript module at src/lib/clipNaming.ts.
// Generates context-aware titles from analysis signals.

/// Event vocabulary — maps tag substrings to readable action labels.
/// These describe WHAT HAPPENED.
pub(crate) const EVENTS: &[(&str, &str)] = &[
    ("kill", "Kill"),
    ("death", "Death"),
    ("clutch", "Clutch Play"),
    ("save", "Save"),
    ("escape", "Escape"),
    ("chase", "Chase"),
    ("fight", "Fight"),
    ("ambush", "Ambush"),
    ("snipe", "Snipe"),
    ("headshot", "Headshot"),
    ("combo", "Combo"),
    ("dodge", "Dodge"),
    ("block", "Block"),
    ("counter", "Counter"),
    ("gank", "Gank"),
    ("wipe", "Team Wipe"),
    ("ace", "Ace"),
    ("steal", "Steal"),
    ("grab", "Grab"),
    ("explosion", "Explosion"),
    ("jumpscare", "Jumpscare"),
    ("scare", "Scare"),
    ("generator", "Generator"),
    ("repair", "Repair"),
    ("hook", "Hook"),
    ("interrupt", "Interrupt"),
    ("down", "Down"),
    ("rescue", "Rescue"),
    ("loop", "Loop"),
    ("mindgame", "Mind Game"),
    ("juke", "Juke"),
    ("bait", "Bait"),
    ("outplay", "Outplay"),
    ("miss", "Missed Hit"),
    ("whiff", "Whiff"),
    ("encounter", "Encounter"),
    ("skirmish", "Skirmish"),
    ("scream", "Scream"),
];

pub(crate) fn parse_tags(tags: Option<&str>) -> Vec<String> {
    let Some(raw) = tags.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };

    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(raw) {
        return parsed
            .into_iter()
            .map(|tag| tag.trim().to_lowercase())
            .filter(|tag| !tag.is_empty())
            .collect();
    }

    raw.split(',')
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect()
}

#[cfg(test)]
mod tag_parsing_tests {
    use super::parse_tags;

    #[test]
    fn parses_json_and_csv_highlight_tags() {
        assert_eq!(
            parse_tags(Some(r#"["Fight", "Panic"]"#)),
            vec!["fight", "panic"]
        );
        assert_eq!(parse_tags(Some("Fight, Panic")), vec!["fight", "panic"]);
    }
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

/// Per-batch usage counter for save-path title variants.
///
/// Threaded through every per-clip title call within a single VOD analysis
/// run. Each title-picking helper consults this map to prefer variants that
/// haven't been used yet, so 17 clips sharing the same dominant tag don't
/// all land on the same template line. Caller (run_analysis_signals) is
/// responsible for creating one fresh map per VOD and incrementing the
/// count at the outermost title return site (`save_path_heuristic_title`).
pub type TitleUsage = std::collections::HashMap<String, usize>;

/// Pick the least-used variant from `pool`, breaking ties deterministically
/// using `seed` (typically `start_seconds as usize`) so the same clip
/// resolves to the same title within a batch (i.e. analysis is idempotent
/// — re-running the same VOD produces the same titles).
///
/// Pure: does NOT mutate `usage`. The caller (`save_path_heuristic_title`)
/// is responsible for incrementing the count exactly once per clip after
/// the final title is locked in. This single-site increment avoids the
/// double-counting trap where multiple internal helpers might each bump
/// the count for the same clip.
///
/// Empty pools return an empty string — callers must ensure non-empty pools.
fn pick_least_used(pool: &[String], usage: &TitleUsage, seed: usize) -> String {
    if pool.is_empty() {
        return String::new();
    }
    // Find the minimum usage count across all variants in this pool.
    let min_count = pool
        .iter()
        .map(|s| usage.get(s).copied().unwrap_or(0))
        .min()
        .unwrap_or(0);
    // Collect indices of all variants tied at that minimum count.
    let candidates: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, s)| usage.get(*s).copied().unwrap_or(0) == min_count)
        .map(|(i, _)| i)
        .collect();
    // Tiebreak deterministically — same `seed` always picks the same variant.
    let chosen_idx = candidates[seed % candidates.len()];
    pool[chosen_idx].clone()
}

/// 2-stage title: ground truth → hook formatting.
///
/// Same logic as clip_labeler but adapted for the save path which
/// receives raw tag strings instead of structured CandidateClip data.
pub(crate) fn grounded_highlight_title(
    transcript_snippet: Option<&str>,
    tags: Option<&str>,
    start_seconds: f64,
    usage: &TitleUsage,
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

    // 4. Event + Tension (verb-forward, timing-aware).
    // Per-event variant pool, picked least-used so multiple clips with the
    // same event tag don't all land on the same line.
    if let Some(ev) = event {
        let phrases: &[&str] = match ev {
            "jumpscare" | "ambush" => &[
                "Ambush comes out of nowhere",
                "Caught off guard instantly",
                "Jumpscare hits with no warning",
            ],
            "fight" => &[
                "Fight breaks out instantly",
                "Fight goes wrong fast",
                "Fight starts and it gets bad",
            ],
            "explosion" => &["Explosion hits out of nowhere", "Blows up with no warning"],
            "panic" => &[
                "Panic hits instantly",
                "Everything goes wrong at once",
                "Panic sets in right away",
            ],
            "celebration" => &[
                "Clutches it at the last second",
                "Barely survives then celebrates",
            ],
            "frustration" => &["Nothing goes right", "Loses it after that play"],
            "shock" | "disbelief" => &["Didn't see that coming", "Shock hits out of nowhere"],
            "hype" => &["Hype hits out of nowhere", "Goes off at the perfect time"],
            "reaction" => &["Reaction says it all", "Reacts instantly"],
            _ => &["Happens out of nowhere"],
        };
        let pool: Vec<String> = phrases.iter().map(|s| (*s).to_string()).collect();
        return pick_least_used(&pool, usage, idx);
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
    if t.ends_with('.') || t.ends_with('!') || t.ends_with('?') {
        t.to_string()
    } else {
        format!("{}.", t)
    }
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
    if has("fight") && has("celebration") {
        return Some("Fight breaks out and they clutch it".into());
    }
    if has("fight") && has("frustration") {
        return Some("Fight goes wrong and they lose it".into());
    }
    if has("fight") && has("panic") {
        return Some("Fight turns bad fast".into());
    }
    if (has("ambush") || has("jumpscare")) && has("panic") {
        return Some("Ambush hits and panic sets in".into());
    }
    if (has("ambush") || has("jumpscare")) && has("shock") {
        return Some("Ambush out of nowhere".into());
    }
    if has("panic") && has("celebration") {
        return Some("Almost dies then clutches it".into());
    }
    if has("hype") && has("celebration") {
        return Some("Clutch play at the last second".into());
    }
    None
}

fn extract_title_phrase(excerpt: &str) -> Option<String> {
    let trimmed = excerpt.trim();
    if trimmed.len() < 3 {
        return None;
    }
    // Reject signal-stat placeholder strings — chat-only and emote-only
    // candidates inject these as fake "transcripts" (see vod.rs around
    // lines 1900-1955) so the analysis stage has SOMETHING to put in
    // the transcript_snippet field. They're useful for downstream signal
    // attribution but disastrous as title basis: every chat-spike clip
    // ends up titled "N chat messages in this..." which (a) reads like
    // a debug log and (b) collapses to a near-duplicate template across
    // every chat-derived clip.
    if is_signal_placeholder(trimmed) {
        return None;
    }
    let filler = ["like", "so", "um", "uh", "okay", "ok", "well", "and", "but"];
    let words: Vec<&str> = trimmed
        .split_whitespace()
        .skip_while(|w| filler.iter().any(|f| w.to_lowercase() == *f))
        .take(8)
        .collect();
    if words.len() < 2 {
        return None;
    }
    Some(words.join(" "))
}

/// Detect signal-stat placeholder strings injected as fake transcripts
/// for chat-rate / emote-burst candidates. These should never reach the
/// title generator. Match is substring-based on lowercase to catch
/// variations ("11 chat messages in this window", "120 chat messages
/// in this 30s window", etc.).
fn is_signal_placeholder(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("chat messages in this")
        || lower.contains("emote occurrences")
        || lower.contains("emote occurrences in this")
}

fn is_vague_phrase(s: &str) -> bool {
    let wc = s.split_whitespace().count();
    if wc < 4 {
        return true;
    }
    let lower = s.to_lowercase();
    let vague = [
        "oh my god",
        "oh my gosh",
        "what the hell",
        "what the fuck",
        "no way dude",
        "are you serious",
        "holy shit",
    ];
    wc <= 4 && vague.iter().any(|v| lower.contains(v))
}

fn primary_event_from_tags(tags: Option<&str>) -> Option<&'static str> {
    let tag_str = tags?;
    let tag_list = parse_tags(Some(tag_str));
    let lower: Vec<String> = tag_list.iter().map(|t| t.to_lowercase()).collect();
    if lower
        .iter()
        .any(|t| t.contains("jumpscare") || t.contains("ambush"))
    {
        return Some("jumpscare");
    }
    if lower.iter().any(|t| t.contains("fight")) {
        return Some("fight");
    }
    if lower.iter().any(|t| t.contains("explosion")) {
        return Some("explosion");
    }
    if lower.iter().any(|t| t.contains("celebration")) {
        return Some("celebration");
    }
    if lower.iter().any(|t| t.contains("panic")) {
        return Some("panic");
    }
    if lower.iter().any(|t| t.contains("frustration")) {
        return Some("frustration");
    }
    if lower.iter().any(|t| t.contains("disbelief")) {
        return Some("disbelief");
    }
    if lower.iter().any(|t| t.contains("shock")) {
        return Some("shock");
    }
    if lower.iter().any(|t| t.contains("hype")) {
        return Some("hype");
    }
    if lower.iter().any(|t| t.contains("reaction")) {
        return Some("reaction");
    }
    if lower.iter().any(|t| t.contains("rapid")) {
        return Some("rapid cuts");
    }
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
    usage: &mut TitleUsage,
    title_config: &crate::game_config::TitleConfig,
) -> String {
    let tags = parse_tags(tags_str);

    // Compute the title via the layer cascade WITHOUT incrementing `usage`
    // intermediately. Internal helpers (`aftermath_from_tags`,
    // `grounded_highlight_title`) read the map to prefer least-used variants
    // but never mutate it. We increment exactly once at the bottom of this
    // function, after the final title is locked in. This single-site
    // increment prevents the double-counting that would otherwise happen
    // if multiple layers participated in selection for the same clip.
    let title = (|| -> String {
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
        // past-tense templated lines, anchored on the game when available. Picks
        // the least-used variant per tag combo so multiple clips sharing a
        // dominant tag don't collide on the same line within a batch.
        if let Some(line) =
            aftermath_from_tags(&tags, game_name, start_seconds, usage, title_config)
        {
            return line;
        }

        // Layer 3: Fall back to the legacy grounded heuristic. Still used when
        // neither of the above produces something Wave 3-shaped.
        grounded_highlight_title(transcript_excerpt, tags_str, start_seconds, usage)
    })();

    // Single-site usage increment — see closure preamble for rationale.
    *usage.entry(title.clone()).or_insert(0) += 1;
    title
}

/// Pick a Wave 3 AftermathConfession-style template based on event tags.
///
/// Each tag combo has 5+ phrase variants. Picks the least-used variant
/// according to `usage` (with `start_seconds` as the deterministic
/// tiebreaker), so multiple clips sharing a dominant tag in the same VOD
/// batch produce different titles. Previous behavior (`start_seconds %
/// pool.len()`) deterministically chose the same index when start times
/// happened to share a residue — this is what produced the bug where
/// 4 clips all landed on "had no warning whatsoever" because their
/// start_seconds all hit `% 5 == 2`.
///
/// Game-anchored variants (where `{game}` is templated in) are added to
/// the pool when a game name is available, expanding the pool from 5 to
/// ~6-7 entries and giving more headroom before duplicates start showing.
///
/// Returns `None` if no tag combo matches — caller should fall through
/// to `grounded_highlight_title`.
fn aftermath_from_tags(
    tags: &[String],
    game_name: Option<&str>,
    start_seconds: f64,
    usage: &TitleUsage,
    title_config: &crate::game_config::TitleConfig,
) -> Option<String> {
    let has = |needle: &str| tags.iter().any(|t| t.contains(needle));
    let game = game_name
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_lowercase);
    let idx = start_seconds.max(0.0) as usize;

    // Build a phrase pool, optionally including game-anchored variants,
    // and pick the least-used variant. `usage` is read-only here — the
    // outer `save_path_heuristic_title` increments after committing.
    let pick = |base: &[&str], game_templates: &[&str]| -> String {
        let mut pool: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
        if let Some(ref g) = game {
            for t in game_templates {
                pool.push(t.replace("{game}", g));
            }
        }
        pick_least_used(&pool, usage, idx)
    };

    // Helper: check if a category is allowed by title_config.disabled_categories.
    let is_category_enabled = |category: &str| -> bool {
        !title_config
            .disabled_categories
            .iter()
            .any(|c| c == category)
    };

    // TODO(v1.3.x): preferred_categories ordering is not yet implemented in
    // the if-chain — the chain returns on first match in source order. To
    // honor preferred_categories we'd need to either: (a) reorder branches
    // based on the list, or (b) collect all matching branches and pick by
    // preference. For v1.3.11, preferred_categories is captured in the
    // config but applied only via the "extras" mechanism (no-op for now).
    let _ = title_config.preferred_categories;

    // Each category has 15 base variants + 3 game-anchored variants when a
    // game name is available, giving an 18-deep pool per category. Sized
    // so a typical 17-clip Otzdarva-tier VOD can produce 0 in-category
    // duplicates even when 12+ clips collapse onto the same dominant tag
    // (the worst case observed in real validation runs).
    if (has("ambush") || has("jumpscare")) && is_category_enabled("ambush") {
        return Some(pick(
            &[
                "ambushed before i could move",
                "got caught completely off guard",
                "had zero seconds to react",
                "didn't even have my hands on the keys",
                "blindsided in the worst way",
                "never saw it coming",
                "thought i was alone. i wasn't.",
                "looked away for one second",
                "no chance to even flinch",
                "got jumped from behind nothing",
                "didn't even hear footsteps",
                "the second i stopped checking corners",
                "they were waiting for that exact moment",
                "let my guard down once",
                "should have been paying attention",
            ],
            &[
                "{game} ambushed me before i could move",
                "{game} doesn't believe in fair fights",
                "got jumped in {game} for the hundredth time",
            ],
        ));
    }
    if has("fight") && has("panic") && is_category_enabled("fight+panic") {
        return Some(pick(
            &[
                "panicked mid-fight",
                "every option was the wrong one",
                "lost the plot at the worst time",
                "had a strategy. then i didn't.",
                "tried to remember which button does what",
                "brain went somewhere else",
                "could not pick a button to save my life",
                "knew what to do. did the opposite.",
                "muscle memory abandoned me",
                "started pressing things at random",
                "couldn't think and play at the same time",
                "ran out of plans mid-execution",
                "forgot how the game works",
                "panic took the wheel",
                "the controls were against me",
            ],
            &[
                "panicked mid-fight in {game}",
                "{game} found my panic button",
                "{game} broke my brain in real time",
            ],
        ));
    }
    if has("fight") && has("frustration") && is_category_enabled("fight+frustration") {
        return Some(pick(
            &[
                "couldn't survive that fight",
                "got served mid-combo",
                "deserved better than this",
                "everything went wrong at once",
                "no version of me wins that one",
                "did everything right and still lost",
                "the game decided i was losing today",
                "my best wasn't close to enough",
                "got read like a book",
                "outplayed in real time",
                "no recovery from that one",
                "stat-checked into oblivion",
                "this fight was over before it started",
                "earned the loss honestly",
                "ran out of answers fast",
            ],
            &[
                "{game} did not play fair this round",
                "{game} won that fair and square",
                "{game} ate my lunch this round",
            ],
        ));
    }
    if has("celebration") && has("hype") && is_category_enabled("celebration+hype") {
        return Some(pick(
            &[
                "clutched it at the last second",
                "shouldn't have worked but it did",
                "actually pulled it off somehow",
                "luck did most of that",
                "made it out by inches",
                "no idea how i won that",
                "did not deserve that win",
                "stole that one fair and square",
                "luck carried, not skill",
                "got it on the last frame",
                "the universe owed me one",
                "looked planned. wasn't planned.",
                "this is going on the highlight reel",
                "won and i'm taking it",
                "ugly but it counts",
            ],
            &[
                "{game} let me have one for once",
                "stole one back from {game}",
                "{game} owed me that round",
            ],
        ));
    }
    if has("death") && is_category_enabled("death") {
        return Some(pick(
            &[
                "broke me before i blinked",
                "ended my run mid-stride",
                "showed me the loading screen",
                "wiped me without saying anything",
                "made me reconsider my life choices",
                "the run ends here",
                "got humbled in record time",
                "had hopes. they're dead now.",
                "back to the menu i go",
                "death came quietly",
                "lasted exactly long enough to fail",
                "the game stopped letting me play",
                "ran out of healthbar",
                "knew the risks. did them anyway.",
                "made it pretty far i guess",
            ],
            &[
                "{game} broke me before i blinked",
                "got dismissed by {game}",
                "{game} took my run home",
            ],
        ));
    }
    if has("explosion") && is_category_enabled("explosion") {
        return Some(pick(
            &[
                "blew up before i could react",
                "the explosion arrived first",
                "fireworks i did not order",
                "got erased by sudden physics",
                "vaporized mid-sentence",
                "physics had other plans",
                "everything became confetti",
                "got launched without warning",
                "deleted by mistake",
                "sent into the next zip code",
                "the kaboom was too much for me",
                "removed from the situation entirely",
                "rocket-jumped against my will",
                "exited the building, immediately",
                "got returned to sender",
            ],
            &[
                "{game} skipped me to the explosion part",
                "blown out of {game} entirely",
                "{game}'s physics chose violence",
            ],
        ));
    }
    if (has("disbelief") || has("shock")) && is_category_enabled("disbelief+shock") {
        return Some(pick(
            &[
                "didn't see that one coming",
                "still trying to understand what happened",
                "had no warning whatsoever",
                "wasn't ready and it showed",
                "couldn't process it in time",
                "what just happened",
                "the brain hadn't caught up yet",
                "took me a second to register that",
                "wait — what?",
                "is that legal",
                "no one was prepared for that",
                "still parsing this in real time",
                "the disbelief is real",
                "looked at the screen like it betrayed me",
                "missed the part where that became possible",
            ],
            &[
                "{game} pulled something new on me",
                "{game} just invented a new way to humble me",
                "didn't know {game} could do that",
            ],
        ));
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
    vod_id: &str,
    db: &Mutex<rusqlite::Connection>,
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

        // Use the highlight's stored transcript snippet — it's the relevant
        // excerpt for this clip range, set during signal fusion.
        let transcript_for_clip = h.transcript_snippet.as_deref();
        let brief = build_moment_brief(
            Some(h),
            transcript_for_clip,
            h.description.as_deref(),
            vod_game,
            None,
        );
        let event_summary = brief.prompt_context();
        let money_quote = brief
            .quote_candidates
            .first()
            .map(|quote| quote.text.as_str());

        // The quote comes from the same local brief, so the save path still
        // makes only one optional BYOK request per clip.
        let mut usage = post_captions::TokenUsage::default();
        match post_captions::generate_llm_titles(
            resolved.provider,
            &resolved.api_key,
            &resolved.model,
            &event_summary,
            money_quote,
            transcript_for_clip,
            &tags,
            vod_game,
            None, // streamer_history — fresh analyze, no prior titles
            None, // target_platform — defaults to TikTok
            Some(&mut usage),
        )
        .await
        {
            Ok(candidates) => {
                if let Some(top) = candidates
                    .iter()
                    .find(|candidate| brief.validate_title(&candidate.text).is_ok())
                {
                    log::info!(
                        "Save-path Wave 3 title for highlight {}: \"{}\" (pattern {:?}, score {:.2})",
                        h.id, top.text, top.pattern, top.score,
                    );
                    h.description = Some(top.text.clone());
                    // Phase 6.0: record this call in ai_usage_log.
                    if let Ok(conn) = db.lock() {
                        crate::ai_usage::log_usage(
                            &conn,
                            crate::ai_usage::UsageEntry {
                                feature: "title_save",
                                provider: resolved.provider,
                                model: &resolved.model,
                                tokens_in: usage.tokens_in,
                                tokens_out: usage.tokens_out,
                                vod_id: Some(vod_id),
                                clip_id: Some(&h.id),
                                context: Some(&brief.signature),
                            },
                        );
                    }
                } else {
                    log::warn!(
                        "Save-path Wave 3: no grounded candidates for highlight {} — keeping local Moment Brief title",
                        h.id
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "Save-path Wave 3 failed for highlight {}: {} — keeping heuristic",
                    h.id,
                    e
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
pub(crate) fn count_active_signals(
    audio: f64,
    visual: f64,
    chat: f64,
    has_transcript: bool,
) -> usize {
    let mut n = 0;
    if audio > 0.1 {
        n += 1;
    }
    if visual > 0.1 {
        n += 1;
    }
    if chat > 0.1 {
        n += 1;
    }
    if has_transcript {
        n += 1;
    }
    n
}

/// Build a factual explanation: signal values + count.
pub(crate) fn build_highlight_explanation(
    audio: f64,
    visual: f64,
    chat: f64,
    has_transcript: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if audio > 0.0 {
        parts.push(format!("audio {:.0}%", audio * 100.0));
    }
    if visual > 0.0 {
        parts.push(format!("visual {:.0}%", visual * 100.0));
    }
    if chat > 0.0 {
        parts.push(format!("chat {:.0}%", chat * 100.0));
    }
    if has_transcript {
        parts.push("transcript match".into());
    }

    let count = parts.len();
    if parts.is_empty() {
        "No signal data".into()
    } else {
        format!(
            "{} signal{} — {}",
            count,
            if count != 1 { "s" } else { "" },
            parts.join(", ")
        )
    }
}

/// Store an accepted or edited generated-copy choice in the local profile.
#[tauri::command]
pub fn record_copy_feedback(
    feedback_id: String,
    clip_id: String,
    brief_signature: String,
    copy_kind: String,
    strategy: String,
    generated_text: String,
    final_text: String,
    outcome: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    if !matches!(copy_kind.as_str(), "title" | "description") {
        return Err("Copy feedback kind must be title or description".into());
    }
    if !matches!(outcome.as_str(), "accepted" | "edited" | "rejected") {
        return Err("Copy feedback outcome is invalid".into());
    }
    if feedback_id.trim().is_empty()
        || clip_id.trim().is_empty()
        || brief_signature.trim().is_empty()
        || strategy.trim().is_empty()
    {
        return Err("Copy feedback is missing required context".into());
    }
    if generated_text.chars().count() > 5_000 || final_text.chars().count() > 5_000 {
        return Err("Copy feedback text is too long".into());
    }

    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::record_copy_feedback(
        &conn,
        &feedback_id,
        &clip_id,
        &brief_signature,
        &copy_kind,
        &strategy,
        &generated_text,
        &final_text,
        &outcome,
    )
    .map_err(|e| format!("Could not save local copy feedback: {e}"))
}

/// Generate a title and caption variants from one shared factual brief.
///
/// Local mode writes directly from the brief. BYOK mode may rewrite the same
/// brief, but every returned candidate is validated against the brief before it
/// reaches the editor.
#[tauri::command]
pub async fn generate_moment_copy(
    clip_id: String,
    seed: Option<u32>,
    transcript_text: Option<String>,
    current_title: Option<String>,
    current_game: Option<String>,
    current_description: Option<String>,
    previous_descriptions: Option<Vec<String>>,
    selected_mode: Option<String>,
    platform: Option<String>,
    db: State<'_, DbConn>,
) -> Result<MomentCopyResponse, String> {
    let (
        clip,
        highlight,
        vod,
        transcript,
        title_provider,
        caption_provider,
        title_preferences,
        caption_preferences,
    ) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Clip not found")?;
        let highlight = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {e}"))?
            .into_iter()
            .find(|row| row.id == clip.highlight_id);
        let vod = db::get_vod_by_id(&conn, &clip.vod_id).map_err(|e| format!("DB error: {e}"))?;
        let transcript = transcript_text
            .filter(|text| !text.trim().is_empty())
            .or_else(|| {
                highlight
                    .as_ref()
                    .and_then(|row| row.transcript_snippet.clone())
            });
        (
            clip,
            highlight,
            vod,
            transcript,
            ai_provider::resolve(&conn, ai_provider::Scope::Titles),
            ai_provider::resolve(&conn, ai_provider::Scope::Captions),
            db::get_copy_strategy_preferences(&conn, "title").unwrap_or_default(),
            db::get_copy_strategy_preferences(&conn, "description").unwrap_or_default(),
        )
    };

    let game_name = current_game
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(clip.game.as_deref())
        .or_else(|| vod.as_ref().and_then(|row| row.game_name.as_deref()));
    let stream_style = vod.as_ref().and_then(|row| {
        row.analyzed_stream_style
            .as_deref()
            .or(Some(row.detected_stream_style.as_str()))
    });
    let detector_title = current_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(Some(clip.title.as_str()));
    let brief = build_moment_brief(
        highlight.as_ref(),
        transcript.as_deref(),
        detector_title,
        game_name,
        stream_style,
    );
    let generation_seed = seed.unwrap_or(0);
    let tags = brief.tags.clone();
    let prompt_context = brief.prompt_context();
    let quote = brief
        .quote_candidates
        .first()
        .map(|item| item.text.as_str());

    let mut title_source = "free".to_string();
    let local_title = || {
        brief
            .title_suggestions(generation_seed, &title_preferences)
            .into_iter()
            .next()
            .ok_or_else(|| {
                "The clip does not have enough verified evidence for a specific title".to_string()
            })
    };
    let title_suggestion = if title_provider.is_llm() {
        let mut usage = post_captions::TokenUsage::default();
        let history = read_title_history(&clip_id);
        match post_captions::generate_llm_titles(
            title_provider.provider,
            &title_provider.api_key,
            &title_provider.model,
            &prompt_context,
            quote,
            transcript.as_deref(),
            &tags,
            game_name,
            (!history.is_empty()).then_some(history.as_slice()),
            Some(detection_platform(platform.as_deref())),
            Some(&mut usage),
        )
        .await
        {
            Ok(candidates) => {
                let grounded = candidates
                    .into_iter()
                    .find(|candidate| brief.validate_title(&candidate.text).is_ok());
                if let Some(candidate) = grounded {
                    title_source = "llm".to_string();
                    push_title_history(&clip_id, &candidate.text);
                    if let Ok(conn) = db.lock() {
                        crate::ai_usage::log_usage(
                            &conn,
                            crate::ai_usage::UsageEntry {
                                feature: "moment_brief_title",
                                provider: title_provider.provider,
                                model: &title_provider.model,
                                tokens_in: usage.tokens_in,
                                tokens_out: usage.tokens_out,
                                vod_id: Some(&clip.vod_id),
                                clip_id: Some(&clip_id),
                                context: Some(&brief.signature),
                            },
                        );
                    }
                    CopySuggestion {
                        text: candidate.text,
                        strategy: format!("byok:{:?}", candidate.pattern).to_lowercase(),
                    }
                } else if title_provider.fallback_to_free {
                    local_title()?
                } else {
                    return Err("BYOK returned titles that were not grounded in this clip".into());
                }
            }
            Err(error) if title_provider.fallback_to_free => {
                log::warn!("Moment Brief title rewrite failed, using local copy: {error}");
                local_title()?
            }
            Err(error) => return Err(format!("Title generation failed: {error}")),
        }
    } else {
        local_title()?
    };
    let title = feedback_suggestion(title_suggestion);

    let mode = selected_mode.unwrap_or_else(|| "punchy".to_string());
    let modes: Vec<&str> = if caption_provider.is_llm() {
        vec![mode.as_str()]
    } else {
        vec![
            "punchy",
            "clean",
            "funny",
            "hype",
            "search",
            "minimal",
            "direct_quote",
            "blame",
            "internal_thought",
            "observation",
        ]
    };

    let mut source = "free".to_string();
    let mut captions = Vec::new();
    if caption_provider.is_llm() {
        let mut avoid_captions = Vec::new();
        for caption in current_description
            .iter()
            .chain(previous_descriptions.iter().flatten())
            .chain(clip.publish_description.iter())
        {
            let trimmed = caption.trim();
            if !trimmed.is_empty()
                && !avoid_captions
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
            {
                avoid_captions.push(trimmed.to_string());
            }
            if avoid_captions.len() == 6 {
                break;
            }
        }

        let (audio, visual, chat) = highlight
            .as_ref()
            .map(|row| (row.audio_score, row.visual_score, row.chat_score))
            .unwrap_or_default();
        let tone =
            post_captions::classify_tone_pub(&tags, transcript.as_deref(), audio, visual, chat);
        let mut usage = post_captions::TokenUsage::default();
        match post_captions::generate_llm_caption(
            caption_provider.provider,
            &caption_provider.api_key,
            &caption_provider.model,
            &mode,
            Some(detection_platform(platform.as_deref())),
            &prompt_context,
            quote,
            quote,
            tone.label(),
            &tags,
            transcript.as_deref(),
            &title.text,
            game_name,
            &[],
            &avoid_captions,
            generation_seed,
            Some(&mut usage),
        )
        .await
        {
            Ok(candidates) => {
                let grounded = candidates.into_iter().find_map(|candidate| {
                    let variant = post_captions::caption_candidate_to_variant(&candidate, &mode);
                    brief
                        .validate_description(&variant.text, &title.text)
                        .is_ok()
                        .then_some(variant.text)
                });
                if let Some(text) = grounded {
                    source = "llm".to_string();
                    captions.push(MomentCaptionSuggestion {
                        mode: mode.clone(),
                        label: caption_mode_label(&mode).to_string(),
                        text,
                        strategy: format!("byok:{}:{}", mode, generation_seed % 6),
                        feedback_id: uuid::Uuid::new_v4().to_string(),
                    });
                    if let Ok(conn) = db.lock() {
                        crate::ai_usage::log_usage(
                            &conn,
                            crate::ai_usage::UsageEntry {
                                feature: "moment_brief_caption",
                                provider: caption_provider.provider,
                                model: &caption_provider.model,
                                tokens_in: usage.tokens_in,
                                tokens_out: usage.tokens_out,
                                vod_id: Some(&clip.vod_id),
                                clip_id: Some(&clip_id),
                                context: Some(&brief.signature),
                            },
                        );
                    }
                } else if !caption_provider.fallback_to_free {
                    return Err("BYOK returned captions that were not grounded in this clip".into());
                }
            }
            Err(error) if caption_provider.fallback_to_free => {
                log::warn!("Moment Brief caption rewrite failed, using local copy: {error}");
            }
            Err(error) => return Err(format!("Caption generation failed: {error}")),
        }
    }

    if captions.is_empty() {
        for (index, candidate_mode) in modes.iter().enumerate() {
            if let Some(suggestion) = brief
                .caption_suggestions(
                    candidate_mode,
                    &title.text,
                    generation_seed.saturating_add(index as u32),
                    &caption_preferences,
                )
                .into_iter()
                .next()
            {
                captions.push(MomentCaptionSuggestion {
                    mode: (*candidate_mode).to_string(),
                    label: caption_mode_label(candidate_mode).to_string(),
                    text: suggestion.text,
                    strategy: suggestion.strategy,
                    feedback_id: uuid::Uuid::new_v4().to_string(),
                });
            }
        }
    }
    if captions.is_empty() {
        return Err(
            "The clip does not have enough verified evidence for specific post copy".into(),
        );
    }

    let (audio, visual, chat) = highlight
        .as_ref()
        .map(|row| (row.audio_score, row.visual_score, row.chat_score))
        .unwrap_or_default();
    let tone = post_captions::classify_tone_pub(&tags, transcript.as_deref(), audio, visual, chat);
    let hashtags = post_captions::build_hashtags_v2(
        &tags,
        tone,
        detection_platform(platform.as_deref()),
        &[],
        game_name,
    );

    Ok(MomentCopyResponse {
        title,
        captions,
        hashtags,
        source,
        title_source,
        brief,
    })
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
    current_description: Option<String>,
    previous_descriptions: Option<Vec<String>>,
    selected_mode: Option<String>,
    db: State<'_, DbConn>,
) -> Result<post_captions::PostCaptions, String> {
    let (clip, tags, transcript, highlight_scores, stored_event_summary, resolved) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.iter().find(|h| h.id == clip.highlight_id);

        let tags = parse_tags(highlight.and_then(|h| h.tags.as_deref()));
        let stored_event_summary = highlight
            .and_then(|h| h.event_summary.clone())
            .filter(|summary| !summary.trim().is_empty())
            .or_else(|| {
                highlight
                    .and_then(|h| h.description.clone())
                    .filter(|summary| !summary.trim().is_empty())
            });

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

        (
            clip,
            tags,
            transcript,
            scores,
            stored_event_summary,
            resolved,
        )
    };

    // Use frontend title if provided, otherwise fall back to clip title
    let title = current_title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| clip.title.clone());

    let (audio, visual, chat) = highlight_scores;

    let generation_seed = seed.unwrap_or(0);
    // Action-first is a safer default than quote mode when transcript quality is weak.
    let mode = selected_mode.unwrap_or_else(|| "punchy".into());

    // ── Try LLM generation if provider is configured ──
    if resolved.is_llm() {
        let tone =
            post_captions::classify_tone_pub(&tags, transcript.as_deref(), audio, visual, chat);
        let event = post_captions::primary_event_pub(&tags);
        let event_summary = stored_event_summary.clone().unwrap_or_else(|| {
            post_captions::synthesize_event_pub(event, tone, &tags, generation_seed as usize)
        });
        let tone_label = tone.label();
        let quote = post_captions::strong_quote_pub(transcript.as_deref());

        // Prefer live game value from frontend; fall back to DB value
        let game_name = current_game
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(clip.game.as_deref());

        log::info!(
            "Caption generation: using {:?} (model: {})",
            resolved.provider,
            resolved.model
        );
        log::info!(
            "Caption generation: mode = {}, game = {:?}",
            mode,
            game_name
        );

        // Wave 3: extract a money-quote first (tiny, separate API call). Non-fatal
        // if it fails — captions still generate without one, just less punchy.
        let mut quote_usage = post_captions::TokenUsage::default();
        let money_quote: Option<String> =
            match transcript.as_deref().filter(|t| !t.trim().is_empty()) {
                Some(ft) => match post_captions::extract_money_quote_llm(
                    resolved.provider,
                    &resolved.api_key,
                    &resolved.model,
                    &event_summary,
                    ft,
                    &tags,
                    Some(&mut quote_usage),
                )
                .await
                {
                    Ok(q) => {
                        if let Ok(conn) = db.lock() {
                            crate::ai_usage::log_usage(
                                &conn,
                                crate::ai_usage::UsageEntry {
                                    feature: "money_quote_caption",
                                    provider: resolved.provider,
                                    model: &resolved.model,
                                    tokens_in: quote_usage.tokens_in,
                                    tokens_out: quote_usage.tokens_out,
                                    vod_id: Some(&clip.vod_id),
                                    clip_id: Some(&clip_id),
                                    context: None,
                                },
                            );
                        }
                        q
                    }
                    Err(e) => {
                        log::debug!("Money-quote extraction skipped: {}", e);
                        None
                    }
                },
                None => None,
            };

        let mut caption_usage = post_captions::TokenUsage::default();
        let mut avoid_captions = Vec::new();
        for caption in current_description
            .iter()
            .chain(previous_descriptions.iter().flatten())
            .chain(clip.publish_description.iter())
        {
            let trimmed = caption.trim();
            if trimmed.is_empty()
                || avoid_captions
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
            {
                continue;
            }
            avoid_captions.push(trimmed.to_string());
            if avoid_captions.len() == 6 {
                break;
            }
        }
        match post_captions::generate_llm_caption(
            resolved.provider,
            &resolved.api_key,
            &resolved.model,
            &mode,
            None, // platform — defaults to TikTok; a future frontend selector can override
            &event_summary,
            money_quote.as_deref(),
            quote.as_deref(),
            tone_label,
            &tags,
            transcript.as_deref(),
            &title,
            game_name,
            &[], // streamer_niche_tags — surface from settings in future work
            &avoid_captions,
            generation_seed,
            Some(&mut caption_usage),
        )
        .await
        {
            Ok(candidates) if !candidates.is_empty() => {
                let top = &candidates[0];
                log::info!(
                    "Wave 3: {} caption candidate(s) for clip {} (mode: {}, top score {:.2}, hook \"{}\")",
                    candidates.len(), clip_id, mode, top.score, top.hook_line,
                );
                // Phase 6.0: log caption regen call.
                if let Ok(conn) = db.lock() {
                    crate::ai_usage::log_usage(
                        &conn,
                        crate::ai_usage::UsageEntry {
                            feature: "caption_regen",
                            provider: resolved.provider,
                            model: &resolved.model,
                            tokens_in: caption_usage.tokens_in,
                            tokens_out: caption_usage.tokens_out,
                            vod_id: Some(&clip.vod_id),
                            clip_id: Some(&clip_id),
                            context: Some(&mode),
                        },
                    );
                }
                // Top-scored only. Candidates are pre-sorted descending by score.
                let llm_captions = vec![post_captions::caption_candidate_to_variant(top, &mode)];
                // Wave 1 platform-aware hashtags: TikTok evergreen tags differ from
                // YouTube Shorts (fyp vs shorts), and the v2 builder reserves a slot
                // for the game name when provided. streamer_niche_tags stays empty
                // until Settings exposes it (future work).
                let hashtags = post_captions::build_hashtags_v2(
                    &tags,
                    tone,
                    crate::detection::Platform::TikTok,
                    &[],
                    game_name,
                );
                let casual = llm_captions
                    .first()
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                let funny = llm_captions
                    .get(1)
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                let hype = llm_captions
                    .get(2)
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                return Ok(post_captions::PostCaptions {
                    captions: llm_captions,
                    hashtags,
                    source: "llm".into(),
                    casual,
                    funny,
                    hype,
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
    Ok(post_captions::generate_from_parts_with_summary(
        &tags,
        transcript.as_deref(),
        &title,
        stored_event_summary.as_deref(),
        clip.start_seconds,
        audio,
        visual,
        chat,
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
    let (clip, highlight, vod, tags, transcript, resolved, title_preferences) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.into_iter().find(|h| h.id == clip.highlight_id);

        let tags = parse_tags(highlight.as_ref().and_then(|row| row.tags.as_deref()));

        let transcript = transcript_text
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                highlight
                    .as_ref()
                    .and_then(|h| h.transcript_snippet.clone())
            });
        let vod = db::get_vod_by_id(&conn, &clip.vod_id).map_err(|e| format!("DB error: {e}"))?;

        let resolved = ai_provider::resolve(&conn, ai_provider::Scope::Titles);
        let title_preferences =
            db::get_copy_strategy_preferences(&conn, "title").unwrap_or_default();

        (
            clip,
            highlight,
            vod,
            tags,
            transcript,
            resolved,
            title_preferences,
        )
    };

    let game_name = current_game
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(clip.game.as_deref())
        .or_else(|| vod.as_ref().and_then(|row| row.game_name.as_deref()));
    let stream_style = vod.as_ref().and_then(|row| {
        row.analyzed_stream_style
            .as_deref()
            .or(Some(row.detected_stream_style.as_str()))
    });
    let brief = build_moment_brief(
        highlight.as_ref(),
        transcript.as_deref(),
        current_title.as_deref().or(Some(clip.title.as_str())),
        game_name,
        stream_style,
    );

    if resolved.is_llm() {
        let event_summary = brief.prompt_context();

        log::info!(
            "AI title generation: using {:?} (model: {})",
            resolved.provider,
            resolved.model
        );

        let money_quote = brief
            .quote_candidates
            .first()
            .map(|quote| quote.text.as_str());

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

        let mut title_usage = post_captions::TokenUsage::default();
        match post_captions::generate_llm_titles(
            resolved.provider,
            &resolved.api_key,
            &resolved.model,
            &event_summary,
            money_quote,
            transcript.as_deref(),
            &tags,
            game_name,
            history_slice,
            None, // target_platform — defaults to TikTok
            Some(&mut title_usage),
        )
        .await
        {
            Ok(candidates) => {
                if let Some(top) = candidates
                    .iter()
                    .find(|candidate| brief.validate_title(&candidate.text).is_ok())
                {
                    log::info!(
                        "Wave 3 title for clip {}: \"{}\" (pattern {:?}, score {:.2}, {} candidates)",
                        clip_id, top.text, top.pattern, top.score, candidates.len(),
                    );
                    // Phase 6.0: log title regen call.
                    if let Ok(conn) = db.lock() {
                        crate::ai_usage::log_usage(
                            &conn,
                            crate::ai_usage::UsageEntry {
                                feature: "title_regen",
                                provider: resolved.provider,
                                model: &resolved.model,
                                tokens_in: title_usage.tokens_in,
                                tokens_out: title_usage.tokens_out,
                                vod_id: Some(&clip.vod_id),
                                clip_id: Some(&clip_id),
                                context: Some(&brief.signature),
                            },
                        );
                    }
                    // Record this regen so subsequent calls on the same clip see
                    // the full session history, not just the current UI title.
                    push_title_history(&clip_id, &top.text);
                    return Ok(top.text.clone());
                }
                log::warn!("LLM returned no title grounded in the Moment Brief");
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

    brief
        .title_suggestions(0, &title_preferences)
        .into_iter()
        .next()
        .map(|suggestion| suggestion.text)
        .ok_or_else(|| {
            "The clip does not have enough verified evidence for a specific title".into()
        })
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
                s => {
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
                s => {
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
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        _ => Err(format!("Unknown provider: {}", provider)),
    }
}
