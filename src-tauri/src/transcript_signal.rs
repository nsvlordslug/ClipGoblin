//! Transcript-based highlight scoring for clip detection.
//!
//! Analyzes timestamped speech-to-text output to find highlight-worthy
//! moments using five rule-based detectors:
//!
//! | Detector        | Finds                                        | Tags                    |
//! |-----------------|----------------------------------------------|-------------------------|
//! | Keyword         | Viral phrases ("no way", "let's go", ...)    | shock, hype, frustration |
//! | Exclamation     | Emphatic delivery (caps, elongation, "!!")   | reaction, excitement    |
//! | Speech burst    | Sudden dense talking after silence           | burst, reaction         |
//! | Emotional tone  | Charged language (profanity, superlatives)   | emotional, intensity    |
//! | Repetition      | Repeated words/phrases ("go go go", "no no") | urgency, panic          |
//!
//! All analysis is local — no API calls.  The module produces
//! [`SignalSegment`] entries compatible with the pipeline's fusion layer.
//!
//! # Extension point
//!
//! The [`analyze_with_scorer`] function accepts an optional
//! [`LlmScorer`] callback.  When provided, segments that pass the
//! rule-based filters are also scored by the callback (an LLM, a
//! local classifier, etc.) and the two scores are blended.
//! See the [`LlmScorer`] type alias for the contract.

use crate::pipeline::{SignalMetadata, SignalSegment, SignalType};

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Maximum segments returned from the analysis.
const MAX_SEGMENTS: usize = 25;

/// Seconds within which detections are considered duplicates.
const DEDUP_WINDOW_SECS: f64 = 6.0;

/// Minimum score for a detection to survive the final filter.
const MIN_SCORE: f64 = 0.15;

// ═══════════════════════════════════════════════════════════════════
//  Input types (matches what faster-whisper produces)
// ═══════════════════════════════════════════════════════════════════

/// A single timestamped speech segment from the transcriber.
#[derive(Debug, Clone)]
pub struct InputSegment {
    /// Start time in the VOD (seconds).
    pub start: f64,
    /// End time in the VOD (seconds).
    pub end: f64,
    /// Transcribed text for this segment.
    pub text: String,
}

/// A keyword detection from the transcriber's viral-phrase scanner.
#[derive(Debug, Clone)]
pub struct InputKeyword {
    /// The matched phrase (e.g. "no way").
    pub keyword: String,
    /// Start time (seconds).
    pub start: f64,
    /// End time (seconds).
    pub end: f64,
    /// Surrounding speech for context.
    pub context: String,
}

/// Complete transcript to analyze.
#[derive(Debug, Clone)]
pub struct TranscriptInput {
    pub segments: Vec<InputSegment>,
    pub keywords: Vec<InputKeyword>,
    /// Language code (e.g. "en").  Some detectors are English-tuned
    /// and reduce their weight for other languages.
    pub language: String,
}

// ═══════════════════════════════════════════════════════════════════
//  LLM extension point
// ═══════════════════════════════════════════════════════════════════

/// Optional callback for LLM-powered transcript scoring.
///
/// When provided, segments that pass rule-based filtering are also
/// scored by this function.  The returned f64 (0.0–1.0) is blended
/// with the rule-based score at a configurable weight.
///
/// # Contract
///
/// - Receives the raw text and the rule-based score.
/// - Returns a score between 0.0 and 1.0.
/// - May be called many times (once per candidate segment).
/// - Must not block for more than a few seconds per call.
///
/// # Example (future integration)
///
/// ```ignore
/// let scorer: LlmScorer = Box::new(|text, rule_score| {
///     // Call a local model or API
///     let llm_score = my_model.score_highlight(text)?;
///     Ok(llm_score)
/// });
/// let segments = transcript_signal::analyze_with_scorer(&input, Some(&scorer));
/// ```
pub type LlmScorer = Box<dyn Fn(&str, f64) -> Result<f64, String> + Send + Sync>;

/// Weight given to LLM score when blending with rule-based score.
/// `final = rule * (1 - LLM_BLEND) + llm * LLM_BLEND`
const LLM_BLEND_WEIGHT: f64 = 0.55;

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Analyze a transcript and return scored signal segments.
///
/// Runs all five rule-based detectors, deduplicates overlapping
/// detections, normalizes scores to 0.0–1.0, and returns up to
/// [`MAX_SEGMENTS`] results sorted by score descending.
pub fn analyze(input: &TranscriptInput) -> Vec<SignalSegment> {
    analyze_with_scorer(input, None)
}

/// Like [`analyze`], but with an optional LLM scoring callback.
///
/// When `scorer` is `Some`, each candidate's rule-based score is
/// blended with the LLM's score.  If the LLM call fails for a
/// given segment, the rule-based score is used alone.
pub fn analyze_with_scorer(
    input: &TranscriptInput,
    scorer: Option<&LlmScorer>,
) -> Vec<SignalSegment> {
    if input.segments.is_empty() && input.keywords.is_empty() {
        return Vec::new();
    }

    let is_english = input.language.starts_with("en");

    // ── Run all detectors ──
    let mut hits: Vec<ScoredHit> = Vec::new();
    hits.extend(detect_keywords(&input.keywords, is_english));
    hits.extend(detect_exclamations(&input.segments, is_english));
    hits.extend(detect_speech_bursts(&input.segments));
    hits.extend(detect_emotional_tone(&input.segments, is_english));
    hits.extend(detect_repetition(&input.segments));

    // ── Deduplicate overlapping detections (keep higher scorer) ──
    dedup_hits(&mut hits);

    // ── Apply LLM scorer if provided ──
    if let Some(score_fn) = scorer {
        for hit in &mut hits {
            if let Ok(llm_score) = score_fn(&hit.text, hit.score) {
                let clamped = llm_score.clamp(0.0, 1.0);
                hit.score = hit.score * (1.0 - LLM_BLEND_WEIGHT) + clamped * LLM_BLEND_WEIGHT;
            }
            // On error, keep the rule-based score unchanged.
        }
    }

    // ── Normalize: scale so the best hit reaches ~0.95 ──
    let max_score = hits.iter().map(|h| h.score).fold(0.0_f64, f64::max);
    if max_score > 0.0 {
        for hit in &mut hits {
            hit.score = (hit.score / max_score * 0.90 + 0.05).clamp(0.0, 1.0);
        }
    }

    // ── Filter, sort, convert ──
    hits.retain(|h| h.score >= MIN_SCORE);
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(MAX_SEGMENTS);

    hits.into_iter().map(|h| h.into_segment()).collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Internal scored hit (intermediate before conversion)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct ScoredHit {
    start: f64,
    end: f64,
    score: f64,
    text: String,
    keyword: Option<String>,
    tags: Vec<String>,
    language: Option<String>,
}

impl ScoredHit {
    fn into_segment(self) -> SignalSegment {
        SignalSegment {
            signal_type: SignalType::Transcript,
            start_time: self.start,
            end_time: self.end,
            score: self.score,
            tags: self.tags,
            metadata: Some(SignalMetadata::Transcript {
                text: self.text,
                keyword: self.keyword,
                language: self.language,
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 1 — Viral keyword matching
// ═══════════════════════════════════════════════════════════════════
//
//  Uses the pre-detected keywords from the Python transcriber.
//  Scores are assigned by tier:
//
//    Tier 1 (0.90): extreme reactions     — "no way", "oh my god", "what the f*"
//    Tier 2 (0.75): strong hype           — "let's go", "clutch", "destroyed"
//    Tier 3 (0.60): moderate excitement   — "dude", "bro", "insane"
//    Tier 4 (0.45): mild triggers         — "gg", "ez", everything else
//
//  Tags are assigned by emotional category of the keyword.

/// Tier 1 — extreme shock / disbelief.
const TIER1_KEYWORDS: &[&str] = &[
    "no way", "oh my god", "what the fuck", "what the hell", "what the",
    "holy shit", "holy crap", "are you kidding", "impossible", "unbelievable",
];
/// Tier 2 — strong hype / celebration.
const TIER2_KEYWORDS: &[&str] = &[
    "let's go", "lets go", "clutch", "destroyed", "legendary", "epic",
    "watch this", "clip it", "clip that", "yes sir",
];
/// Tier 3 — moderate excitement / emphasis.
const TIER3_KEYWORDS: &[&str] = &[
    "insane", "crazy", "massive", "huge", "oh no", "oh god", "oh snap",
    "yooo", "yoo", "bruh", "dude", "bro",
];

fn detect_keywords(keywords: &[InputKeyword], _is_english: bool) -> Vec<ScoredHit> {
    keywords
        .iter()
        .map(|kw| {
            let lower = kw.keyword.to_lowercase();

            // Score by tier
            let score = if TIER1_KEYWORDS.iter().any(|t| lower.contains(t)) {
                0.90
            } else if TIER2_KEYWORDS.iter().any(|t| lower.contains(t)) {
                0.75
            } else if TIER3_KEYWORDS.iter().any(|t| lower.contains(t)) {
                0.60
            } else {
                0.45
            };

            // Tag by emotional category
            let mut tags = vec!["keyword".to_string()];
            if lower.contains("no") || lower.contains("what") || lower.contains("oh")
                || lower.contains("impossible") || lower.contains("unbelievable")
            {
                tags.push("shock".to_string());
            }
            if lower.contains("go") || lower.contains("yes") || lower.contains("clutch")
                || lower.contains("epic") || lower.contains("legendary")
            {
                tags.push("hype".to_string());
            }
            if lower.contains("rage") || lower.contains("done") || lower.contains("dead")
                || lower.contains("quit")
            {
                tags.push("frustration".to_string());
            }
            if lower.contains("run") || lower.contains("help") || lower.contains("behind") {
                tags.push("panic".to_string());
            }

            ScoredHit {
                start: kw.start,
                end: kw.end,
                score,
                text: kw.context.clone(),
                keyword: Some(kw.keyword.clone()),
                tags,
                language: None,
            }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 2 — Exclamation / emphatic delivery
// ═══════════════════════════════════════════════════════════════════
//
//  Scoring heuristics (additive, capped at 1.0):
//    +0.30  if text contains "!!" or "!!!"
//    +0.25  if >40% of alphabetic chars are uppercase (SHOUTING)
//    +0.20  if word has 3+ repeated chars ("noooo", "yesss")
//    +0.15  if text ends with "!" and is short (< 6 words)
//
//  These fire on delivery style, not content — a whispered "no way"
//  and a SCREAMED "NO WAY!!!" should score differently.

fn detect_exclamations(segments: &[InputSegment], is_english: bool) -> Vec<ScoredHit> {
    if !is_english {
        // Exclamation markers are somewhat language-universal, but
        // the uppercase heuristic is Latin-script-specific.
        // Still run, but halve the weight.
    }

    let mut hits = Vec::new();
    let lang_factor = if is_english { 1.0 } else { 0.5 };

    for seg in segments {
        let text = &seg.text;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut score = 0.0_f64;
        let mut tags: Vec<String> = Vec::new();

        // Multiple exclamation marks — strong emphasis
        if trimmed.contains("!!!") {
            score += 0.35;
            tags.push("excitement".to_string());
        } else if trimmed.contains("!!") {
            score += 0.25;
            tags.push("excitement".to_string());
        }

        // Uppercase ratio — SHOUTING
        let alpha_chars: Vec<char> = trimmed.chars().filter(|c| c.is_alphabetic()).collect();
        if alpha_chars.len() >= 3 {
            let upper_ratio =
                alpha_chars.iter().filter(|c| c.is_uppercase()).count() as f64
                    / alpha_chars.len() as f64;
            if upper_ratio > 0.6 {
                score += 0.30;
                tags.push("shouting".to_string());
            } else if upper_ratio > 0.4 {
                score += 0.15;
            }
        }

        // Elongated words — "noooo", "yesss", "goooo"
        if has_elongation(trimmed) {
            score += 0.20;
            tags.push("elongation".to_string());
        }

        // Short exclamatory phrase — "Let's go!" (high energy density)
        let word_count = trimmed.split_whitespace().count();
        if trimmed.ends_with('!') && word_count <= 5 {
            score += 0.15;
        }

        score *= lang_factor;

        if score >= 0.20 {
            tags.insert(0, "exclamation".to_string());
            hits.push(ScoredHit {
                start: seg.start,
                end: seg.end,
                score: score.min(1.0),
                text: trimmed.to_string(),
                keyword: None,
                tags,
                language: None,
            });
        }
    }

    hits
}

/// Check if text contains a word with 3+ consecutive identical characters.
fn has_elongation(text: &str) -> bool {
    text.split_whitespace().any(|word| {
        let chars: Vec<char> = word.to_lowercase().chars().collect();
        chars.windows(3).any(|w| w[0] == w[1] && w[1] == w[2])
    })
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 3 — Speech bursts (silence → sudden dense speech)
// ═══════════════════════════════════════════════════════════════════
//
//  Finds moments where speech density jumps suddenly — the streamer
//  was quiet, then starts talking rapidly.  Classic "something just
//  happened" signal.
//
//  Algorithm:
//    1. Build a density timeline: words-per-second per segment.
//    2. For each segment, compare its density to the previous 15s.
//    3. Score = (current_density - before_density) / max_density.
//    4. Only keep positive jumps (quiet → talkative).

/// Seconds to look back for the "quiet before" baseline.
const BURST_LOOKBACK_SECS: f64 = 15.0;
/// Minimum words-per-second to qualify as "dense" speech.
const BURST_MIN_WPS: f64 = 2.5;

fn detect_speech_bursts(segments: &[InputSegment]) -> Vec<ScoredHit> {
    if segments.len() < 3 {
        return Vec::new();
    }

    // Compute words-per-second for each segment
    let wps: Vec<f64> = segments
        .iter()
        .map(|s| {
            let dur = (s.end - s.start).max(0.1);
            s.text.split_whitespace().count() as f64 / dur
        })
        .collect();

    let max_wps = wps.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let mut hits = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        let current_wps = wps[i];
        if current_wps < BURST_MIN_WPS {
            continue;
        }

        // Average wps in the lookback window
        let lookback_start = (seg.start - BURST_LOOKBACK_SECS).max(0.0);
        let before_segments: Vec<f64> = segments[..i]
            .iter()
            .zip(&wps[..i])
            .filter(|(s, _)| s.end > lookback_start && s.start < seg.start)
            .map(|(_, &w)| w)
            .collect();

        let before_avg = if before_segments.is_empty() {
            0.0
        } else {
            before_segments.iter().sum::<f64>() / before_segments.len() as f64
        };

        let delta = current_wps - before_avg;
        if delta <= 0.5 {
            continue; // Not a meaningful jump
        }

        let score = (delta / max_wps).min(1.0) * 0.85;

        if score >= 0.15 {
            hits.push(ScoredHit {
                start: seg.start,
                end: seg.end,
                score,
                text: seg.text.trim().to_string(),
                keyword: None,
                tags: vec!["burst".to_string(), "reaction".to_string()],
                language: None,
            });
        }
    }

    hits
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 4 — Emotional tone (charged language)
// ═══════════════════════════════════════════════════════════════════
//
//  Looks for words and patterns that indicate emotional intensity,
//  regardless of whether they match "viral keyword" lists:
//
//    - Profanity / expletives  →  high arousal
//    - Superlatives            →  emphasis
//    - Interjections           →  surprise / reaction
//    - Question words in short phrases  →  disbelief
//
//  Score = (matched_weight / max_possible) * 0.85

const PROFANITY: &[&str] = &[
    "fuck", "shit", "damn", "hell", "ass", "crap", "wtf",
    "goddamn", "bullshit", "motherfucker",
];
const SUPERLATIVES: &[&str] = &[
    "best", "worst", "most", "ever", "never", "always",
    "insane", "incredible", "ridiculous", "absurd",
    "greatest", "craziest", "biggest", "smallest",
];
const INTERJECTIONS: &[&str] = &[
    "oh", "wow", "whoa", "woah", "damn", "ugh", "oof",
    "yikes", "geez", "jeez", "huh", "wha", "yo",
];

fn detect_emotional_tone(segments: &[InputSegment], is_english: bool) -> Vec<ScoredHit> {
    if !is_english {
        return Vec::new(); // These word lists are English-only
    }

    let mut hits = Vec::new();

    for seg in segments {
        let lower = seg.text.to_lowercase();
        // Strip punctuation so "How?!" matches "how"
        let clean_words: Vec<String> = lower
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| !w.is_empty())
            .collect();
        if clean_words.is_empty() {
            continue;
        }

        let mut weight = 0.0_f64;
        let mut tags: Vec<String> = Vec::new();

        // Profanity — high emotional arousal
        let profanity_count = clean_words
            .iter()
            .filter(|w| PROFANITY.iter().any(|p| w.contains(p)))
            .count();
        if profanity_count > 0 {
            weight += 0.35 + (profanity_count as f64 - 1.0).min(2.0) * 0.10;
            tags.push("emotional".to_string());
        }

        // Superlatives — emphasis / extremes
        let superlative_count = clean_words
            .iter()
            .filter(|w| SUPERLATIVES.iter().any(|s| w.contains(s)))
            .count();
        if superlative_count > 0 {
            weight += 0.20 + (superlative_count as f64 - 1.0).min(2.0) * 0.08;
            tags.push("intensity".to_string());
        }

        // Interjections — surprise / reaction
        let interjection_count = clean_words
            .iter()
            .filter(|w| INTERJECTIONS.iter().any(|ij| w.as_str() == *ij))
            .count();
        if interjection_count > 0 {
            weight += 0.15 + (interjection_count as f64 - 1.0).min(2.0) * 0.08;
            tags.push("reaction".to_string());
        }

        // Short disbelief questions — "How?!", "Why?!", "What?!"
        if clean_words.len() <= 3
            && (lower.contains('?') || lower.contains('!'))
            && clean_words.iter().any(|w| ["how", "why", "what", "where", "who"].contains(&w.as_str()))
        {
            weight += 0.25;
            tags.push("disbelief".to_string());
        }

        if weight >= 0.25 {
            let score = (weight / 1.0).min(1.0) * 0.85;
            tags.insert(0, "tone".to_string());
            hits.push(ScoredHit {
                start: seg.start,
                end: seg.end,
                score,
                text: seg.text.trim().to_string(),
                keyword: None,
                tags,
                language: Some("en".to_string()),
            });
        }
    }

    hits
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 5 — Repetition (urgency / emphasis)
// ═══════════════════════════════════════════════════════════════════
//
//  Detects repeated words within a single segment:
//    "go go go"   → urgency
//    "no no no"   → distress / disbelief
//    "yes yes"    → celebration
//
//  Scoring:
//    2 repeats of same word → 0.35
//    3+ repeats             → 0.55
//    Multiple repeated words → +0.15 each

fn detect_repetition(segments: &[InputSegment]) -> Vec<ScoredHit> {
    let mut hits = Vec::new();

    for seg in segments {
        let words: Vec<String> = seg
            .text
            .to_lowercase()
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| w.len() >= 2) // Skip single-char noise
            .collect();

        if words.len() < 2 {
            continue;
        }

        // Count consecutive runs of the same word
        let mut max_run = 1_usize;
        let mut distinct_repeats = 0_usize;
        let mut current_run = 1_usize;

        for i in 1..words.len() {
            if words[i] == words[i - 1] {
                current_run += 1;
            } else {
                if current_run >= 2 {
                    distinct_repeats += 1;
                    max_run = max_run.max(current_run);
                }
                current_run = 1;
            }
        }
        if current_run >= 2 {
            distinct_repeats += 1;
            max_run = max_run.max(current_run);
        }

        if distinct_repeats == 0 {
            continue;
        }

        let mut score = if max_run >= 3 { 0.55 } else { 0.35 };
        score += (distinct_repeats as f64 - 1.0).min(2.0) * 0.15;
        let score = score.min(1.0);

        let mut tags = vec!["repetition".to_string()];
        // Classify the emotional direction of the repetition
        let repeated_words: Vec<&str> = words.windows(2)
            .filter(|w| w[0] == w[1])
            .map(|w| w[0].as_str())
            .collect();
        if repeated_words.iter().any(|w| ["go", "run", "move", "push"].contains(w)) {
            tags.push("urgency".to_string());
        }
        if repeated_words.iter().any(|w| ["no", "stop", "wait", "why"].contains(w)) {
            tags.push("panic".to_string());
        }
        if repeated_words.iter().any(|w| ["yes", "yeah", "yep"].contains(w)) {
            tags.push("celebration".to_string());
        }

        hits.push(ScoredHit {
            start: seg.start,
            end: seg.end,
            score,
            text: seg.text.trim().to_string(),
            keyword: None,
            tags,
            language: None,
        });
    }

    hits
}

// ═══════════════════════════════════════════════════════════════════
//  Deduplication
// ═══════════════════════════════════════════════════════════════════

/// Remove lower-scoring detections that overlap in time.
fn dedup_hits(hits: &mut Vec<ScoredHit>) {
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep = Vec::with_capacity(hits.len());
    'outer: for hit in hits.drain(..) {
        for kept in &keep {
            let kept: &ScoredHit = kept;
            if (hit.start - kept.start).abs() < DEDUP_WINDOW_SECS {
                continue 'outer; // Skip — a higher-scoring hit already covers this time
            }
        }
        keep.push(hit);
    }
    *hits = keep;
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> InputSegment {
        InputSegment { start, end, text: text.to_string() }
    }

    fn kw(keyword: &str, start: f64, end: f64, context: &str) -> InputKeyword {
        InputKeyword {
            keyword: keyword.to_string(),
            start,
            end,
            context: context.to_string(),
        }
    }

    fn input(segments: Vec<InputSegment>, keywords: Vec<InputKeyword>) -> TranscriptInput {
        TranscriptInput { segments, keywords, language: "en".to_string() }
    }

    // ── Empty input ──

    #[test]
    fn empty_transcript_returns_empty() {
        let result = analyze(&input(vec![], vec![]));
        assert!(result.is_empty());
    }

    // ── Keyword detector ──

    #[test]
    fn tier1_keyword_scores_highest() {
        let inp = input(vec![], vec![
            kw("no way", 10.0, 11.0, "No way that just happened"),
            kw("gg", 30.0, 31.0, "GG boys"),
        ]);
        let result = analyze(&inp);
        assert!(result.len() == 2);
        // "no way" (tier 1) should score higher than "gg" (tier 4)
        assert!(result[0].score > result[1].score);
    }

    #[test]
    fn keyword_gets_shock_tag() {
        let inp = input(vec![], vec![kw("what the", 5.0, 6.0, "What the heck")]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"shock".to_string()));
    }

    // ── Exclamation detector ──

    #[test]
    fn all_caps_scores_as_shouting() {
        let inp = input(vec![seg(10.0, 12.0, "OH MY GOD NO WAY")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"shouting".to_string()));
    }

    #[test]
    fn elongated_word_detected() {
        let inp = input(vec![seg(5.0, 7.0, "noooooo why")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"elongation".to_string()));
    }

    #[test]
    fn triple_exclamation_detected() {
        let inp = input(vec![seg(20.0, 22.0, "Let's go!!!")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"excitement".to_string()));
    }

    // ── Speech burst detector ──

    #[test]
    fn burst_after_silence_detected() {
        let segments = vec![
            seg(0.0, 2.0, "yeah"),                // 0.5 wps — quiet
            seg(5.0, 7.0, "ok"),                   // 0.5 wps — quiet
            seg(20.0, 22.0, "yeah"),               // 0.5 wps — quiet
            // Sudden dense speech after gap
            seg(40.0, 42.0, "oh my god what just happened that was insane dude I can't believe it"),
        ];
        let result = analyze(&input(segments, vec![]));
        assert!(!result.is_empty());
        assert!(result.iter().any(|s| s.tags.contains(&"burst".to_string())));
    }

    // ── Emotional tone detector ──

    #[test]
    fn profanity_scores_as_emotional() {
        let inp = input(vec![seg(10.0, 13.0, "what the fuck was that shit")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"emotional".to_string()));
    }

    #[test]
    fn short_disbelief_question_detected() {
        let inp = input(vec![seg(30.0, 31.0, "How?!")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"disbelief".to_string()));
    }

    #[test]
    fn non_english_skips_emotional_tone() {
        let mut inp = input(vec![seg(10.0, 13.0, "what the fuck was that")], vec![]);
        inp.language = "ja".to_string();
        let result = analyze(&inp);
        // Emotional tone detector is English-only; should not fire
        assert!(result.iter().all(|s| !s.tags.contains(&"tone".to_string())));
    }

    // ── Repetition detector ──

    #[test]
    fn triple_repeat_detected() {
        let inp = input(vec![seg(10.0, 12.0, "go go go push push")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"repetition".to_string()));
        assert!(result[0].tags.contains(&"urgency".to_string()));
    }

    #[test]
    fn no_no_no_gets_panic_tag() {
        let inp = input(vec![seg(10.0, 12.0, "no no no stop stop")], vec![]);
        let result = analyze(&inp);
        assert!(!result.is_empty());
        assert!(result[0].tags.contains(&"panic".to_string()));
    }

    // ── Full pipeline ──

    #[test]
    fn all_scores_normalized_0_to_1() {
        let inp = input(
            vec![
                seg(0.0, 3.0, "oh my god NO WAY!!!"),
                seg(10.0, 12.0, "yeah whatever"),
                seg(20.0, 23.0, "GO GO GO PUSH!!!"),
                seg(40.0, 42.0, "that was insane holy shit"),
            ],
            vec![
                kw("no way", 1.0, 2.0, "oh my god NO WAY"),
                kw("insane", 40.0, 41.0, "that was insane"),
            ],
        );
        let result = analyze(&inp);
        assert!(!result.is_empty());
        for seg in &result {
            assert!(seg.score >= 0.0 && seg.score <= 1.0,
                "score {} out of range", seg.score);
        }
    }

    #[test]
    fn results_sorted_by_score_descending() {
        let inp = input(
            vec![
                seg(0.0, 2.0, "oh"),
                seg(10.0, 12.0, "NOOOO WHAT THE FUCK!!!"),
                seg(30.0, 32.0, "nice"),
            ],
            vec![kw("what the", 10.0, 11.0, "WHAT THE FUCK")],
        );
        let result = analyze(&inp);
        for pair in result.windows(2) {
            assert!(pair[0].score >= pair[1].score,
                "not sorted: {} before {}", pair[0].score, pair[1].score);
        }
    }

    #[test]
    fn overlapping_detections_deduplicated() {
        // Same timestamp: keyword + exclamation + tone should merge into one
        let inp = input(
            vec![seg(10.0, 12.0, "HOLY SHIT NO WAY!!!")],
            vec![kw("no way", 10.0, 12.0, "HOLY SHIT NO WAY!!!")],
        );
        let result = analyze(&inp);
        // Multiple detectors fire on the same segment but dedup keeps one
        let at_ten: Vec<_> = result.iter().filter(|s| s.start_time < 13.0).collect();
        assert_eq!(at_ten.len(), 1, "overlapping detections should be deduplicated");
    }

    #[test]
    fn metadata_is_populated() {
        let inp = input(
            vec![],
            vec![kw("clutch", 20.0, 21.0, "That was so clutch")],
        );
        let result = analyze(&inp);
        assert!(!result.is_empty());
        match &result[0].metadata {
            Some(SignalMetadata::Transcript { text, keyword, .. }) => {
                assert!(text.contains("clutch"));
                assert_eq!(keyword.as_deref(), Some("clutch"));
            }
            other => panic!("Expected Transcript metadata, got {:?}", other),
        }
    }

    #[test]
    fn capped_at_max_segments() {
        let segments: Vec<InputSegment> = (0..100)
            .map(|i| seg(i as f64 * 10.0, i as f64 * 10.0 + 2.0, "HOLY SHIT!!!"))
            .collect();
        let result = analyze(&input(segments, vec![]));
        assert!(result.len() <= MAX_SEGMENTS);
    }

    // ── LLM scorer extension ──

    #[test]
    fn llm_scorer_blends_with_rule_score() {
        let inp = input(vec![], vec![kw("no way", 10.0, 11.0, "No way!")]);
        let scorer: LlmScorer = Box::new(|_text, _rule_score| Ok(1.0));
        let with_llm = analyze_with_scorer(&inp, Some(&scorer));
        let without = analyze(&inp);

        assert!(!with_llm.is_empty());
        assert!(!without.is_empty());
        // LLM returning 1.0 should push the score higher
        assert!(with_llm[0].score >= without[0].score);
    }

    #[test]
    fn llm_scorer_failure_keeps_rule_score() {
        let inp = input(vec![], vec![kw("clutch", 10.0, 11.0, "Clutch play")]);
        let scorer: LlmScorer = Box::new(|_text, _rule_score| Err("model offline".into()));
        let with_llm = analyze_with_scorer(&inp, Some(&scorer));
        let without = analyze(&inp);

        assert!(!with_llm.is_empty());
        // Should produce approximately the same score since LLM failed
        assert!((with_llm[0].score - without[0].score).abs() < 0.01);
    }
}
