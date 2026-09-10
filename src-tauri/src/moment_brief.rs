//! Shared, local-first evidence model for detected moments and publish copy.
//!
//! A `MomentBrief` is the single factual handoff between detection, local copy,
//! and optional BYOK rewriting. It contains only evidence already available to
//! ClipGoblin; it never calls an external service.

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const BRIEF_VERSION: u32 = 1;
const MAX_EVIDENCE_TEXT: usize = 240;

#[derive(Debug, Clone, Default)]
pub struct MomentScores {
    pub hook: f64,
    pub emotion: f64,
    pub payoff: f64,
    pub alignment: f64,
    pub context: f64,
    pub confidence: f64,
}

pub struct MomentEvidence<'a> {
    pub transcript: Option<&'a str>,
    pub detector_summary: Option<&'a str>,
    pub detector_title: Option<&'a str>,
    pub payoff_summary: Option<&'a str>,
    pub outcome_label: Option<&'a str>,
    pub tags: &'a [String],
    pub game: Option<&'a str>,
    pub stream_style: Option<&'a str>,
    pub signal_sources: &'a [String],
    pub scores: MomentScores,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MomentSignalEvidence {
    pub source: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MomentQuote {
    pub text: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MomentStyle {
    Action,
    Cozy,
    Story,
    Talking,
    Mixed,
}

impl MomentStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Cozy => "cozy",
            Self::Story => "story",
            Self::Talking => "talking",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MomentBrief {
    pub version: u32,
    pub signature: String,
    pub core_event: String,
    pub setup: Option<String>,
    pub pivot: Option<String>,
    pub payoff: Option<String>,
    pub quote_candidates: Vec<MomentQuote>,
    pub tags: Vec<String>,
    pub game_context: Option<String>,
    pub stream_style: MomentStyle,
    pub signal_evidence: Vec<MomentSignalEvidence>,
    pub confidence: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CopySuggestion {
    pub text: String,
    pub strategy: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CopyRejection {
    Empty,
    Generic,
    Ungrounded,
    UnsupportedClaim(String),
    RepeatsTitle,
}

impl MomentBrief {
    pub fn build(evidence: MomentEvidence<'_>) -> Self {
        let transcript = clean_transcript(evidence.transcript.unwrap_or_default());
        let clauses = transcript_clauses(&transcript);
        let tags = normalize_tags(evidence.tags);
        let stream_style =
            classify_style(evidence.stream_style, &tags, &transcript, &evidence.scores);
        let quote_candidates = build_quote_candidates(&clauses);

        let setup = clauses.first().cloned().filter(|text| is_specific(text));
        let pivot = clauses
            .iter()
            .find(|text| contains_pivot_marker(text))
            .cloned()
            .or_else(|| (clauses.len() >= 3).then(|| clauses[clauses.len() / 2].clone()))
            .filter(|text| setup.as_deref() != Some(text.as_str()));
        let payoff = clauses
            .last()
            .cloned()
            .filter(|text| is_specific(text))
            .filter(|text| setup.as_deref() != Some(text.as_str()));

        let core_event = choose_core_event(&evidence, &tags, &clauses, payoff.as_deref());
        let signal_evidence = build_signal_evidence(evidence.signal_sources, &evidence.scores);
        let confidence = brief_confidence(&evidence.scores, &tags, &clauses, &signal_evidence);
        let game_context = clean_optional(evidence.game);

        let mut brief = Self {
            version: BRIEF_VERSION,
            signature: String::new(),
            core_event,
            setup,
            pivot,
            payoff,
            quote_candidates,
            tags,
            game_context,
            stream_style,
            signal_evidence,
            confidence,
        };
        brief.signature = brief.compute_signature();
        brief
    }

    pub fn prompt_context(&self) -> String {
        let mut lines = Vec::new();
        if !self.core_event.trim().is_empty() {
            lines.push(format!("VERIFIED EVENT: {}", self.core_event));
        }
        if let Some(setup) = self.setup.as_deref() {
            lines.push(format!("SETUP: {setup}"));
        }
        if let Some(pivot) = self.pivot.as_deref() {
            lines.push(format!("TURN: {pivot}"));
        }
        if let Some(payoff) = self.payoff.as_deref() {
            lines.push(format!("PAYOFF: {payoff}"));
        }
        if let Some(quote) = self.quote_candidates.first() {
            lines.push(format!("VERIFIED QUOTE: \"{}\"", quote.text));
        }
        if !self.tags.is_empty() {
            lines.push(format!("EVIDENCE TAGS: {}", self.tags.join(", ")));
        }
        if let Some(game) = self.game_context.as_deref() {
            lines.push(format!("GAME CONTEXT: {game}"));
        }
        lines.push(format!(
            "STYLE: {}; BRIEF CONFIDENCE: {:.2}",
            self.stream_style.as_str(),
            self.confidence
        ));
        lines.join("\n")
    }

    pub fn title_suggestions(
        &self,
        seed: u32,
        preferences: &HashMap<String, f64>,
    ) -> Vec<CopySuggestion> {
        let mut candidates = Vec::new();
        let mut add = |text: String, strategy: &str| {
            let text = title_case_copy(&limit_at_word(&text, 60));
            if self.validate_title(&text).is_ok()
                && !candidates.iter().any(|item: &CopySuggestion| {
                    normalize_copy(&item.text) == normalize_copy(&text)
                })
            {
                candidates.push(CopySuggestion {
                    text,
                    strategy: strategy.to_string(),
                });
            }
        };

        if let (Some(pivot), Some(payoff)) = (self.pivot.as_deref(), self.payoff.as_deref()) {
            if normalize_copy(pivot) != normalize_copy(payoff) {
                add(
                    format!("{} - {}", concise_beat(pivot), concise_beat(payoff)),
                    "pivot_payoff",
                );
            }
        }
        if let Some(payoff) = self.payoff.as_deref() {
            add(concise_beat(payoff), "payoff");
        }
        if let Some(quote) = self.quote_candidates.first() {
            if quote.text.chars().count() <= 52 {
                add(format!("\"{}\"", quote.text), "quote");
            }
        }
        if !self.core_event.trim().is_empty() {
            add(self.core_event.clone(), "core_event");
            if let Some(game) = self.game_context.as_deref() {
                if !self
                    .core_event
                    .to_lowercase()
                    .contains(&game.to_lowercase())
                {
                    add(format!("{} in {}", self.core_event, game), "event_game");
                }
            }
        }

        rank_suggestions(candidates, seed, preferences)
    }

    pub fn caption_suggestions(
        &self,
        mode: &str,
        title: &str,
        seed: u32,
        preferences: &HashMap<String, f64>,
    ) -> Vec<CopySuggestion> {
        let event = sentence_case(&self.core_event);
        let setup = self.setup.as_deref().map(sentence_case);
        let pivot = self.pivot.as_deref().map(sentence_case);
        let payoff = self.payoff.as_deref().map(sentence_case);
        let quote = self.quote_candidates.first().map(|item| item.text.as_str());
        let mut candidates = Vec::new();
        let mut add = |text: String, strategy: &str| {
            let text = limit_at_word(&normalize_spacing(&text), 280);
            if self.validate_description(&text, title).is_ok()
                && !candidates.iter().any(|item: &CopySuggestion| {
                    normalize_copy(&item.text) == normalize_copy(&text)
                })
            {
                candidates.push(CopySuggestion {
                    text,
                    strategy: format!("{mode}:{strategy}"),
                });
            }
        };

        match mode {
            "direct_quote" => {
                if let Some(quote) = quote {
                    add(format!("\"{}\" {}", quote, event), "quote_event");
                }
            }
            "observation" | "clean" => {
                if let (Some(setup), Some(payoff)) = (setup.as_deref(), payoff.as_deref()) {
                    add(format!("{} {}", setup, payoff), "setup_payoff");
                }
            }
            "funny" | "internal_thought" => {
                if let (Some(quote), Some(payoff)) = (quote, payoff.as_deref()) {
                    add(format!("\"{}\" {}", quote, payoff), "quote_payoff");
                }
                if let (Some(setup), Some(pivot)) = (setup.as_deref(), pivot.as_deref()) {
                    add(format!("{} {}", setup, pivot), "setup_turn");
                }
            }
            "hype" | "punchy" => {
                if let Some(payoff) = payoff.as_deref() {
                    add(format!("{} {}", event, payoff), "event_payoff");
                }
            }
            "search" => {
                if let Some(game) = self.game_context.as_deref() {
                    add(format!("{} {}", game, event), "game_event");
                }
            }
            "blame" => {
                if let (Some(pivot), Some(payoff)) = (pivot.as_deref(), payoff.as_deref()) {
                    add(format!("{} {}", pivot, payoff), "cause_payoff");
                }
            }
            "minimal" => {
                if let Some(payoff) = payoff.as_deref() {
                    add(payoff.to_string(), "payoff");
                }
            }
            _ => {}
        }

        if let (Some(pivot), Some(payoff)) = (pivot.as_deref(), payoff.as_deref()) {
            add(format!("{} {}", pivot, payoff), "turn_payoff");
        }
        if let Some(quote) = quote {
            add(format!("\"{}\" {}", quote, event), "quote_event");
        }
        if let Some(payoff) = payoff.as_deref() {
            add(format!("{} {}", event, payoff), "event_payoff");
        }
        if let Some(setup) = setup.as_deref() {
            add(format!("{} {}", setup, event), "setup_event");
        }

        rank_suggestions(candidates, seed, preferences)
    }

    pub fn validate_title(&self, text: &str) -> Result<(), CopyRejection> {
        self.validate_grounding(text)
    }

    pub fn validate_description(&self, text: &str, title: &str) -> Result<(), CopyRejection> {
        self.validate_grounding(text)?;
        if descriptions_repeat(text, title, self) {
            return Err(CopyRejection::RepeatsTitle);
        }
        Ok(())
    }

    fn validate_grounding(&self, text: &str) -> Result<(), CopyRejection> {
        let cleaned = normalize_spacing(text);
        if cleaned.is_empty() {
            return Err(CopyRejection::Empty);
        }
        if is_generic_copy(&cleaned) {
            return Err(CopyRejection::Generic);
        }

        let evidence = self.evidence_tokens();
        let output = content_tokens(&cleaned);
        for claim in unsupported_claims(&output, &evidence) {
            return Err(CopyRejection::UnsupportedClaim(claim));
        }

        let has_quote = self.quote_candidates.iter().any(|quote| {
            quote.text.split_whitespace().count() >= 2
                && normalize_copy(&cleaned).contains(&normalize_copy(&quote.text))
        });
        let shared = output.intersection(&evidence).count();
        if !has_quote && shared == 0 {
            return Err(CopyRejection::Ungrounded);
        }
        Ok(())
    }

    fn evidence_tokens(&self) -> HashSet<String> {
        let mut all = content_tokens(&self.core_event);
        for text in [
            self.setup.as_deref(),
            self.pivot.as_deref(),
            self.payoff.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            all.extend(content_tokens(text));
        }
        for quote in &self.quote_candidates {
            all.extend(content_tokens(&quote.text));
        }
        for tag in &self.tags {
            all.extend(content_tokens(tag));
        }
        all
    }

    fn compute_signature(&self) -> String {
        let payload = serde_json::json!({
            "version": self.version,
            "core_event": self.core_event,
            "setup": self.setup,
            "pivot": self.pivot,
            "payoff": self.payoff,
            "quotes": self.quote_candidates,
            "tags": self.tags,
            "game": self.game_context,
            "style": self.stream_style,
            "signals": self.signal_evidence,
        });
        let mut hasher = Sha256::new();
        hasher.update(payload.to_string().as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

fn choose_core_event(
    evidence: &MomentEvidence<'_>,
    tags: &[String],
    clauses: &[String],
    payoff: Option<&str>,
) -> String {
    for candidate in [
        evidence.detector_summary,
        evidence.payoff_summary,
        evidence.outcome_label,
        payoff,
        clauses.last().map(String::as_str),
        evidence.detector_title,
    ]
    .into_iter()
    .flatten()
    {
        let cleaned = trim_evidence(candidate);
        if is_specific(&cleaned) {
            return cleaned;
        }
    }

    let meaningful: Vec<&str> = tags
        .iter()
        .map(String::as_str)
        .filter(|tag| !is_technical_tag(tag))
        .take(2)
        .collect();
    match meaningful.as_slice() {
        [event, reaction] => format!(
            "the {} turned into {}",
            humanize_tag(event),
            humanize_tag(reaction)
        ),
        [event] => format!("the {} changed the moment", humanize_tag(event)),
        _ => String::new(),
    }
}

fn build_signal_evidence(sources: &[String], scores: &MomentScores) -> Vec<MomentSignalEvidence> {
    sources
        .iter()
        .map(|source| {
            let lower = source.to_lowercase();
            let confidence = if lower.contains("audio") {
                scores.hook.max(scores.emotion)
            } else if lower.contains("transcript") || lower.contains("speech") {
                scores.context.max(scores.alignment)
            } else if lower.contains("chat") || lower.contains("community") {
                scores.payoff.max(scores.confidence)
            } else {
                scores.confidence
            };
            MomentSignalEvidence {
                source: source.clone(),
                confidence: confidence.clamp(0.0, 1.0),
            }
        })
        .collect()
}

fn brief_confidence(
    scores: &MomentScores,
    tags: &[String],
    clauses: &[String],
    signals: &[MomentSignalEvidence],
) -> f64 {
    let scored = [
        scores.hook,
        scores.emotion,
        scores.payoff,
        scores.alignment,
        scores.context,
        scores.confidence,
    ];
    let nonzero: Vec<f64> = scored.into_iter().filter(|score| *score > 0.0).collect();
    let score_mean = if nonzero.is_empty() {
        0.35
    } else {
        nonzero.iter().sum::<f64>() / nonzero.len() as f64
    };
    let evidence_bonus = (tags.len().min(3) as f64 * 0.03)
        + (clauses.len().min(3) as f64 * 0.04)
        + (signals.len().min(3) as f64 * 0.03);
    (score_mean * 0.82 + evidence_bonus).clamp(0.15, 0.99)
}

fn classify_style(
    requested: Option<&str>,
    tags: &[String],
    transcript: &str,
    scores: &MomentScores,
) -> MomentStyle {
    match requested.unwrap_or_default().trim().to_lowercase().as_str() {
        "action" => return MomentStyle::Action,
        "cozy" => return MomentStyle::Cozy,
        "story" => return MomentStyle::Story,
        "talking" => return MomentStyle::Talking,
        "mixed" => return MomentStyle::Mixed,
        _ => {}
    }

    let tag_text = tags.join(" ");
    if contains_any(
        &tag_text,
        &["cozy", "farming", "crafting", "building", "chill"],
    ) {
        return MomentStyle::Cozy;
    }
    if contains_any(
        &tag_text,
        &[
            "fight",
            "chase",
            "kill",
            "escape",
            "clutch",
            "jumpscare",
            "ambush",
        ],
    ) {
        return MomentStyle::Action;
    }
    if contains_any(&tag_text, &["story", "narrative", "reveal", "confession"]) {
        return MomentStyle::Story;
    }
    if transcript.split_whitespace().count() >= 20 && scores.context >= scores.emotion {
        return MomentStyle::Talking;
    }
    MomentStyle::Mixed
}

fn build_quote_candidates(clauses: &[String]) -> Vec<MomentQuote> {
    let mut scored: Vec<(String, f64)> = clauses
        .iter()
        .filter(|clause| {
            let words = clause.split_whitespace().count();
            (2..=18).contains(&words) && is_specific(clause)
        })
        .map(|clause| {
            let lower = clause.to_lowercase();
            let words = clause.split_whitespace().count();
            let expressive = contains_any(
                &lower,
                &[
                    "wait",
                    "what",
                    "why",
                    "no",
                    "yes",
                    "help",
                    "look",
                    "right behind",
                    "goes hard",
                ],
            );
            let score: f64 = 0.45
                + if expressive { 0.25 } else { 0.0 }
                + if (3..=10).contains(&words) { 0.15 } else { 0.0 };
            (limit_at_word(clause, 120), score.min(0.95))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = HashSet::new();
    scored
        .into_iter()
        .filter(|(text, _)| seen.insert(normalize_copy(text)))
        .take(3)
        .map(|(text, confidence)| MomentQuote { text, confidence })
        .collect()
}

fn transcript_clauses(transcript: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    for sentence in transcript.split(['.', '!', '?', '\n']) {
        let sentence = normalize_spacing(sentence);
        if sentence.split_whitespace().count() < 2 {
            continue;
        }
        let mut split_any = false;
        for marker in [" but ", " then ", " until ", " suddenly "] {
            if sentence.to_lowercase().contains(marker) {
                for part in split_case_insensitive(&sentence, marker) {
                    if part.split_whitespace().count() >= 2 {
                        clauses.push(trim_evidence(&part));
                    }
                }
                split_any = true;
                break;
            }
        }
        if !split_any {
            let words: Vec<&str> = sentence.split_whitespace().collect();
            if words.len() <= 18 {
                clauses.push(trim_evidence(&sentence));
            } else {
                clauses.push(words[..12].join(" "));
                clauses.push(words[words.len().saturating_sub(12)..].join(" "));
            }
        }
    }
    let mut seen = HashSet::new();
    clauses
        .into_iter()
        .filter(|clause| seen.insert(normalize_copy(clause)))
        .collect()
}

fn split_case_insensitive(value: &str, marker: &str) -> Vec<String> {
    // ASCII case folding preserves byte offsets into the original UTF-8 text.
    // The markers are English ASCII words, while surrounding speech may not be.
    let lower = value.to_ascii_lowercase();
    let marker = marker.to_ascii_lowercase();
    let mut result = Vec::new();
    let mut start = 0;
    for (index, _) in lower.match_indices(&marker) {
        result.push(value[start..index].trim().to_string());
        // Keep the pivot word on the following clause so setup/turn/payoff
        // extraction can still identify why the moment changed direction.
        start = index + marker.len() - marker.trim_start().len();
    }
    result.push(value[start..].trim().to_string());
    result
}

fn clean_transcript(value: &str) -> String {
    let lines: Vec<&str> = value
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.contains("-->") && !line.chars().all(|ch| ch.is_ascii_digit())
        })
        .collect();
    normalize_spacing(&lines.join(" "))
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty() && seen.insert(tag.clone()))
        .collect()
}

fn rank_suggestions(
    mut suggestions: Vec<CopySuggestion>,
    seed: u32,
    preferences: &HashMap<String, f64>,
) -> Vec<CopySuggestion> {
    suggestions.sort_by(|left, right| {
        let left_score = preferences.get(&left.strategy).copied().unwrap_or(0.0);
        let right_score = preferences.get(&right.strategy).copied().unwrap_or(0.0);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if suggestions.len() > 1 {
        let top_score = preferences
            .get(&suggestions[0].strategy)
            .copied()
            .unwrap_or(0.0);
        let tied = suggestions
            .iter()
            .take_while(|item| {
                (preferences.get(&item.strategy).copied().unwrap_or(0.0) - top_score).abs()
                    < f64::EPSILON
            })
            .count();
        if tied > 1 {
            suggestions[..tied].rotate_left(seed as usize % tied);
        }
    }
    suggestions
}

fn descriptions_repeat(description: &str, title: &str, brief: &MomentBrief) -> bool {
    let description_norm = normalize_copy(description);
    let title_norm = normalize_copy(title);
    if description_norm == title_norm {
        return true;
    }
    let description_tokens = content_tokens(description);
    let title_tokens = content_tokens(title);
    if description_tokens.is_empty() || title_tokens.is_empty() {
        return false;
    }
    let shared = description_tokens.intersection(&title_tokens).count();
    let overlap = shared as f64 / title_tokens.len().max(description_tokens.len()) as f64;
    if overlap < 0.82 {
        return false;
    }
    let evidence = brief.evidence_tokens();
    description_tokens
        .difference(&title_tokens)
        .all(|token| !evidence.contains(token))
}

fn unsupported_claims(output: &HashSet<String>, evidence: &HashSet<String>) -> Vec<String> {
    const CLAIMS: &[&str] = &[
        "ace", "boss", "clutch", "enemy", "escape", "escaped", "headshot", "kill", "killed",
        "killer", "lose", "lost", "survive", "survived", "weapon", "win", "won",
    ];
    CLAIMS
        .iter()
        .filter(|claim| output.contains(**claim) && !evidence.contains(**claim))
        .map(|claim| (*claim).to_string())
        .collect()
}

fn is_generic_copy(text: &str) -> bool {
    let normalized = normalize_copy(text);
    const GENERIC: &[&str] = &[
        "caught on stream",
        "check this out",
        "gaming at its finest",
        "insane clip",
        "just happened",
        "no context needed",
        "stream moment",
        "this happened",
        "this moment hits different",
        "things got wild",
        "watch this",
        "what a moment",
        "you need to see this",
        "you wont believe",
    ];
    if GENERIC.iter().any(|phrase| normalized == *phrase) {
        return true;
    }
    let tokens = content_tokens(text);
    tokens.is_empty() || (tokens.len() <= 3 && tokens.iter().all(|token| is_generic_token(token)))
}

fn is_specific(text: &str) -> bool {
    let cleaned = trim_evidence(text);
    if cleaned.chars().count() < 7 || is_generic_copy(&cleaned) {
        return false;
    }
    content_tokens(&cleaned).len() >= 2
}

fn is_technical_tag(tag: &str) -> bool {
    contains_any(
        tag,
        &[
            "audio-spike",
            "auto",
            "community-clip",
            "creator-approved",
            "transcript",
        ],
    )
}

fn content_tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'', " ")
        .split_whitespace()
        .map(|token| token.trim_matches('\'').trim_end_matches("'s").to_string())
        .filter(|token| token.len() >= 3 && !is_stop_word(token) && !is_generic_token(token))
        .collect()
}

fn is_stop_word(token: &str) -> bool {
    matches!(
        token,
        "and"
            | "are"
            | "but"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "into"
            | "its"
            | "just"
            | "not"
            | "that"
            | "the"
            | "their"
            | "then"
            | "this"
            | "was"
            | "were"
            | "what"
            | "when"
            | "with"
            | "you"
            | "your"
    )
}

fn is_generic_token(token: &str) -> bool {
    matches!(
        token,
        "clip"
            | "content"
            | "crazy"
            | "epic"
            | "game"
            | "gaming"
            | "insane"
            | "literally"
            | "moment"
            | "stream"
            | "thing"
            | "things"
            | "timing"
            | "wild"
    )
}

fn contains_pivot_marker(text: &str) -> bool {
    let lower = format!(" {} ", text.to_lowercase());
    [" but ", " then ", " suddenly ", " until ", " except "]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn humanize_tag(tag: &str) -> String {
    tag.replace(['-', '_'], " ")
}

fn concise_beat(text: &str) -> String {
    limit_at_word(text.trim().trim_end_matches(['.', '!', '?']), 52)
}

fn sentence_case(text: &str) -> String {
    let cleaned = normalize_spacing(text)
        .trim_end_matches(['.', '!', '?'])
        .to_string();
    let mut chars = cleaned.chars();
    let body = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => return String::new(),
    };
    format!("{body}.")
}

fn title_case_copy(text: &str) -> String {
    let cleaned = normalize_spacing(text)
        .trim_end_matches(['.', '!', '?'])
        .to_string();
    if cleaned.starts_with('"') {
        return cleaned;
    }
    cleaned
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn limit_at_word(text: &str, max_chars: usize) -> String {
    let cleaned = normalize_spacing(text);
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let prefix: String = cleaned.chars().take(max_chars).collect();
    match prefix.rfind(' ') {
        Some(index) if index >= max_chars / 2 => prefix[..index].trim().to_string(),
        _ => prefix,
    }
}

fn trim_evidence(text: &str) -> String {
    limit_at_word(
        text.trim().trim_matches(['"', '\'', ' ', '.', '!', '?']),
        MAX_EVIDENCE_TEXT,
    )
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_spacing)
        .filter(|value| !value.is_empty())
}

fn normalize_spacing(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_copy(text: &str) -> String {
    text.to_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(transcript: &str, summary: &str, tags: &[&str], style: &str) -> MomentBrief {
        let tags = tags
            .iter()
            .map(|tag| (*tag).to_string())
            .collect::<Vec<_>>();
        let signals = vec!["audio".to_string(), "transcript".to_string()];
        MomentBrief::build(MomentEvidence {
            transcript: Some(transcript),
            detector_summary: Some(summary),
            detector_title: None,
            payoff_summary: None,
            outcome_label: None,
            tags: &tags,
            game: Some("Fixture Game"),
            stream_style: Some(style),
            signal_sources: &signals,
            scores: MomentScores {
                hook: 0.78,
                emotion: 0.72,
                payoff: 0.81,
                alignment: 0.76,
                context: 0.84,
                confidence: 0.79,
            },
        })
    }

    #[test]
    fn action_fixture_keeps_setup_and_payoff_grounded() {
        let brief = brief(
            "I hear the killer behind us. Wait, vault the window! We both escaped the hit.",
            "vaulted the window before the killer could land the hit",
            &["chase", "escape", "panic"],
            "action",
        );
        assert_eq!(brief.stream_style, MomentStyle::Action);
        assert!(brief.core_event.contains("vaulted the window"));
        let title = brief.title_suggestions(0, &HashMap::new())[0].clone();
        assert!(brief.validate_title(&title.text).is_ok());
        let captions = brief.caption_suggestions("punchy", &title.text, 0, &HashMap::new());
        assert!(!captions.is_empty());
        assert!(captions
            .iter()
            .all(|item| brief.validate_description(&item.text, &title.text).is_ok()));
    }

    #[test]
    fn cozy_fixture_does_not_invent_action_claims() {
        let brief = brief(
            "I planted the last row of pumpkins. The rain started, so we stayed inside and decorated.",
            "finished the pumpkin field before the rain started",
            &["farming", "cozy", "conversation"],
            "cozy",
        );
        assert_eq!(brief.stream_style, MomentStyle::Cozy);
        let title = &brief.title_suggestions(1, &HashMap::new())[0];
        assert!(!title.text.to_lowercase().contains("clutch"));
        assert!(!title.text.to_lowercase().contains("kill"));
    }

    #[test]
    fn talking_fixture_prefers_verified_dialogue() {
        let brief = brief(
            "I thought the shortcut would save time. It added twenty minutes, and everybody noticed.",
            "the shortcut added twenty minutes instead of saving time",
            &["conversation", "reaction"],
            "talking",
        );
        assert_eq!(brief.stream_style, MomentStyle::Talking);
        assert!(!brief.quote_candidates.is_empty());
        let title = brief.title_suggestions(2, &HashMap::new())[0].clone();
        let captions = brief.caption_suggestions("direct_quote", &title.text, 0, &HashMap::new());
        assert!(captions.iter().any(|item| item.text.contains('"')));
    }

    #[test]
    fn story_fixture_preserves_turn_and_ending() {
        let brief = brief(
            "We went back for the missing key. But the door was already open. The note explained who opened it.",
            "the open door led to the note that explained the mystery",
            &["story", "reveal", "dialogue"],
            "story",
        );
        assert_eq!(brief.stream_style, MomentStyle::Story);
        assert!(brief.pivot.is_some());
        assert!(brief.payoff.is_some());
        let title = brief.title_suggestions(0, &HashMap::new())[0].clone();
        let captions = brief.caption_suggestions("observation", &title.text, 0, &HashMap::new());
        assert!(!captions.is_empty());
    }

    #[test]
    fn rejects_generic_unsupported_and_duplicate_copy() {
        let brief = brief(
            "I planted the last row before the rain started.",
            "finished planting before the rain",
            &["farming", "rain"],
            "cozy",
        );
        assert_eq!(
            brief.validate_title("What a moment"),
            Err(CopyRejection::Generic)
        );
        assert!(matches!(
            brief.validate_title("Won the boss fight"),
            Err(CopyRejection::UnsupportedClaim(_))
        ));
        let title = "Finished Planting Before the Rain";
        assert_eq!(
            brief.validate_description("Finished planting before the rain", title),
            Err(CopyRejection::RepeatsTitle)
        );
    }

    #[test]
    fn feedback_preferences_reorder_future_strategy_choices() {
        let brief = brief(
            "I heard the door open. Then the killer stepped through. We vaulted out before the hit.",
            "vaulted away before the killer landed the hit",
            &["chase", "escape"],
            "action",
        );
        let mut preferences = HashMap::new();
        preferences.insert("quote".to_string(), 5.0);
        let suggestions = brief.title_suggestions(0, &preferences);
        assert_eq!(
            suggestions.first().map(|item| item.strategy.as_str()),
            Some("quote")
        );
    }

    #[test]
    fn signature_is_deterministic_and_changes_with_evidence() {
        let first = brief(
            "we found the key",
            "found the missing key",
            &["story"],
            "story",
        );
        let same = brief(
            "we found the key",
            "found the missing key",
            &["story"],
            "story",
        );
        let changed = brief(
            "we lost the key",
            "lost the missing key",
            &["story"],
            "story",
        );
        assert_eq!(first.signature, same.signature);
        assert_ne!(first.signature, changed.signature);
    }

    #[test]
    fn empty_evidence_produces_no_generic_copy() {
        let tags = Vec::new();
        let signals = Vec::new();
        let brief = MomentBrief::build(MomentEvidence {
            transcript: None,
            detector_summary: None,
            detector_title: None,
            payoff_summary: None,
            outcome_label: None,
            tags: &tags,
            game: Some("Fixture Game"),
            stream_style: Some("mixed"),
            signal_sources: &signals,
            scores: MomentScores::default(),
        });

        assert!(brief.core_event.is_empty());
        assert!(brief.title_suggestions(0, &HashMap::new()).is_empty());
        assert!(brief
            .caption_suggestions("punchy", "", 0, &HashMap::new())
            .is_empty());
    }

    #[test]
    fn unicode_transcript_before_pivot_keeps_safe_offsets() {
        let brief = brief(
            "Élodie found the key, BUT the door was already open.",
            "the open door changed the plan",
            &["story", "reveal"],
            "story",
        );

        assert!(brief.pivot.is_some());
        assert!(brief
            .prompt_context()
            .contains("GAME CONTEXT: Fixture Game"));
    }
}
