//! User-facing label generation for detected clips.
//!
//! Turns raw signal data (tags, scores, transcript, vision descriptions)
//! into short, human-readable labels that make the clip list feel
//! understandable instead of being a wall of scored timestamps.
//!
//! # What gets generated
//!
//! | Field       | Example                              | Source priority          |
//! |-------------|--------------------------------------|-------------------------|
//! | `title`     | "Huge Reaction After Clutch Play"    | vision > transcript > tags |
//! | `hook`      | "Wait for the scream..."             | transcript > tags        |
//! | `reason`    | "Audio spike + excited speech"       | score breakdown          |
//!
//! Labels are built from tags and transcript heuristics.
//! Analysis is always local — no external API calls.

use crate::pipeline::{CandidateClip, ClipScoreBreakdown};

// ═══════════════════════════════════════════════════════════════════
//  Output
// ═══════════════════════════════════════════════════════════════════

/// Labels generated for a single clip.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClipLabels {
    /// Short title for the clip list (≤60 chars).
    /// Example: "Big Reaction to Jumpscare"
    pub title: String,
    /// One-line hook / tease for thumbnails or previews (≤40 chars).
    /// Example: "Wait for it..."
    pub hook: String,
    /// Short explanation of why this clip was selected.
    /// Example: "Audio spike + shocked speech + scene cut"
    pub reason: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Generate labels for a single clip and write them onto it in-place.
///
/// Sets `title` and `hook`.  The full scoring explanation lives on
/// `score_report`, which is set by the ranker — the labeler only
/// handles the user-facing text labels.
pub fn label_clip(clip: &mut CandidateClip) {
    let labels = generate(clip);

    if clip.title.is_none() {
        clip.title = Some(labels.title);
    }
    if clip.hook.is_none() {
        clip.hook = Some(labels.hook);
    }
}

/// Generate labels for every clip in a list.
pub fn label_all(clips: &mut [CandidateClip]) {
    for clip in clips.iter_mut() {
        label_clip(clip);
    }
}

/// Generate labels without writing them (for inspection / testing).
pub fn generate(clip: &CandidateClip) -> ClipLabels {
    let title = build_title(clip);
    let hook = build_hook(clip);
    let reason = build_reason(clip);

    ClipLabels { title, hook, reason }
}

// ═══════════════════════════════════════════════════════════════════
//  Title generation
// ═══════════════════════════════════════════════════════════════════
//
//  2-stage title system: ground truth → hook formatting.
//
//  Titles are engaging but never lie.  No fake hype words.
//  Target: 5–10 words.
//
//  Priority:
//    1. Vision model description
//    2. Reaction + Context  — transcript quote + event tag
//    3. Transcript alone    — if ≥4 specific words
//    4. Outcome-based       — compound tag pairs
//    5. Event + Tension     — event + natural phrase
//    6. Soft curiosity      — only very high confidence
//    7. Grounded fallback   — signal + timestamp

fn build_title(clip: &CandidateClip) -> String {
    // 1. Vision model
    if let Some(ref title) = clip.title {
        if !title.is_empty() {
            return truncate(title, 60);
        }
    }

    let phrase = extract_phrase(clip.transcript_excerpt.as_deref().unwrap_or(""));
    let event = primary_event(&clip.tags);
    let t = clip.start_time;

    // 2. Reaction + Context (strongest: real speech + event)
    if let Some(title) = reaction_context(&phrase, event) {
        return truncate(&title, 60);
    }

    // 3. Transcript alone (specific enough to stand on its own)
    if let Some(title) = standalone_quote(&phrase) {
        return truncate(&title, 60);
    }

    // 4. Outcome-based (compound tags imply a narrative)
    if let Some(title) = outcome_title(&clip.tags) {
        return truncate(&title, 60);
    }

    // 5. Event + Tension (single event, no transcript)
    if let Some(title) = event_tension(event, t) {
        return truncate(&title, 60);
    }

    // 6. Soft curiosity (rare — only very strong multi-signal)
    if let Some(title) = soft_curiosity(&clip.score_breakdown, t) {
        return truncate(&title, 60);
    }

    // 7. Grounded fallback
    let mins = (t as u32) / 60;
    let secs = (t as u32) % 60;
    let signal = dominant_signal_name(&clip.score_breakdown);
    format!("{} at {}:{:02}", signal, mins, secs)
}

// ── Stage 2 helpers: hook formatting ─────────────────────────────

/// Add a trailing period if the string has no terminal punctuation.
fn punctuate(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with('.') || t.ends_with('!') || t.ends_with('?') {
        t.to_string()
    } else {
        format!("{}.", t)
    }
}

/// Reaction + Context: `"quote." context_tag`
///
/// Best format — grounds the title in real speech and adds event
/// context without a mechanical connector like "during".
///
/// Examples:
///   `"Oh no." after the ambush`
///   `"What just happened..." instant reaction`
fn reaction_context(phrase: &Option<String>, event: Option<&str>) -> Option<String> {
    let phrase = phrase.as_ref()?;
    let event = event?;
    let ctx = context_tag(event);

    // Skip if quote already mentions the event
    if phrase.to_lowercase().contains(event) { return None; }

    let words: Vec<&str> = phrase.split_whitespace().take(5).collect();
    let quote = words.join(" ");

    let formatted = if word_count(phrase) > 5 {
        format!("{}...", quote)
    } else {
        punctuate(&quote)
    };

    Some(format!("\"{}\" {}", formatted, ctx))
}

/// Map events to gaming-native context tags.
/// Uses verbs and timing language over static nouns.
fn context_tag(event: &str) -> &'static str {
    match event {
        "jumpscare" | "ambush" => "caught off guard",
        "fight"       => "mid-fight",
        "explosion"   => "right before it blows up",
        "celebration" => "clutches it",
        "panic"       => "instant panic",
        "frustration" => "loses it",
        "disbelief"   => "didn't see that coming",
        "shock"       => "instant reaction",
        "hype"        => "peak hype",
        "reaction"    => "the reaction",
        "rapid cuts"  => "rapid-fire",
        _ => "out of nowhere",
    }
}

/// Standalone quote — only for specific transcripts (≥4 non-vague
/// words).  Short/generic exclamations get rejected so they fall
/// through to event-based formats.
fn standalone_quote(phrase: &Option<String>) -> Option<String> {
    let phrase = phrase.as_ref()?;
    if is_vague(phrase) { return None; }

    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() >= 7 {
        let short: Vec<&str> = words[..6].to_vec();
        Some(format!("\"{}...\"", short.join(" ")))
    } else {
        Some(format!("\"{}\"", phrase))
    }
}

/// Outcome-based: compound tags imply a narrative.
/// Uses verb-forward, stakes-aware phrasing.
fn outcome_title(tags: &[String]) -> Option<String> {
    let t = TagSet::new(tags);

    // Fight outcomes
    if t.has("fight") && t.has("celebration") {
        return Some("Fight breaks out and they clutch it".into());
    }
    if t.has("fight") && t.has("frustration") {
        return Some("Fight goes wrong and they lose it".into());
    }
    if t.has("fight") && t.has("panic") {
        return Some("Fight turns bad fast".into());
    }

    // Ambush/jumpscare outcomes
    if (t.has("ambush") || t.has("jumpscare")) && t.has("panic") {
        return Some("Ambush hits and panic sets in".into());
    }
    if (t.has("ambush") || t.has("jumpscare")) && t.has("shock") {
        return Some("Ambush out of nowhere".into());
    }
    if (t.has("ambush") || t.has("jumpscare")) && t.has("celebration") {
        return Some("Ambush happens but they barely survive".into());
    }

    // Clutch / hype
    if t.has("panic") && t.has("celebration") {
        return Some("Almost dies then clutches it".into());
    }
    if t.has("hype") && t.has("celebration") {
        return Some("Clutch play at the last second".into());
    }

    None
}

/// Event + Tension: verb-forward, timing-aware phrases.
/// Deterministic rotation seeded by `start_time`.
fn event_tension(event: Option<&str>, start_time: f64) -> Option<String> {
    let event = event?;
    let idx = start_time as usize;

    let phrases: &[&str] = match event {
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
        "explosion" => &[
            "Explosion hits out of nowhere",
            "Blows up with no warning",
        ],
        "panic" => &[
            "Panic hits instantly",
            "Everything goes wrong at once",
            "Panic sets in right away",
        ],
        "celebration" => &[
            "Clutches it at the last second",
            "Barely survives then celebrates",
        ],
        "frustration" => &[
            "Nothing goes right",
            "Loses it after that play",
        ],
        "shock" | "disbelief" => &[
            "Didn't see that coming",
            "Shock hits out of nowhere",
        ],
        "hype" => &[
            "Hype hits out of nowhere",
            "Goes off at the perfect time",
        ],
        "reaction" => &[
            "Reaction says it all",
            "Reacts instantly",
        ],
        "rapid cuts" => &[
            "Everything happens at once",
            "Too much too fast",
        ],
        _ => return None,
    };

    Some(phrases[idx % phrases.len()].to_string())
}

/// Soft curiosity — speed/timing phrasing for genuinely exceptional clips.
/// Gated behind 3+ strong signals so it's earned, not manufactured.
fn soft_curiosity(scores: &ClipScoreBreakdown, start_time: f64) -> Option<String> {
    if scores.active_signal_count() < 3 { return None; }
    if scores.best_raw() < 0.85 { return None; }
    let mut vals = vec![scores.audio_score, scores.speech_score, scores.scene_score];
    if let Some(v) = scores.vision_score { vals.push(v); }
    vals.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    if vals.len() < 2 || vals[1] < 0.65 { return None; }

    const PHRASES: &[&str] = &[
        "This happens way too fast",
        "Zero time to react",
        "Watch what happens next",
    ];
    Some(PHRASES[start_time as usize % PHRASES.len()].to_string())
}

// ── Stage 1 helpers: ground truth extraction ─────────────────────

/// Extract the most informative phrase from a transcript excerpt.
/// Drops leading filler, takes up to 8 words.
fn extract_phrase(excerpt: &str) -> Option<String> {
    let trimmed = excerpt.trim();
    if trimmed.len() < 3 { return None; }

    let filler = ["like", "so", "um", "uh", "okay", "ok", "well", "and", "but"];
    let words: Vec<&str> = trimmed.split_whitespace()
        .skip_while(|w| filler.contains(&w.to_lowercase().as_str()))
        .take(8)
        .collect();

    if words.len() < 2 { return None; }
    Some(words.join(" "))
}

/// True if the phrase is too vague to stand alone.
fn is_vague(s: &str) -> bool {
    let wc = s.split_whitespace().count();
    if wc < 4 { return true; }
    let lower = s.to_lowercase();
    let vague = ["oh my god", "oh my gosh", "what the hell", "what the fuck",
                  "no way dude", "are you serious", "holy shit"];
    wc <= 4 && vague.iter().any(|v| lower.contains(v))
}

/// Pick the primary event tag for context.
fn primary_event(tags: &[String]) -> Option<&'static str> {
    let t = TagSet::new(tags);
    if t.has("jumpscare") || t.has("ambush") { return Some("jumpscare"); }
    if t.has("fight")       { return Some("fight"); }
    if t.has("explosion")   { return Some("explosion"); }
    if t.has("celebration") { return Some("celebration"); }
    if t.has("panic")       { return Some("panic"); }
    if t.has("frustration") { return Some("frustration"); }
    if t.has("disbelief")   { return Some("disbelief"); }
    if t.has("shock")       { return Some("shock"); }
    if t.has("hype")        { return Some("hype"); }
    if t.has("reaction")    { return Some("reaction"); }
    if t.has("rapid_cuts")  { return Some("rapid cuts"); }
    None
}

fn word_count(s: &str) -> usize { s.split_whitespace().count() }

/// Factual label for the dominant signal, used in fallback titles.
fn dominant_signal_name(scores: &ClipScoreBreakdown) -> &'static str {
    let a = scores.audio_score;
    let s = scores.speech_score;
    let sc = scores.scene_score;
    let v = scores.vision_score.unwrap_or(0.0);
    let best = a.max(s).max(sc).max(v);

    if (a - best).abs() < 0.01 { "Audio peak" }
    else if (s - best).abs() < 0.01 { "Speech signal" }
    else if (sc - best).abs() < 0.01 { "Scene change" }
    else if (v - best).abs() < 0.01 { "Vision signal" }
    else { "Signal" }
}

// ═══════════════════════════════════════════════════════════════════
//  Hook generation
// ═══════════════════════════════════════════════════════════════════
//
//  The hook is a secondary line shown below the title.  It states
//  what the system measured — transcript quote or signal summary.

fn build_hook(clip: &CandidateClip) -> String {
    // Transcript: use the actual speech
    if let Some(ref excerpt) = clip.transcript_excerpt {
        let trimmed = excerpt.trim();
        if trimmed.len() >= 5 {
            if trimmed.len() <= 35 {
                return format!("\"{}\"", trimmed);
            }
            let first: String = trimmed.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
            return format!("\"{}...\"", first);
        }
    }

    // No transcript: describe which signals fired
    signal_summary(clip)
}

/// Short factual summary of the active signals.
fn signal_summary(clip: &CandidateClip) -> String {
    let b = &clip.score_breakdown;
    let mut parts: Vec<String> = Vec::new();

    if b.audio_score > 0.0 {
        parts.push(format!("Audio {:.0}%", b.audio_score * 100.0));
    }
    if b.speech_score > 0.0 {
        parts.push(format!("Speech {:.0}%", b.speech_score * 100.0));
    }
    if b.scene_score > 0.0 {
        parts.push(format!("Scene {:.0}%", b.scene_score * 100.0));
    }
    if let Some(v) = b.vision_score {
        if v > 0.0 {
            parts.push(format!("Vision {:.0}%", v * 100.0));
        }
    }

    if parts.is_empty() {
        "No strong signals".into()
    } else {
        parts.join(" · ")
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Reason generation (why this clip was selected)
// ═══════════════════════════════════════════════════════════════════
//
//  Lists the contributing signals in plain language.

fn build_reason(clip: &CandidateClip) -> String {
    let b = &clip.score_breakdown;

    // List active signals with their values
    let mut signals: Vec<(f64, &str)> = vec![
        (b.audio_score, "audio"),
        (b.speech_score, "speech"),
        (b.scene_score, "scene"),
    ];
    if let Some(vs) = b.vision_score {
        signals.push((vs, "vision"));
    }
    signals.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut parts: Vec<String> = Vec::new();
    for &(score, label) in &signals {
        if score >= 0.3 {
            parts.push(format!("{} ({:.0}%)", label, score * 100.0));
        }
    }

    if parts.is_empty() {
        if let Some(&(score, label)) = signals.first() {
            parts.push(format!("{} ({:.0}%)", label, score * 100.0));
        }
    }

    let active = b.active_signal_count();
    let suffix = match active {
        c if c >= 3 => format!(" — {} signals active", c),
        2 => " — 2 signals active".into(),
        _ => " — 1 signal".into(),
    };

    let mut reason = parts.join(", ");
    reason.push_str(&suffix);

    // Capitalize first letter
    if let Some(first) = reason.get_mut(..1) {
        first.make_ascii_uppercase();
    }

    reason
}

// ═══════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════

/// Truncate a string to `max` chars at a word boundary.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    // Walk back to the last space to avoid mid-word truncation
    if let Some(space) = s[..end].rfind(' ') {
        format!("{}...", &s[..space])
    } else {
        format!("{}...", &s[..end])
    }
}

/// Helper for checking tag membership without allocating.
struct TagSet<'a> {
    tags: &'a [String],
}

impl<'a> TagSet<'a> {
    fn new(tags: &'a [String]) -> Self {
        Self { tags }
    }

    fn has(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ClipScoreBreakdown, SignalType};

    fn make_clip(
        tags: &[&str],
        audio: f64,
        speech: f64,
        scene: f64,
        vision: Option<f64>,
    ) -> CandidateClip {
        let mut c = CandidateClip::new(
            60.0,
            85.0,
            ClipScoreBreakdown::new(audio, speech, scene, vision),
            vec![SignalType::Audio, SignalType::Transcript],
        );
        c.tags = tags.iter().map(|s| s.to_string()).collect();
        c
    }

    // ── Title generation ──

    #[test]
    fn vision_title_preserved() {
        let mut clip = make_clip(&["reaction"], 0.7, 0.5, 0.3, Some(0.9));
        clip.title = Some("Streamer Falls Off Chair".into());
        let labels = generate(&clip);
        assert_eq!(labels.title, "Streamer Falls Off Chair");
    }

    #[test]
    fn reaction_plus_context() {
        let mut clip = make_clip(&["shock"], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("Oh my god what just happened".into());
        let labels = generate(&clip);
        // Reaction + Context: quote + gaming context tag
        assert!(labels.title.starts_with('"'), "should quote: {}", labels.title);
        // "shock" maps to "instant reaction"
        assert!(labels.title.contains("instant reaction"), "title: {}", labels.title);
    }

    #[test]
    fn specific_transcript_stands_alone() {
        let mut clip = make_clip(&[], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("I can not believe he just did that to us".into());
        let labels = generate(&clip);
        // Specific enough (≥4 words), no tags → standalone quote
        assert!(labels.title.starts_with('"'), "should quote: {}", labels.title);
        // No context tag appended
        assert!(!labels.title.contains("reaction"), "standalone: {}", labels.title);
    }

    #[test]
    fn short_transcript_gets_context_tag() {
        let mut clip = make_clip(&["jumpscare"], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("Oh no".into());
        let labels = generate(&clip);
        // "Oh no" + jumpscare → `"Oh no." caught off guard`
        assert_eq!(labels.title, "\"Oh no.\" caught off guard");
    }

    #[test]
    fn single_word_falls_to_event_tension() {
        let mut clip = make_clip(&["jumpscare"], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("What".into());
        let labels = generate(&clip);
        // Single word → no phrase → event tension format
        let lower = labels.title.to_lowercase();
        assert!(lower.contains("ambush") || lower.contains("jumpscare"),
            "should use event: {}", labels.title);
    }

    #[test]
    fn no_transcript_uses_event_tension() {
        let clip = make_clip(&["jumpscare", "shock", "reaction"], 0.8, 0.5, 0.4, None);
        let labels = generate(&clip);
        // No transcript → event tension or outcome
        let lower = labels.title.to_lowercase();
        assert!(lower.contains("ambush") || lower.contains("jumpscare"),
            "should use event tension: {}", labels.title);
    }

    #[test]
    fn outcome_from_compound_tags() {
        let clip = make_clip(&["fight", "panic"], 0.8, 0.5, 0.4, None);
        let labels = generate(&clip);
        assert_eq!(labels.title, "Fight turns bad fast");
    }

    #[test]
    fn fallback_title_includes_signal_and_timestamp() {
        let clip = make_clip(&[], 0.3, 0.2, 0.1, None);
        let labels = generate(&clip);
        assert!(labels.title.contains("1:00"), "fallback should show time: {}", labels.title);
        assert!(labels.title.contains("Audio peak"), "fallback should name signal: {}", labels.title);
    }

    #[test]
    fn title_truncated_at_60_chars() {
        let mut clip = make_clip(&[], 0.5, 0.5, 0.3, Some(0.8));
        clip.title = Some("A".repeat(100));
        let labels = generate(&clip);
        assert!(labels.title.len() <= 63); // 60 + "..."
    }

    // ── Hook generation ──

    #[test]
    fn transcript_becomes_hook() {
        let mut clip = make_clip(&["reaction"], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("No way dude".into());
        let labels = generate(&clip);
        assert_eq!(labels.hook, "\"No way dude\"");
    }

    #[test]
    fn long_transcript_truncated_in_hook() {
        let mut clip = make_clip(&[], 0.7, 0.6, 0.3, None);
        clip.transcript_excerpt = Some("Oh my god I can not believe what just happened that was insane".into());
        let labels = generate(&clip);
        assert!(labels.hook.len() <= 40, "hook too long: {}", labels.hook);
        assert!(labels.hook.ends_with("...\""));
    }

    #[test]
    fn jumpscare_hook_shows_signals() {
        let clip = make_clip(&["jumpscare"], 0.8, 0.0, 0.3, None);
        let labels = generate(&clip);
        // No transcript → signal summary
        assert!(labels.hook.contains("Audio 80%"), "hook: {}", labels.hook);
        assert!(labels.hook.contains("Scene 30%"), "hook: {}", labels.hook);
    }

    #[test]
    fn hype_hook_shows_signals() {
        let clip = make_clip(&["hype", "celebration"], 0.6, 0.5, 0.3, None);
        let labels = generate(&clip);
        assert!(labels.hook.contains("Audio 60%"), "hook: {}", labels.hook);
    }

    #[test]
    fn no_transcript_hook_shows_signal_values() {
        let clip = make_clip(&[], 0.9, 0.9, 0.5, None);
        let labels = generate(&clip);
        // Hook should show actual signal percentages
        assert!(labels.hook.contains("Audio 90%"), "hook: {}", labels.hook);
        assert!(labels.hook.contains("Speech 90%"), "hook: {}", labels.hook);
    }

    // ── Reason generation ──

    #[test]
    fn reason_shows_signal_values() {
        let clip = make_clip(&["reaction"], 0.8, 0.7, 0.3, None);
        let labels = generate(&clip);
        // First signal capitalized, rest lowercase
        let lower = labels.reason.to_lowercase();
        assert!(lower.contains("audio (80%)"), "reason: {}", labels.reason);
        assert!(lower.contains("speech (70%)"), "reason: {}", labels.reason);
        assert!(lower.contains("scene (30%)"), "reason: {}", labels.reason);
    }

    #[test]
    fn reason_shows_signal_count() {
        let clip = make_clip(&["reaction"], 0.8, 0.7, 0.6, None);
        let labels = generate(&clip);
        assert!(labels.reason.contains("3 signals active"),
            "reason: {}", labels.reason);
    }

    #[test]
    fn reason_mentions_vision_when_present() {
        let clip = make_clip(&[], 0.5, 0.3, 0.2, Some(0.9));
        let labels = generate(&clip);
        let lower = labels.reason.to_lowercase();
        assert!(lower.contains("vision (90%)"), "reason: {}", labels.reason);
    }

    #[test]
    fn reason_always_has_at_least_one_signal() {
        let clip = make_clip(&[], 0.1, 0.05, 0.02, None);
        let labels = generate(&clip);
        assert!(!labels.reason.is_empty());
        let lower = labels.reason.to_lowercase();
        assert!(lower.contains("audio"), "reason: {}", labels.reason);
    }

    // ── label_clip in-place ──

    #[test]
    fn label_clip_sets_title_and_hook() {
        let mut clip = make_clip(&["shock", "reaction"], 0.8, 0.6, 0.4, None);
        assert!(clip.title.is_none());
        assert!(clip.hook.is_none());

        label_clip(&mut clip);

        assert!(clip.title.is_some());
        assert!(clip.hook.is_some());
    }

    #[test]
    fn label_clip_does_not_overwrite_vision_title() {
        let mut clip = make_clip(&["reaction"], 0.7, 0.5, 0.3, Some(0.9));
        clip.title = Some("Model-Generated Title".into());

        label_clip(&mut clip);

        assert_eq!(clip.title.as_deref(), Some("Model-Generated Title"));
    }

    #[test]
    fn label_all_processes_every_clip() {
        let mut clips = vec![
            make_clip(&["shock"], 0.8, 0.6, 0.3, None),
            make_clip(&["hype"], 0.6, 0.7, 0.4, None),
            make_clip(&["fight"], 0.9, 0.3, 0.5, None),
        ];

        label_all(&mut clips);

        for clip in &clips {
            assert!(clip.title.is_some(), "every clip should get a title");
            assert!(clip.hook.is_some(), "every clip should get a hook");
            // explanation comes from the ranker, not the labeler
        }
    }

    // ── Helpers ──

    #[test]
    fn truncate_at_word_boundary() {
        let s = "Hello world this is a test";
        let t = truncate(s, 15);
        assert_eq!(t, "Hello world...");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        let s = "Short";
        assert_eq!(truncate(s, 60), "Short");
    }

    #[test]
    fn dominant_signal_name_picks_strongest() {
        let audio_dom = ClipScoreBreakdown::new(0.9, 0.3, 0.2, None);
        assert_eq!(dominant_signal_name(&audio_dom), "Audio peak");

        let speech_dom = ClipScoreBreakdown::new(0.3, 0.8, 0.2, None);
        assert_eq!(dominant_signal_name(&speech_dom), "Speech signal");

        let scene_dom = ClipScoreBreakdown::new(0.1, 0.2, 0.5, None);
        assert_eq!(dominant_signal_name(&scene_dom), "Scene change");
    }
}
