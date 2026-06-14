//! Viral clip selection engine.
//!
//! Pipeline stages:
//!   1. Candidate generation (audio spikes, transcript keywords, chat peaks)
//!   2. Signal fusion (merge overlapping raw signals into moments)
//!   3. Quality scoring (6-dimension viral score per candidate)
//!   4. Boundary optimization (trim dead air, snap to hooks, preserve payoffs)
//!   5. Rejection (discard clips that fail minimum quality checks)
//!   6. Duplicate suppression (remove near-duplicate detections)
//!   7. Diversity-aware final selection (curate a varied, non-repetitive set)

use crate::db;
use crate::commands::vod::{TranscriptKeyword, TranscriptResult};

// ═══════════════════════════════════════════════════════════════════════
// Data structures
// ═══════════════════════════════════════════════════════════════════════

/// Raw signal from a single source (audio, transcript, or chat).
#[derive(Clone, Debug)]
pub struct RawSignal {
    pub center: f64,
    pub intensity: f64,
    pub source: SignalSource,
    pub tags: Vec<String>,
    pub transcript_snippet: Option<String>,
    pub spike_delta: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SignalSource { Audio, Transcript, Chat, Community, EmoteBurst, Semantic }

/// A community-created Twitch clip associated with this VOD. Used as a
/// human-curated detection signal: if multiple viewers clipped a moment,
/// it's almost certainly worth surfacing.
#[derive(Clone, Debug)]
pub struct CommunityClip {
    /// Seconds into the VOD where the clip starts.
    pub vod_offset_seconds: f64,
    /// Clip length in seconds (usually 5-60).
    pub duration_seconds: f64,
    /// Number of views on the clip itself — strong signal of quality.
    pub view_count: i64,
    /// Optional: the clipper's chosen title. Used for display only, not scoring.
    pub title: String,
}

/// Multi-signal moment after fusion.
#[derive(Clone, Debug)]
pub struct FusedMoment {
    pub center: f64,
    pub best_intensity: f64,
    pub spike_delta: f64,
    pub signal_sources: Vec<SignalSource>,
    pub tags: Vec<String>,
    pub transcript_snippet: Option<String>,
}

/// A fully scored clip candidate.
#[derive(Clone, Debug)]
pub struct ClipCandidate {
    pub start_time: f64,
    pub end_time: f64,
    pub peak_time: f64,
    pub transcript_excerpt: Option<String>,
    pub event_tags: Vec<String>,
    pub emotion_tags: Vec<String>,
    pub payoff_summary: Option<String>,
    pub outcome_label: Option<String>,
    pub signal_sources: Vec<SignalSource>,

    // Viral scoring dimensions (0.0–1.0)
    pub hook_strength: f64,
    pub emotional_spike: f64,
    pub payoff_clarity: f64,
    pub event_reaction_alignment: f64,
    pub context_simplicity: f64,
    pub replay_value: f64,
    pub total_score: f64,
    /// AI clip-worthiness verdict (0.0–1.0) when the BYOK judge ran; None when
    /// AI detection is off. Drives the fusion blend and the gate bypass.
    pub ai_score: Option<f64>,

    // Selection metadata
    pub similarity_fingerprint: String,
    pub novelty_score: f64,
    pub diversity_penalty: f64,
    pub selection_score: f64,
    pub selected_reason: Option<String>,
    pub rejection_reason: Option<String>,
}

/// Configurable curation parameters.
pub struct CurationConfig {
    /// Seconds after a selected clip during which nearby clips are penalized.
    pub cooldown_window: f64,
    /// Similarity threshold (0–1) below which a clip inside the cooldown window
    /// is considered "clearly different" and exempt from the cooldown penalty.
    pub cooldown_distinctness_threshold: f64,
    /// Penalty applied to clips inside the cooldown window that aren't distinct.
    /// 1.0 = full rejection, 0.5 = halve their selection score, etc.
    pub cooldown_penalty: f64,
    /// Maximum clips of the same type fingerprint (event+emotion).
    pub max_same_type: usize,
    /// Maximum total clips to select.
    pub max_clips: usize,
    /// Minimum score threshold for the rejection stage.
    pub min_total_score: f64,
    /// Minimum hook strength to survive rejection.
    pub min_hook: f64,
    /// Minimum emotional spike to survive rejection.
    pub min_emotion: f64,
    /// Minimum 0–100 display score a clip must reach to qualify (the user-facing
    /// quality floor; genuinely differs per sensitivity — see DisplayCalibrator).
    pub min_display_score: f64,
}

impl CurationConfig {
    /// Build a config scaled to VOD duration and sensitivity.
    ///
    /// Sensitivity: "low" (fewer, best only), "medium" (balanced), "high" (more clips).
    ///
    /// Clip count target:
    ///   ~4-6 for 30 min, ~8-12 for 1h, ~15-25 for 2h, ~20-35 for 3h+
    /// Formula: max(6, min(35, duration_minutes / 6))
    pub fn for_duration(
        duration_secs: f64,
        sensitivity: &str,
        selector_config: &crate::game_config::SelectorConfig,
    ) -> Self {
        let duration_min = (duration_secs / 60.0).max(1.0);
        let duration_hrs = duration_min / 60.0;

        // ── Dynamic clip count ──
        let base_max = ((duration_min / 6.0).round() as usize).clamp(6, 35);
        let (max_clips, sensitivity_mult) = match sensitivity {
            "low"  => ((base_max as f64 * 0.6).round() as usize, 0.6_f64),
            "high" => ((base_max as f64 * 1.4).round() as usize, 1.4_f64),
            _      => ((base_max as f64 * 0.8).round() as usize, 1.0_f64),
        };
        let max_clips = max_clips.clamp(4, 40);

        // ── Threshold scaling ──
        // Longer VODs → slightly lower bar so good clips aren't thrown away.
        // Sensitivity also shifts thresholds.
        let duration_factor = 1.0 - (duration_hrs * 0.03).min(0.12); // 0.88–1.0
        let sensitivity_threshold = match sensitivity {
            "low"  => 1.15,  // raise the bar
            "high" => 0.85,  // lower the bar
            _      => 1.0,
        };
        let threshold_scale = duration_factor * sensitivity_threshold;

        let min_total_score = (0.50 * threshold_scale).clamp(0.50, 0.55);
        let min_hook        = (0.32 * threshold_scale).clamp(0.22, 0.40);
        let min_emotion     = (0.28 * threshold_scale).clamp(0.18, 0.35);
        // User-facing quality floor on the 0–100 display scale. Genuinely differs
        // per preset (the old min_total_score clamp collapsed Medium==High).
        let min_display_score = match sensitivity {
            "low"  => 58.0,
            "high" => 30.0,
            _      => 40.0,
        };

        // ── Cooldown scaling ──
        // Shorter cooldown for longer VODs (more content spread out). Floor
        // is the per-game min_gap_between_clips so cozy/RPG games get the
        // longer gaps they need even on short VODs.
        let dynamic_cooldown = (120.0 - (duration_hrs * 15.0)).clamp(45.0, 120.0);
        let cooldown = dynamic_cooldown.max(selector_config.min_gap_between_clips as f64);

        // ── Same-type cap scales with clip count ──
        let max_same_type = ((max_clips as f64 * 0.35).ceil() as usize).clamp(2, 6);

        log::info!(
            "CurationConfig: duration={:.0}min sensitivity={} max_clips={} thresholds=({:.2}/{:.2}/{:.2}) cooldown={:.0}s max_same_type={}",
            duration_min, sensitivity, max_clips, min_total_score, min_hook, min_emotion, cooldown, max_same_type
        );

        Self {
            cooldown_window: cooldown,
            cooldown_distinctness_threshold: 0.25,
            cooldown_penalty: if sensitivity == "high" { 0.45 } else { 0.60 },
            max_same_type,
            max_clips,
            min_total_score,
            min_hook,
            min_emotion,
            min_display_score,
        }
    }
}

/// Per-second audio data.
pub struct AudioContext {
    pub rms_per_second: Vec<f64>,
    pub avg_rms: f64,
    pub spike_seconds: Vec<usize>,
}

impl AudioContext {
    pub fn new(rms: Vec<f64>, spikes: Vec<usize>) -> Self {
        let avg = rms.iter().sum::<f64>() / rms.len().max(1) as f64;
        Self { rms_per_second: rms, avg_rms: avg, spike_seconds: spikes }
    }

    pub fn intensity_in_range(&self, start: f64, end: f64) -> f64 {
        let s = (start as usize).min(self.rms_per_second.len().saturating_sub(1));
        let e = (end as usize).min(self.rms_per_second.len());
        if e <= s { return 0.0; }
        self.rms_per_second[s..e].iter().sum::<f64>() / (e - s) as f64
    }

    /// Per-second baseline-relative z-score envelope: how far each second sits
    /// above the stream's OWN rolling normal, in units of its variability. This
    /// is what lets a spike on a loud-but-steady stream stand out (the global
    /// avg_rms cannot). Slow half-life 90s ("normal"), fast 5s ("now").
    pub fn z_envelope(&self) -> Vec<f64> {
        let mut b = crate::signal_calibration::RollingBaseline::new(1.0, 90.0, 5.0, 1e-4);
        self.rms_per_second.iter().map(|&x| b.push(x)).collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 1: Candidate generation
// ═══════════════════════════════════════════════════════════════════════

pub fn generate_audio_candidates(audio: &AudioContext, _duration: f64) -> Vec<RawSignal> {
    let len = audio.rms_per_second.len();
    let window = 3usize;
    let avg = audio.avg_rms;

    struct Moment { sec: usize, ratio: f64, delta: f64, before: f64, _during: f64 }
    let mut moments: Vec<Moment> = Vec::new();

    for i in window..len.saturating_sub(window) {
        let before_start = i.saturating_sub(5);
        let before: f64 = audio.rms_per_second[before_start..i].iter().sum::<f64>()
            / (i - before_start).max(1) as f64;
        let during: f64 = audio.rms_per_second[i..i+window].iter().sum::<f64>()
            / window as f64;
        if during <= avg * 1.3 { continue; }
        moments.push(Moment { sec: i, ratio: during / avg.max(0.001), delta: during - before, before, _during: during });
    }

    moments.sort_by(|a, b| {
        let sa = a.delta * 2.0 + a.ratio;
        let sb = b.delta * 2.0 + b.ratio;
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut used: Vec<usize> = Vec::new();
    let mut signals: Vec<RawSignal> = Vec::new();

    for (rank, m) in moments.iter().enumerate() {
        if used.iter().any(|&u| (m.sec as i64 - u as i64).unsigned_abs() < 30) { continue; }
        used.push(m.sec);

        let intensity = (0.95 - (rank as f64 * 0.03)).max(0.45);
        let mut tags = vec!["audio-spike".to_string()];

        if m.delta > avg * 1.5 && m.before < avg * 0.8 {
            tags.extend(["ambush", "jumpscare", "shock"].map(String::from));
        } else if m.ratio > 2.5 {
            tags.extend(["fight", "scream", "reaction"].map(String::from));
        } else if m.ratio > 1.8 {
            tags.extend(["chase", "encounter", "hype"].map(String::from));
        } else {
            tags.extend(["encounter", "skirmish"].map(String::from));
        }

        signals.push(RawSignal { center: m.sec as f64 + 1.5, intensity, source: SignalSource::Audio, tags, transcript_snippet: None, spike_delta: m.delta });
        if signals.len() >= 15 { break; }
    }
    signals
}

pub fn generate_transcript_candidates(keywords: &[TranscriptKeyword]) -> Vec<RawSignal> {
    keywords.iter().map(|kw| {
        let lower = kw.keyword.to_lowercase();
        let intensity = if lower.contains("no way") || lower.contains("oh my god") || lower.contains("what the") || lower.contains("holy") { 0.85 }
            else if lower.contains("let's go") || lower.contains("clutch") || lower.contains("rage") || lower.contains("noooo") { 0.75 }
            else { 0.60 };
        let mut tags = vec!["speech".to_string()];
        // — Emotion tagging: require stronger signals than single common words —
        // Shock: needs exclamation OR multiple shock words together, not just "oh" alone
        let shock_words = ["what the", "oh my", "oh no", "holy", "wtf", "dude"];
        let shock_exclaim = (lower.contains("no!") || lower.contains("what!") || lower.contains("oh!"));
        if shock_words.iter().any(|w| lower.contains(w)) || shock_exclaim {
            tags.push("shock".to_string());
        }

        // Hype: "go" alone is too common — require gaming-specific phrases
        let hype_words = ["let's go!", "lets go", "let's gooo", "yes!", "clutch", "got 'em", "got him", "got em", "nice!", "huge"];
        if hype_words.iter().any(|w| lower.contains(w)) {
            tags.push("hype".to_string());
        }

        // Frustration: "dead" and "done" are too common in gaming narration
        let frust_words = ["rage", "i'm done", "are you kidding", "are you serious", "bull", "stupid", "i quit", "i can't"];
        if frust_words.iter().any(|w| lower.contains(w)) {
            tags.push("frustration".to_string());
        }

        // Panic: these are more specific, keep but tighten
        let panic_words = ["run!", "help!", "behind me", "behind you", "get out", "oh god", "he's coming", "she's coming"];
        if panic_words.iter().any(|w| lower.contains(w)) {
            tags.push("panic".to_string());
        }
        RawSignal { center: kw.timestamp, intensity, source: SignalSource::Transcript, tags, transcript_snippet: Some(kw.context.clone()), spike_delta: 0.0 }
    }).collect()
}

pub fn generate_chat_candidates(chat_peaks: &[db::HighlightRow]) -> Vec<RawSignal> {
    chat_peaks.iter().map(|ch| {
        RawSignal { center: (ch.start_seconds + ch.end_seconds) / 2.0, intensity: ch.chat_score * 0.8 + 0.2, source: SignalSource::Chat, tags: vec!["chat-peak".to_string(), "reaction".to_string()], transcript_snippet: ch.transcript_snippet.clone(), spike_delta: 0.0 }
    }).collect()
}

/// Convert emote-burst windows into RawSignals. Emote bursts use shorter
/// windows than chat-rate peaks (10s vs 30s) and trigger on cleaner thresholds
/// because the signal is sharper — a 5-emote spike in 2 seconds is hard to
/// fake. Centered mid-window so the fusion catches nearby audio/transcript.
///
/// `chat_score` on the input HighlightRow carries the normalized emote-density
/// (peak / max_peak across the VOD), so it maps directly to intensity here.
pub fn generate_emote_candidates(emote_peaks: &[db::HighlightRow]) -> Vec<RawSignal> {
    emote_peaks.iter().map(|ch| {
        RawSignal {
            center: (ch.start_seconds + ch.end_seconds) / 2.0,
            // Same shape as chat: floor at 0.2, scale up to 1.0 with the chat_score.
            intensity: ch.chat_score * 0.8 + 0.2,
            source: SignalSource::EmoteBurst,
            tags: vec!["emote-burst".to_string(), "reaction".to_string()],
            transcript_snippet: ch.transcript_snippet.clone(),
            spike_delta: 0.0,
        }
    }).collect()
}

/// Convert community-created Twitch clips into detection signals. Each clip's
/// center timestamp becomes a RawSignal with intensity scaled by view count.
///
/// Intensity curve: log(views + 1) / log(1000), clamped to [0.45, 1.0].
/// This means:
///   1 view    → 0.45  (floor — still a strong human signal, just not viral)
///   10 views  → 0.45
///   50 views  → 0.57
///   200 views → 0.77
///   1k+ views → 1.0   (ceiling)
pub fn generate_community_candidates(clips: &[CommunityClip]) -> Vec<RawSignal> {
    clips.iter().map(|c| {
        let raw = ((c.view_count as f64 + 1.0).ln() / 1000.0_f64.ln()).clamp(0.45, 1.0);
        RawSignal {
            // Center the signal mid-clip so the fusion window catches nearby
            // audio/chat/transcript peaks.
            center: c.vod_offset_seconds + c.duration_seconds / 2.0,
            intensity: raw,
            source: SignalSource::Community,
            tags: vec!["community-clip".to_string(), "viewer-validated".to_string()],
            transcript_snippet: if c.title.is_empty() { None } else { Some(c.title.clone()) },
            spike_delta: 0.0,
        }
    }).collect()
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 2: Signal fusion
// ═══════════════════════════════════════════════════════════════════════

pub fn fuse_signals(signals: &[RawSignal], merge_window: f64) -> Vec<FusedMoment> {
    let mut sorted = signals.to_vec();
    sorted.sort_by(|a, b| a.center.partial_cmp(&b.center).unwrap_or(std::cmp::Ordering::Equal));
    let mut moments: Vec<FusedMoment> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let anchor = &sorted[i];
        let mut tags = anchor.tags.clone();
        let mut best_intensity = anchor.intensity;
        let mut best_delta = anchor.spike_delta;
        let mut sources = vec![anchor.source.clone()];
        let mut snippet = anchor.transcript_snippet.clone();
        let mut center = anchor.center;
        let mut j = i + 1;
        while j < sorted.len() && (sorted[j].center - anchor.center) < merge_window {
            let c = &sorted[j];
            tags.extend(c.tags.clone());
            if c.intensity > best_intensity { best_intensity = c.intensity; center = c.center; }
            if c.spike_delta > best_delta { best_delta = c.spike_delta; }
            if !sources.contains(&c.source) { sources.push(c.source.clone()); }
            if snippet.is_none() { snippet = c.transcript_snippet.clone(); }
            j += 1;
        }
        i = j;
        tags.sort(); tags.dedup();
        moments.push(FusedMoment { center, best_intensity, spike_delta: best_delta, signal_sources: sources, tags, transcript_snippet: snippet });
    }
    moments
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 3: Quality scoring
// ═══════════════════════════════════════════════════════════════════════

pub fn analyze_hook_strength(m: &FusedMoment, audio: Option<&AudioContext>) -> f64 {
    let Some(a) = audio else { return 0.3 + m.best_intensity * 0.3 };
    let hook_start = (m.center - 2.0).max(0.0);
    let hook_audio = a.intensity_in_range(hook_start, hook_start + 3.0);
    let ratio = hook_audio / a.avg_rms.max(0.001);
    let before = a.intensity_in_range((hook_start - 5.0).max(0.0), hook_start);
    let delta_boost = if before < a.avg_rms * 0.7 && hook_audio > a.avg_rms * 1.5 { 0.25 } else { 0.0 };
    ((ratio * 0.35) + delta_boost).min(1.0)
    // TODO(v2): visual first-frame saliency model
}

pub fn analyze_emotional_spike(m: &FusedMoment, audio: Option<&AudioContext>) -> f64 {
    let mut score = m.best_intensity * 0.45;
    let has = |tag: &str| m.tags.iter().any(|t| t == tag);
    if has("jumpscare") || has("shock") || has("surprise") { score += 0.35; }
    else if has("scream") || has("reaction") || has("frustration") || has("panic") { score += 0.25; }
    else if has("hype") || has("excitement") { score += 0.15; }
    if let Some(a) = audio { if a.intensity_in_range(m.center - 1.0, m.center + 2.0) > a.avg_rms * 2.0 { score += 0.10; } }
    if m.transcript_snippet.is_some() { score += 0.03; }
    // Community validation boost — viewers thought this was clip-worthy.
    if has("community-clip") { score += 0.12; }
    score.min(1.0)
    // TODO(v2): facial expression recognition
}

pub fn analyze_payoff_clarity(m: &FusedMoment) -> f64 {
    let mut score = 0.25;
    if m.transcript_snippet.is_some() { score += 0.10; }
    score += (m.signal_sources.len() as f64 * 0.12).min(0.25);
    let has = |tag: &str| m.tags.iter().any(|t| t == tag);
    if has("jumpscare") || has("scream") { score += 0.12; }
    if has("shock") || has("panic") { score += 0.08; }
    // Community signal implies a concrete payoff viewers recognized.
    if has("community-clip") { score += 0.10; }
    score.min(1.0)
    // TODO(v2): game state detection (kill feeds, objectives)
}

pub fn analyze_event_reaction_alignment(m: &FusedMoment) -> f64 {
    match m.signal_sources.len() {
        n if n >= 3 => 0.95,
        2 => if m.signal_sources.contains(&SignalSource::Transcript) { 0.76 } else { 0.72 },
        _ => if m.spike_delta > 0.0 { 0.35 + (m.spike_delta * 0.5).min(0.25) } else { 0.30 + m.best_intensity * 0.2 },
    }
    // TODO(v2): temporal alignment model
}

pub fn analyze_context_simplicity(m: &FusedMoment) -> f64 {
    let has = |tag: &str| m.tags.iter().any(|t| t == tag);
    if has("jumpscare") || has("scream") || has("shock") || has("surprise") { 0.88 }
    else if has("hype") || has("excitement") || has("panic") { 0.68 }
    else if has("frustration") || has("chat-peak") { 0.55 }
    else { 0.45 }
    // TODO(v2): game identification for context requirements
}

pub fn analyze_replay_value(m: &FusedMoment) -> f64 {
    let mut score = m.best_intensity * 0.35;
    let has = |tag: &str| m.tags.iter().any(|t| t == tag);
    if has("jumpscare") || has("surprise") { score += 0.40; }
    else if has("shock") { score += 0.20; }
    if has("scream") || has("panic") { score += 0.20; }
    if m.transcript_snippet.is_some() { score += 0.05; }
    if m.signal_sources.len() >= 2 { score += 0.10; }
    score.min(1.0)
    // TODO(v2): audio loop analysis
}

/// A candidate is corroborated when its loudness is backed by something
/// INDEPENDENT: ≥2 distinct signal sources agree, or viewers themselves clipped
/// it (Community). Keyword-derived emotion/event tags deliberately do NOT count —
/// they're produced from a single signal (already in the source count) and are
/// over-applied in practice (real data: the "shock" tag lands on mundane OBS
/// chatter), so trusting them would re-admit the very ambient laughter this gate
/// exists to dampen.
fn is_corroborated(c: &ClipCandidate) -> bool {
    c.signal_sources.len() >= 2 || c.signal_sources.contains(&SignalSource::Community)
}

/// Max baseline-relative audio boost an UNcorroborated moment may receive. A
/// small spike (a real single-signal moment — e.g. a talky VOD's reactions) is
/// already under this and passes through untouched; only a BIG bare spike (a
/// loud laugh with no independent backing) gets capped. This is the key to
/// fixing the over-rate WITHOUT re-starving single-signal streams — validated on
/// both real VODs (laugh clip 90→70, "You sound big" holds at 6 clips). Tunable.
const UNCORROBORATED_BOOST_CAP: f64 = 0.12;

/// Baseline-relative audio boost for a moment's local z-score, CAPPED by
/// corroboration. The loud-stream fix (commit cf8502a) made honest: a real
/// (corroborated) spike earns the full lift, a bare loud spike at most the cap.
fn audio_boost(local_z: f64, corroborated: bool) -> f64 {
    let base = (local_z / 4.0).clamp(0.0, 0.35);
    if corroborated { base } else { base.min(UNCORROBORATED_BOOST_CAP) }
}

pub fn score_clip_candidate(c: &mut ClipCandidate) {
    // Phase A: transcript-only candidates emit a boilerplate dimension
    // fingerprint (typically context=0.88 from the "shock" tag branch in
    // analyze_context_simplicity, emotion≈0.7625 from the same tag
    // driving analyze_emotional_spike) regardless of actual content. The
    // override below replaces those values with less-confident defaults
    // BEFORE the weighted-sum total is computed, so the total reflects
    // the scorer's actual epistemic state for transcript-only inputs.
    // See docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md
    // Phase A (amended): the override + cap fire for any single-signal
    // candidate carrying shock-family tags ("shock" / "jumpscare" /
    // "scream" / "surprise"). These are the clips where
    // analyze_context_simplicity returns 0.88 and analyze_emotional_spike
    // gets the +0.35 shock-tag boost — the boilerplate fingerprint Phase B
    // identified. Multi-signal clips with the same tags are unaffected
    // (their other signals provide independent confirmation). See
    // docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md §7a
    let has_shock_family_tag = {
        let tag_check = |t: &str| matches!(t, "shock" | "jumpscare" | "scream" | "surprise");
        c.event_tags.iter().any(|t| tag_check(t.as_str()))
            || c.emotion_tags.iter().any(|t| tag_check(t.as_str()))
    };
    let is_unreliable_single_signal = c.signal_sources.len() == 1 && has_shock_family_tag;
    if is_unreliable_single_signal {
        c.context_simplicity = 0.50;
        c.emotional_spike = 0.40;
    }

    c.total_score = (c.hook_strength * 0.30)
        + (c.emotional_spike * 0.20)
        + (c.payoff_clarity * 0.20)
        + (c.event_reaction_alignment * 0.15)
        + (c.context_simplicity * 0.10)
        + (c.replay_value * 0.05);
    let bonus = match c.signal_sources.len() { n if n >= 3 => 0.10, 2 => 0.05, _ => 0.0 };
    c.total_score = (c.total_score + bonus).min(0.99);

    // Phase A safety net: single-signal-with-shock-tag candidates capped at
    // 0.65 even if the dimension override + weighted sum somehow lands them
    // above. See docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md §7a
    if is_unreliable_single_signal {
        c.total_score = c.total_score.min(0.65);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 4: Boundary optimization
// ═══════════════════════════════════════════════════════════════════════

/// Optimize clip start — snap to the first real action.
/// Short-form rule: if the first frame isn't interesting, they swipe.
fn optimize_clip_start(c: &mut ClipCandidate, a: &AudioContext) {
    // Pass 1: scan forward up to 10s for the first above-average energy second.
    // This trims all dead movement/wandering before the moment begins.
    let search_end = (c.start_time + 10.0).min(c.end_time - 10.0);
    if search_end > c.start_time {
        let s = c.start_time as usize;
        let e = (search_end as usize).min(a.rms_per_second.len());
        if e > s {
            // Find first second above 90% of average — tight threshold
            if let Some(active) = (s..e).find(|&sec|
                a.rms_per_second.get(sec).copied().unwrap_or(0.0) > a.avg_rms * 0.9
            ) {
                // Start right at the energy, no buffer — the action IS the hook
                let new_start = (active as f64).max(c.start_time);
                if c.end_time - new_start >= 12.0 { c.start_time = new_start; }
            }
        }
    }

    // Pass 2: if first 1.5s are still quiet, jump directly to the loudest point
    let hook_energy = a.intensity_in_range(c.start_time, c.start_time + 1.5);
    if hook_energy < a.avg_rms * 0.85 {
        let ss = c.start_time as usize;
        let se = ((c.start_time + 10.0) as usize).min(a.rms_per_second.len());
        if se > ss {
            if let Some(spike) = (ss..se).max_by(|&x, &y| {
                a.rms_per_second.get(x).unwrap_or(&0.0)
                    .partial_cmp(a.rms_per_second.get(y).unwrap_or(&0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                let spike_val = a.rms_per_second.get(spike).copied().unwrap_or(0.0);
                if spike_val > hook_energy * 1.2 {
                    // Start 0.5s before the spike — just enough context
                    let new_start = (spike as f64 - 0.5).max(0.0);
                    if c.end_time - new_start >= 12.0 { c.start_time = new_start; }
                }
            }
        }
    }
}

/// Optimize clip end to preserve reaction payoff and trim weak tail.
fn optimize_clip_end(c: &mut ClipCandidate, a: &AudioContext, duration: f64) {
    let original_end = c.end_time;

    // Always extend by 5s as a speech-tail buffer. Reactions and replies
    // commonly run a few seconds past the chat/audio peak that triggered
    // selection, and on loud-game VODs (e.g. Elden Ring boss fights) the
    // speaker's voice doesn't necessarily register as "high energy"
    // relative to combat audio average — so a threshold-based extension
    // alone misses common speech tails. 5s is the empirical sweet spot:
    // catches most sentence-completion cases (3s frequently wasn't enough
    // for slower/longer sentences) without burning much benign tail when
    // there's nothing to catch. The 45s hard cap in
    // optimize_clip_boundaries keeps this bounded.
    c.end_time = (c.end_time + 5.0).min(duration);

    // If audio activity is genuinely high at the (new) end, extend further
    // to catch sustained reactions / multi-sentence exchanges.
    let end_energy = a.intensity_in_range((c.end_time - 3.0).max(c.start_time), c.end_time);
    if end_energy > a.avg_rms * 1.1 {
        let extended = (c.end_time + 5.0).min(duration);
        if a.intensity_in_range(c.end_time, extended) > a.avg_rms * 0.9 {
            c.end_time = extended;
        }
    }

    log::info!(
        "[boundary] [{:.1}s..{:.1}s -> {:.1}s] (+{:.1}s, avg_rms={:.3})",
        c.start_time, original_end, c.end_time, c.end_time - original_end, a.avg_rms,
    );

    // If the last 3s are dead air, trim the tail
    let tail_energy = a.intensity_in_range((c.end_time - 3.0).max(c.start_time), c.end_time);
    if tail_energy < a.avg_rms * 0.5 && (c.end_time - c.start_time) > 15.0 {
        // Walk backwards to find where the energy drops off
        let mut cut = c.end_time as usize;
        let start_sec = c.start_time as usize + 10; // keep at least 10s
        while cut > start_sec {
            if a.rms_per_second.get(cut).copied().unwrap_or(0.0) > a.avg_rms * 0.8 {
                c.end_time = (cut as f64 + 2.0).min(duration); // add 2s after last energy
                break;
            }
            cut -= 1;
        }
    }
}

/// Full boundary optimization pipeline.
pub fn optimize_clip_boundaries(c: &mut ClipCandidate, audio: Option<&AudioContext>, duration: f64) {
    let Some(a) = audio else { return };

    optimize_clip_start(c, a);
    optimize_clip_end(c, a, duration);

    // Enforce duration limits: 12–45s for short-form
    let clip_len = c.end_time - c.start_time;
    if clip_len > 45.0 { c.end_time = c.start_time + 45.0; }
    if clip_len < 12.0 {
        c.start_time = (c.start_time - (12.0 - clip_len)).max(0.0);
        // Re-check after clamping — if still too short, extend end instead
        if c.end_time - c.start_time < 12.0 {
            c.end_time = (c.start_time + 12.0).min(duration);
        }
    }

    // Re-score hook after boundary changes
    // TODO(v2): re-run full scoring after boundary optimization for accuracy
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 5: Rejection
// ═══════════════════════════════════════════════════════════════════════

pub fn evaluate_rejection(c: &mut ClipCandidate, audio: Option<&AudioContext>, cfg: &CurationConfig) {
    if c.hook_strength < cfg.min_hook {
        c.rejection_reason = Some(format!("weak hook — first 3s have no energy ({:.0}% < {:.0}%)", c.hook_strength * 100.0, cfg.min_hook * 100.0)); return;
    }
    if c.emotional_spike < cfg.min_emotion {
        c.rejection_reason = Some(format!("no emotional spike — flat energy ({:.0}% < {:.0}%)", c.emotional_spike * 100.0, cfg.min_emotion * 100.0)); return;
    }
    if let Some(a) = audio {
        let body_start = c.start_time + 3.0;
        let body_end = (c.end_time - 2.0).max(body_start + 1.0);
        if a.intensity_in_range(body_start, body_end) < a.avg_rms * 0.4 {
            c.rejection_reason = Some("dead air — clip body is too quiet".into()); return;
        }
    }
    if c.total_score < cfg.min_total_score {
        c.rejection_reason = Some(format!("below viral threshold ({:.0}% < {:.0}%)", c.total_score * 100.0, cfg.min_total_score * 100.0)); return;
    }
    if c.signal_sources.len() == 1 && c.transcript_excerpt.is_none() && c.payoff_clarity < 0.35 {
        c.rejection_reason = Some("vague event — single signal, no transcript".into());
    }
}

/// The absolute no-noise quality gates (everything EXCEPT the score cliff). A
/// candidate failing any of these is genuine noise / dead air and must never
/// surface, regardless of sensitivity. This is the structural "no noise" floor.
fn passes_quality_gates(c: &ClipCandidate, audio: Option<&AudioContext>, cfg: &CurationConfig) -> bool {
    if c.hook_strength < cfg.min_hook { return false; }
    if c.emotional_spike < cfg.min_emotion { return false; }
    if let Some(a) = audio {
        let body_start = c.start_time + 3.0;
        let body_end = (c.end_time - 2.0).max(body_start + 1.0);
        if a.intensity_in_range(body_start, body_end) < a.avg_rms * 0.4 { return false; }
    }
    if c.signal_sources.len() == 1 && c.transcript_excerpt.is_none() && c.payoff_clarity < 0.35 {
        return false;
    }
    true
}

/// Detects scene-card / transition segments (starting-soon / BRB / ending cards):
/// the transcript over the clip is a music annotation — either the WHOLE snippet
/// (e.g. "(upbeat music)", pure music with no speech) OR a music tag sitting near
/// the stream's start/end (intro/outro music). Background music *with* speech
/// around it mid-gameplay is NOT flagged (it has real content).
/// Core scene-card test on a transcript STRING + clip position. A scene card is
/// an intro/outro/BRB screen: a music annotation that is EITHER the whole clip
/// (pure music) OR sits in the first/last 5 minutes (idle chatter over intro/
/// outro music). Pass the FULL-RANGE transcript over the clip — a candidate's
/// single-sentence peak excerpt can miss the music tag entirely.
fn is_scene_card_text(text: &str, peak_time: f64, duration: f64) -> bool {
    let t = text.trim();
    if !(t.to_lowercase().contains("music") && (t.contains('(') || t.contains('['))) {
        return false;
    }
    // Pure music annotation (nothing but bracketed tags) → card anywhere.
    if is_music_only_text(t) {
        return true;
    }
    // Otherwise only a card in the intro/outro band.
    let edge = 300.0;
    peak_time < edge || (duration > 2.0 * edge && peak_time > duration - edge)
}

/// Scene-card test for a candidate, using the authoritative full-range transcript
/// over the clip window when one is available (falls back to the peak excerpt).
/// This is what catches chat-sourced cards (whose excerpt is the chat text) and
/// outro chatter (whose excerpt may omit the music tag).
fn is_scene_card_full(
    c: &ClipCandidate,
    transcript: Option<&TranscriptResult>,
    duration: f64,
) -> bool {
    let text = transcript
        .and_then(|t| crate::commands::vod::extract_transcript_for_range(t, c.start_time, c.end_time))
        .or_else(|| c.transcript_excerpt.clone());
    match text.as_deref() {
        Some(s) => is_scene_card_text(s, c.peak_time, duration),
        None => false,
    }
}

/// True if a transcript STRING is music-only: it has a music annotation and,
/// after stripping all bracketed annotations, no real speech remains. Run on the
/// full-range transcript over a clip window — catches chat-sourced scene cards
/// whose ClipCandidate excerpt is the chat text, not the music tag.
pub fn is_music_only_text(s: &str) -> bool {
    if !(s.to_lowercase().contains("music") && (s.contains('(') || s.contains('['))) {
        return false;
    }
    let mut depth = 0i32;
    let mut remainder = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            c if depth == 0 => remainder.push(c),
            _ => {}
        }
    }
    remainder.trim().is_empty()
}

// ── AI clip-worthiness fusion (Piece 2) ──
// When the BYOK judge ran, blend its per-moment verdict into the signal score.
// The AI is the primary ranker; signals corroborate. This VETOES loud-but-empty
// moments (the AI read the transcript and passed them over → low ai_score →
// demoted out) and RESCUES quiet ones the signals never spiked on (AI-only
// moments appended as Semantic candidates that bypass the signal quality gates).

/// Weight on the AI verdict vs. the signal composite in the fused score.
const AI_WEIGHT: f64 = 0.65;
const SIGNAL_WEIGHT: f64 = 0.35;
/// ai_score for a signal candidate the AI did NOT flag — it read the transcript
/// and passed this moment over, so treat it as probably-not-clip-worthy.
const AI_PASSED_OVER: f64 = 0.15;
/// Neutral signal composite for an AI-discovered moment no signal fired on
/// (quiet banter): the AI vouches; the signals are silent, not opposed.
const AI_RESCUE_SIGNAL: f64 = 0.40;
/// ai_score at/above which a candidate bypasses the signal quality gates — the
/// AI's judgment stands in for the hook/emotion/dead-air checks.
const AI_VOUCH_THRESHOLD: f64 = 0.50;

/// Do two time windows overlap at all?
fn windows_overlap(a_start: f64, a_end: f64, b_start: f64, b_end: f64) -> bool {
    a_start < b_end && b_start < a_end
}

/// The highest-scoring AI moment overlapping `[start, end]`, if any.
fn best_overlapping(
    start: f64,
    end: f64,
    moments: &[crate::clip_judge::JudgedMoment],
) -> Option<&crate::clip_judge::JudgedMoment> {
    moments
        .iter()
        .filter(|m| windows_overlap(start, end, m.start_sec, m.end_sec))
        .max_by(|x, y| x.score.partial_cmp(&y.score).unwrap_or(std::cmp::Ordering::Equal))
}

/// Blend the AI verdict into `candidates` and append AI-discovered moments the
/// signals missed. No-op when `ai_moments` is empty (AI off). Afterwards each
/// candidate's `total_score` is the fused score and `ai_score` is set.
fn fuse_ai_moments(
    candidates: &mut Vec<ClipCandidate>,
    ai_moments: &[crate::clip_judge::JudgedMoment],
    duration: f64,
) {
    if ai_moments.is_empty() {
        return;
    }
    // 1. Blend the AI verdict into every existing signal candidate.
    for c in candidates.iter_mut() {
        let ai = best_overlapping(c.start_time, c.end_time, ai_moments)
            .map(|m| m.score)
            .unwrap_or(AI_PASSED_OVER);
        c.ai_score = Some(ai);
        c.total_score = (AI_WEIGHT * ai + SIGNAL_WEIGHT * c.total_score).min(0.99);
    }
    // 2. Rescue: AI moments overlapping NO signal candidate become candidates.
    let existing: Vec<(f64, f64)> =
        candidates.iter().map(|c| (c.start_time, c.end_time)).collect();
    for m in ai_moments {
        if existing.iter().any(|&(s, e)| windows_overlap(s, e, m.start_sec, m.end_sec)) {
            continue;
        }
        let start = m.start_sec.max(0.0);
        let end = m.end_sec.min(duration);
        if end - start < 1.0 {
            continue;
        }
        let mut c = ClipCandidate {
            start_time: start,
            end_time: end,
            peak_time: (start + end) / 2.0,
            transcript_excerpt: Some(m.reason.clone()),
            event_tags: vec![m.category.clone()],
            emotion_tags: Vec::new(),
            payoff_summary: Some(m.reason.clone()),
            outcome_label: None,
            signal_sources: vec![SignalSource::Semantic],
            hook_strength: 0.5,
            emotional_spike: 0.5,
            payoff_clarity: 0.5,
            event_reaction_alignment: 0.5,
            context_simplicity: 0.5,
            replay_value: 0.5,
            total_score: (AI_WEIGHT * m.score + SIGNAL_WEIGHT * AI_RESCUE_SIGNAL).min(0.99),
            ai_score: Some(m.score),
            similarity_fingerprint: String::new(),
            novelty_score: 0.0,
            diversity_penalty: 0.0,
            selection_score: 0.0,
            selected_reason: None,
            rejection_reason: None,
        };
        c.similarity_fingerprint = compute_similarity_fingerprint(&c);
        candidates.push(c);
    }
}

/// Two-gate selection. Gate A = the no-noise quality gates + the per-sensitivity
/// display-score floor. Gate B = rank Gate-A survivors by score and take the top
/// `max_clips` (the existing diversity/cooldown logic). The old fixed total_score
/// cliff is gone — score now RANKS, it no longer guillotines, so a loud stream's
/// (calibrated) moments are capped rather than collapsed.
fn apply_two_gate_selection(
    candidates: &mut Vec<ClipCandidate>,
    audio: Option<&AudioContext>,
    transcript: Option<&TranscriptResult>,
    duration: f64,
    cfg: &CurationConfig,
) -> Vec<ClipCandidate> {
    let display = crate::signal_calibration::DisplayCalibrator::default();
    // Diagnostic (for real-VOD tuning from the log): quality-gate pass count,
    // scene cards dropped, and the full display-score distribution entering the gate.
    let qpass = candidates.iter().filter(|c| passes_quality_gates(c, audio, cfg)).count();
    let scene_cards = candidates.iter().filter(|c| is_scene_card_full(c, transcript, duration)).count();
    let mut dscores: Vec<f64> = candidates.iter().map(|c| display.to_display(c.total_score)).collect();
    dscores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    log::info!("Clip selector: gate-A — {} of {} pass quality gates; {} scene card(s) dropped; floor={:.0}; display scores desc: {}",
        qpass, candidates.len(), scene_cards, cfg.min_display_score,
        dscores.iter().map(|d| format!("{:.0}", d)).collect::<Vec<_>>().join(","));
    candidates.retain(|c| {
        // An AI-vouched moment bypasses the SIGNAL quality gates — the judge's
        // verdict is its quality check (this is what lets quiet banter survive).
        let ai_vouched = c.ai_score.map_or(false, |s| s >= AI_VOUCH_THRESHOLD);
        !is_scene_card_full(c, transcript, duration)
            && (ai_vouched || passes_quality_gates(c, audio, cfg))
            && display.to_display(c.total_score) >= cfg.min_display_score
    });
    diversify_final_selection(&candidates[..], duration, cfg)
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 6: Duplicate suppression
// ═══════════════════════════════════════════════════════════════════════

/// Compute a similarity fingerprint for a clip (primary event + primary emotion).
fn compute_similarity_fingerprint(c: &ClipCandidate) -> String {
    let event = c.event_tags.first().map(|s| s.as_str()).unwrap_or("none");
    let emotion = c.emotion_tags.first().map(|s| s.as_str()).unwrap_or("none");
    format!("{}+{}", event, emotion)
}

/// Compute similarity — primarily time-based for dedup.
/// Tag similarity is handled separately by diversity penalties.
/// Dedup answers: "are these detections of the SAME moment?"
/// Diversity answers: "do we have enough of this TYPE?"
fn compute_clip_similarity(a: &ClipCandidate, b: &ClipCandidate) -> f64 {
    let mut sim: f64 = 0.0;

    // Timeline proximity is the primary dedup signal.
    // Two detections 20s apart are very likely the same moment.
    let time_gap = (a.peak_time - b.peak_time).abs();
    if time_gap < 10.0 { sim += 0.60; }       // nearly identical
    else if time_gap < 25.0 { sim += 0.40; }   // same sequence
    else if time_gap < 45.0 { sim += 0.20; }   // overlapping clips
    else if time_gap < 90.0 { sim += 0.05; }   // nearby but distinct

    // Tag overlap amplifies time-proximity — same moment + same tags = definite dup
    if time_gap < 60.0 {
        if !a.event_tags.is_empty() && !b.event_tags.is_empty() {
            let shared: usize = a.event_tags.iter().filter(|t| b.event_tags.contains(t)).count();
            let total = a.event_tags.len().max(b.event_tags.len());
            sim += (shared as f64 / total as f64) * 0.20;
        }
        if !a.emotion_tags.is_empty() && !b.emotion_tags.is_empty() {
            let shared: usize = a.emotion_tags.iter().filter(|t| b.emotion_tags.contains(t)).count();
            let total = a.emotion_tags.len().max(b.emotion_tags.len());
            sim += (shared as f64 / total as f64) * 0.10;
        }
    }
    // Far-apart clips: tags don't matter for dedup (diversity handles that)

    // Transcript overlap confirms same moment
    if let (Some(ta), Some(tb)) = (&a.transcript_excerpt, &b.transcript_excerpt) {
        let wa: Vec<&str> = ta.split_whitespace().collect();
        let wb: Vec<&str> = tb.split_whitespace().collect();
        let shared = wa.iter().filter(|w| wb.contains(w)).count();
        let total = wa.len().max(wb.len()).max(1);
        sim += (shared as f64 / total as f64) * 0.15;
    }

    sim.min(1.0)
    // TODO(v2): ML-based semantic similarity via sentence embeddings
}

/// Two clips are near-duplicates if they're detections of the same stream moment.
fn is_near_duplicate(a: &ClipCandidate, b: &ClipCandidate) -> bool {
    compute_clip_similarity(a, b) >= 0.50
}

/// Remove near-duplicate candidates, keeping the highest-scored version.
fn suppress_duplicate_candidates(candidates: &mut Vec<ClipCandidate>) {
    // Sort by score descending — higher-scored clips survive
    candidates.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));

    let mut kept: Vec<ClipCandidate> = Vec::new();
    let mut suppressed = 0;

    for c in candidates.iter() {
        let dominated = kept.iter().any(|existing| is_near_duplicate(c, existing));
        if dominated {
            suppressed += 1;
            log::info!("Clip selector: suppressed duplicate at {:.0}s (sim to existing clip)",
                c.peak_time);
        } else {
            kept.push(c.clone());
        }
    }

    if suppressed > 0 {
        log::info!("Clip selector: suppressed {} near-duplicate clips", suppressed);
    }
    *candidates = kept;
}

/// Enforce a minimum time gap between clips.
/// If two clips overlap by >50% or their peaks are closer than `min_gap_secs`,
/// keep the higher-scored one.
fn enforce_minimum_gap(candidates: &mut Vec<ClipCandidate>, min_gap_secs: f64) {
    // Already sorted by score desc from suppress_duplicate_candidates
    let mut kept: Vec<ClipCandidate> = Vec::new();
    let mut dropped = 0_usize;

    for c in candidates.iter() {
        let too_close = kept.iter().any(|k| {
            // Check peak-to-peak distance
            let peak_gap = (c.peak_time - k.peak_time).abs();
            if peak_gap < min_gap_secs { return true; }

            // Check actual time overlap percentage
            let overlap_start = c.start_time.max(k.start_time);
            let overlap_end = c.end_time.min(k.end_time);
            if overlap_end > overlap_start {
                let overlap_dur = overlap_end - overlap_start;
                let shorter_dur = (c.end_time - c.start_time).min(k.end_time - k.start_time).max(1.0);
                overlap_dur / shorter_dur > 0.50
            } else {
                false
            }
        });

        if too_close {
            dropped += 1;
            log::info!("Clip selector: dropped clip at {:.0}s — too close to existing clip (min gap {:.0}s)",
                c.peak_time, min_gap_secs);
        } else {
            kept.push(c.clone());
        }
    }

    if dropped > 0 {
        log::info!("Clip selector: dropped {} clips for minimum gap enforcement", dropped);
    }
    *candidates = kept;
}

// ═══════════════════════════════════════════════════════════════════════
// Stage 7: Diversity-aware final selection
// ═══════════════════════════════════════════════════════════════════════

/// Compute how novel a candidate is relative to already-selected clips.
fn compute_novelty_score(candidate: &ClipCandidate, selected: &[ClipCandidate]) -> f64 {
    if selected.is_empty() { return 1.0; }

    // Average dissimilarity to all selected clips
    let avg_dissim: f64 = selected.iter()
        .map(|s| 1.0 - compute_clip_similarity(candidate, s))
        .sum::<f64>() / selected.len() as f64;

    avg_dissim
}

/// Compute diversity penalty — heavily penalizes repetition to force editorial variety.
fn compute_diversity_penalty(candidate: &ClipCandidate, selected: &[ClipCandidate], duration: f64) -> f64 {
    let mut penalty: f64 = 0.0;

    let sig = compute_similarity_fingerprint(candidate);

    // Same type penalty — steep escalation to favor the single best version
    let same_type = selected.iter()
        .filter(|s| compute_similarity_fingerprint(s) == sig)
        .count();
    match same_type {
        0 => {},                     // novel type — welcome
        1 => penalty += 0.30,        // one already exists — significant penalty
        _ => penalty += 0.70,        // two+ exist — near-blocking, only extraordinary clips break through
    }

    // Same primary event (even if emotion differs)
    let primary_event = candidate.event_tags.first().cloned().unwrap_or_default();
    if !primary_event.is_empty() {
        let same_event = selected.iter()
            .filter(|s| s.event_tags.first().map(|e| e == &primary_event).unwrap_or(false))
            .count();
        if same_event >= 2 { penalty += 0.25; } // 2+ of the same event type is too many
        else if same_event >= 1 { penalty += 0.10; }
    }

    // Stream region saturation
    let region = stream_region(candidate.peak_time, duration);
    let same_region = selected.iter()
        .filter(|s| stream_region(s.peak_time, duration) == region)
        .count();
    if same_region >= 3 { penalty += 0.35; }
    else if same_region >= 2 { penalty += 0.15; }

    // Temporal clustering is now handled by check_temporal_cooldown().
    // This penalty only catches pairwise content similarity (not time-proximity).

    // Pairwise similarity to existing clips — if very similar to any one, penalize
    let max_sim = selected.iter()
        .map(|s| compute_clip_similarity(candidate, s))
        .fold(0.0_f64, |a, b| a.max(b));
    if max_sim > 0.5 { penalty += 0.25; }
    else if max_sim > 0.3 { penalty += 0.10; }

    penalty.min(0.90_f64)
}

/// Check temporal cooldown: is this candidate too close to an already-selected clip?
/// Returns (is_blocked, penalty) where:
///   is_blocked = true means the clip should be hard-rejected (inside cooldown and not distinct)
///   penalty = soft penalty to apply if not blocked (0.0 if no cooldown applies)
fn check_temporal_cooldown(
    candidate: &ClipCandidate,
    selected: &[ClipCandidate],
    cfg: &CurationConfig,
) -> (bool, f64) {
    let mut max_penalty: f64 = 0.0;

    for existing in selected {
        let gap = (candidate.peak_time - existing.peak_time).abs();
        if gap >= cfg.cooldown_window { continue; }

        // Candidate is inside the cooldown window — check if it's distinct enough
        let sim = compute_clip_similarity(candidate, existing);

        if sim < cfg.cooldown_distinctness_threshold {
            // Clearly different event/content despite being nearby — exempt
            log::info!("Clip selector: [{:.0}s] inside cooldown of [{:.0}s] (gap={:.0}s) but distinct (sim={:.2})",
                candidate.peak_time, existing.peak_time, gap, sim);
            continue;
        }

        // Not distinct enough — compute scaled penalty based on how close and how similar
        let proximity_factor = 1.0 - (gap / cfg.cooldown_window); // 1.0 = adjacent, 0.0 = edge of window
        let similarity_factor = (sim - cfg.cooldown_distinctness_threshold)
            / (1.0 - cfg.cooldown_distinctness_threshold); // normalized 0–1

        let penalty = cfg.cooldown_penalty * proximity_factor * (0.5 + 0.5 * similarity_factor);
        if penalty > max_penalty { max_penalty = penalty; }

        // Hard block if very close AND very similar
        if gap < cfg.cooldown_window * 0.3 && sim > 0.4 {
            log::info!("Clip selector: cooldown-blocked [{:.0}s] — too close to [{:.0}s] (gap={:.0}s, sim={:.2})",
                candidate.peak_time, existing.peak_time, gap, sim);
            return (true, cfg.cooldown_penalty);
        }
    }

    (false, max_penalty)
}

/// Select the final curated set using diversity + cooldown logic.
fn diversify_final_selection(
    candidates: &[ClipCandidate],
    duration: f64,
    cfg: &CurationConfig,
) -> Vec<ClipCandidate> {
    let mut selected: Vec<ClipCandidate> = Vec::new();
    let mut remaining: Vec<ClipCandidate> = candidates.to_vec();

    while selected.len() < cfg.max_clips && !remaining.is_empty() {
        let mut best_idx = 0;
        let mut best_selection_score = f64::MIN;

        for (i, c) in remaining.iter().enumerate() {
            // Cooldown check — may hard-block or apply penalty
            let (blocked, cooldown_penalty) = check_temporal_cooldown(c, &selected, cfg);
            if blocked { continue; }

            let novelty = compute_novelty_score(c, &selected);
            let diversity_penalty = compute_diversity_penalty(c, &selected, duration);

            // SELECTION = quality * 0.55 + novelty * 0.25 + diversity * 0.20
            // minus cooldown penalty
            let diversity_benefit = (1.0 - diversity_penalty).max(0.0);
            let selection_score = (c.total_score * 0.55)
                + (novelty * 0.25)
                + (diversity_benefit * 0.20)
                - cooldown_penalty;

            if selection_score > best_selection_score {
                best_selection_score = selection_score;
                best_idx = i;
            }
        }

        // If the best remaining candidate was hard-blocked, remove it and retry
        if best_selection_score == f64::MIN {
            // All remaining were blocked by cooldown — try removing the worst
            if !remaining.is_empty() { remaining.remove(0); }
            continue;
        }

        let mut chosen = remaining.remove(best_idx);

        // Hard cap: max N of same fingerprint
        let sig = compute_similarity_fingerprint(&chosen);
        let same_count = selected.iter()
            .filter(|s| compute_similarity_fingerprint(s) == sig)
            .count();
        if same_count >= cfg.max_same_type {
            log::info!("Clip selector: hard-blocked [{:.0}s] — {}th '{}' clip refused",
                chosen.peak_time, same_count + 1, sig);
            continue;
        }

        // Final cooldown gate — re-check after removal (in case of index shift)
        let (blocked, _) = check_temporal_cooldown(&chosen, &selected, cfg);
        if blocked { continue; }

        chosen.novelty_score = compute_novelty_score(&chosen, &selected);
        chosen.diversity_penalty = compute_diversity_penalty(&chosen, &selected, duration);
        chosen.selection_score = best_selection_score;

        let region = stream_region(chosen.peak_time, duration);
        chosen.selected_reason = Some(format!(
            "quality={:.0}% novelty={:.0}% type={} region={}",
            chosen.total_score * 100.0, chosen.novelty_score * 100.0, sig, region
        ));

        log::info!("Clip selector: selected [{:.0}s] score={:.0}% sel={:.2} type={} region={}",
            chosen.peak_time, chosen.total_score * 100.0, best_selection_score, sig, region);

        selected.push(chosen);
    }

    selected
}

fn stream_region(time: f64, duration: f64) -> &'static str {
    let pct = time / duration.max(1.0);
    if pct < 0.33 { "early" } else if pct < 0.66 { "mid" } else { "late" }
}

// ═══════════════════════════════════════════════════════════════════════
// Full pipeline
// ═══════════════════════════════════════════════════════════════════════

/// Detection stats returned alongside clips for UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectionStats {
    pub candidates_found: usize,
    pub candidates_rejected: usize,
    pub duplicates_suppressed: usize,
    pub clips_selected: usize,
    pub sensitivity: String,
}

/// Pre-selector: identify audio time ranges worth transcribing.
///
/// Used by the two-pass analysis flow to skip transcribing non-interesting
/// stretches of long VODs. A 7h VOD on CPU base-model whisper takes ~7 hours
/// to transcribe in full — but 90% of the runtime is on stretches the audio,
/// chat, and emote signals already flagged as low-interest. This function
/// returns the time windows that DO carry signal so the caller can transcribe
/// only those (typically 5-15% of the total VOD by duration).
///
/// Each input signal contributes a window centered on its peak with ±30s
/// padding for transcript context — so the transcribed text doesn't get
/// truncated mid-sentence around the actual moment of interest. Adjacent
/// windows within 60s of each other are merged so we don't run dozens of
/// tiny whisper sessions back-to-back (each session has model-load and
/// state-init overhead).
///
/// Trade-off: this loses "transcript-only discovery" — clips that would be
/// found purely from interesting things the streamer says when audio and
/// chat are both quiet. For gaming streamers with reactive chat (the bulk
/// of the user base) this is fine; for narrative streamers it could miss
/// some clips. Mitigation for that case is a separate follow-up (sample
/// exploratory windows from "quiet" stretches).
///
/// Returns an empty vec if no signals fired — caller should fall back to
/// full-VOD transcription in that case (short VODs, dead-chat streams,
/// audio-uniform content where no spikes exist).
pub fn select_candidate_windows(
    audio: Option<&AudioContext>,
    chat_peaks: &[db::HighlightRow],
    emote_peaks: &[db::HighlightRow],
    community_clips: &[CommunityClip],
    duration: f64,
) -> Vec<(f64, f64)> {
    const PADDING_SECS: f64 = 30.0;
    const MERGE_GAP_SECS: f64 = 60.0;

    let mut raw: Vec<(f64, f64)> = Vec::new();

    // Audio peaks: each spike-second contributes a ±30s window.
    if let Some(audio_ctx) = audio {
        for &spike in &audio_ctx.spike_seconds {
            let start = ((spike as f64) - PADDING_SECS).max(0.0);
            let end = ((spike as f64) + PADDING_SECS).min(duration);
            raw.push((start, end));
        }
    }

    // Chat-rate peaks: window around the peak start time.
    for peak in chat_peaks {
        let center = peak.start_seconds;
        let start = (center - PADDING_SECS).max(0.0);
        let end = (center + PADDING_SECS).min(duration);
        raw.push((start, end));
    }

    // Emote-burst peaks: same treatment.
    for peak in emote_peaks {
        let center = peak.start_seconds;
        let start = (center - PADDING_SECS).max(0.0);
        let end = (center + PADDING_SECS).min(duration);
        raw.push((start, end));
    }

    // Community clips: use the clipper's chosen span with padding.
    for cc in community_clips {
        let start = (cc.vod_offset_seconds - PADDING_SECS).max(0.0);
        let end = (cc.vod_offset_seconds + cc.duration_seconds + PADDING_SECS).min(duration);
        raw.push((start, end));
    }

    if raw.is_empty() {
        return Vec::new();
    }

    // Sort by start time, then merge ranges that overlap or sit within
    // MERGE_GAP_SECS of each other. The merge is in-place: each iteration
    // either extends the last accepted range or starts a new one.
    raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for w in raw {
        if let Some(last) = merged.last_mut() {
            if w.0 - last.1 <= MERGE_GAP_SECS {
                last.1 = w.1.max(last.1);
                continue;
            }
        }
        merged.push(w);
    }

    merged
}

pub fn select_clips(
    audio: Option<&AudioContext>,
    transcript: Option<&TranscriptResult>,
    chat_peaks: &[db::HighlightRow],
    emote_peaks: &[db::HighlightRow],
    community_clips: &[CommunityClip],
    ai_moments: &[crate::clip_judge::JudgedMoment],
    duration: f64,
    sensitivity: &str,
    selector_config: &crate::game_config::SelectorConfig,
) -> (Vec<ClipCandidate>, DetectionStats) {
    let cfg = CurationConfig::for_duration(duration, sensitivity, selector_config);

    // ── Stage 1: Generate candidates ──
    let mut all_signals: Vec<RawSignal> = Vec::new();
    if let Some(a) = audio {
        let s = generate_audio_candidates(a, duration);
        log::info!("Clip selector: {} audio candidates", s.len());
        all_signals.extend(s);
    }
    if let Some(t) = transcript {
        let s = generate_transcript_candidates(&t.keywords_found);
        log::info!("Clip selector: {} transcript candidates", s.len());
        all_signals.extend(s);
    }
    let cs = generate_chat_candidates(chat_peaks);
    if !cs.is_empty() { log::info!("Clip selector: {} chat candidates", cs.len()); }
    all_signals.extend(cs);
    let es = generate_emote_candidates(emote_peaks);
    if !es.is_empty() { log::info!("Clip selector: {} emote-burst candidates", es.len()); }
    all_signals.extend(es);
    let community = generate_community_candidates(community_clips);
    if !community.is_empty() {
        log::info!("Clip selector: {} community clip candidates", community.len());
    }
    all_signals.extend(community);
    if all_signals.is_empty() {
        log::warn!("Clip selector: no candidates");
        let stats = DetectionStats {
            candidates_found: 0, candidates_rejected: 0,
            duplicates_suppressed: 0, clips_selected: 0,
            sensitivity: sensitivity.to_string(),
        };
        return (Vec::new(), stats);
    }

    // ── Stage 2: Fuse signals ──
    let moments = fuse_signals(&all_signals, 10.0);
    log::info!("Clip selector: {} fused moments", moments.len());

    // ── Stage 3: Score ──
    let clip_len = 25.0_f64.min(duration * 0.10).max(15.0);
    // Baseline-relative audio calibration (per-second z over the RMS envelope),
    // computed once and reused below to lift moments that spike above the
    // stream's own rolling normal.
    let audio_z: Option<Vec<f64>> = audio.map(|a| a.z_envelope());
    let mut candidates: Vec<ClipCandidate> = moments.iter().map(|m| {
        let start = (m.center - clip_len * 0.3).max(0.0);
        let end = (start + clip_len).min(duration);

        let event_tags: Vec<String> = m.tags.iter().filter(|t| {
            matches!(t.as_str(), "jumpscare"|"ambush"|"chase"|"encounter"|"skirmish"|"fight"|"kill"|"escape"|"death"|"save"|"interrupt"|"hook"|"scream"|"audio-spike")
        }).cloned().collect();
        let emotion_tags: Vec<String> = m.tags.iter().filter(|t| {
            matches!(t.as_str(), "shock"|"surprise"|"panic"|"hype"|"frustration"|"rage"|"fear"|"reaction"|"relief")
        }).cloned().collect();
        let outcome_label = m.tags.iter().find(|t| {
            matches!(t.as_str(), "escape"|"death"|"save"|"win"|"fail"|"clutch")
        }).cloned();

        let mut c = ClipCandidate {
            start_time: start, end_time: end, peak_time: m.center,
            transcript_excerpt: m.transcript_snippet.clone(),
            event_tags, emotion_tags,
            payoff_summary: m.transcript_snippet.clone(),
            outcome_label,
            signal_sources: m.signal_sources.clone(),
            hook_strength: analyze_hook_strength(m, audio),
            emotional_spike: analyze_emotional_spike(m, audio),
            payoff_clarity: analyze_payoff_clarity(m),
            event_reaction_alignment: analyze_event_reaction_alignment(m),
            context_simplicity: analyze_context_simplicity(m),
            replay_value: analyze_replay_value(m),
            total_score: 0.0,
            ai_score: None,
            similarity_fingerprint: String::new(),
            novelty_score: 0.0, diversity_penalty: 0.0, selection_score: 0.0,
            selected_reason: None, rejection_reason: None,
        };
        score_clip_candidate(&mut c);
        // Baseline-relative boost: reward a moment that spikes above the stream's
        // own rolling normal (z) — the loud-throughout-stream fix the global
        // avg_rms can't see — but CAPPED for UNcorroborated spikes so a bare loud
        // spike (ambient laughter) can't dominate the way a corroborated one can.
        if let Some(z) = audio_z.as_deref() {
            let sec = (c.peak_time as usize).min(z.len().saturating_sub(1));
            let local_z = z.get(sec).copied().unwrap_or(0.0);
            c.total_score = (c.total_score + audio_boost(local_z, is_corroborated(&c))).min(1.0);
        }
        c.similarity_fingerprint = compute_similarity_fingerprint(&c);
        // Intro penalty: only for audio-only signals (music/overlays without speech).
        // If transcript is present, the streamer is talking — likely real gameplay.
        if c.peak_time < 150.0 && c.signal_sources.len() == 1 && c.signal_sources.contains(&SignalSource::Audio) {
            let intro_factor = (c.peak_time / 150.0).max(0.3);
            c.total_score *= intro_factor;
            log::info!("Clip selector: intro penalty at {:.0}s (audio-only) — score reduced to {:.0}%",
                c.peak_time, c.total_score * 100.0);
        }
        c
    }).collect();

    let candidates_found = candidates.len();

    // ── Stage 4: Optimize boundaries ──
    for c in &mut candidates { optimize_clip_boundaries(c, audio, duration); }

    // ── Stage 5: Suppress duplicates ──
    let before_dedup = candidates.len();
    suppress_duplicate_candidates(&mut candidates);
    let duplicates_suppressed = before_dedup - candidates.len();

    // ── Stage 5b: Enforce minimum gap (30s) — merge or drop heavy overlap ──
    enforce_minimum_gap(&mut candidates, 30.0);

    // ── Stage 6: Two-gate selection — absolute quality floor + relative top-K.
    // Replaces the old fixed-score rejection cliff that starved loud streams. ──
    log::info!("Clip selector: cooldown={}s distinctness={:.2} penalty={:.2} max_type={} max_clips={} min_display={:.0}",
        cfg.cooldown_window, cfg.cooldown_distinctness_threshold, cfg.cooldown_penalty,
        cfg.max_same_type, cfg.max_clips, cfg.min_display_score);
    // Fuse the AI verdict (no-op when AI detection is off / found nothing).
    fuse_ai_moments(&mut candidates, ai_moments, duration);
    let before_gate = candidates.len();
    let final_clips = apply_two_gate_selection(&mut candidates, audio, transcript, duration, &cfg);
    let rejected = before_gate.saturating_sub(candidates.len());

    log::info!("Clip selector: final {} clips from {} candidates (scores: {})",
        final_clips.len(), candidates_found,
        final_clips.iter().map(|c| format!("{:.0}%", c.total_score * 100.0)).collect::<Vec<_>>().join(", "));

    // NOTE: Per-game min/max clip-duration enforcement was attempted via a
    // post-selection clamp (centered on peak_time) but it overwrote the
    // audio-aware boundaries from optimize_clip_boundaries, producing clips
    // that cut mid-sentence. Reverted for v1.3.11. The TOML knobs
    // (selector.min_clip_duration / max_clip_duration) are documented but
    // not enforced yet — v1.3.12 will integrate them with the audio-aware
    // boundary logic instead of post-clamping.
    let _ = selector_config.min_clip_duration;
    let _ = selector_config.max_clip_duration;

    let stats = DetectionStats {
        candidates_found,
        candidates_rejected: rejected,
        duplicates_suppressed,
        clips_selected: final_clips.len(),
        sensitivity: sensitivity.to_string(),
    };

    (final_clips, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_candidate(sources: Vec<SignalSource>) -> ClipCandidate {
        ClipCandidate {
            start_time: 100.0, end_time: 130.0, peak_time: 115.0,
            transcript_excerpt: None,
            event_tags: Vec::new(),
            emotion_tags: Vec::new(),
            payoff_summary: None,
            outcome_label: None,
            signal_sources: sources,
            hook_strength: 0.5, emotional_spike: 0.7625,
            payoff_clarity: 0.55, event_reaction_alignment: 0.47,
            context_simplicity: 0.88, replay_value: 0.5475,
            total_score: 0.0,
            ai_score: None,
            similarity_fingerprint: String::new(),
            novelty_score: 0.0, diversity_penalty: 0.0, selection_score: 0.0,
            selected_reason: None, rejection_reason: None,
        }
    }

    #[test]
    fn sensitivity_presets_have_distinct_floors_and_caps() {
        let sel = crate::game_config::SelectorConfig { min_clip_duration: 15, max_clip_duration: 60, min_gap_between_clips: 30 };
        let low  = CurationConfig::for_duration(99.0 * 60.0, "low", &sel);
        let med  = CurationConfig::for_duration(99.0 * 60.0, "medium", &sel);
        let high = CurationConfig::for_duration(99.0 * 60.0, "high", &sel);
        // Floors must strictly differ across presets (the Med==High placebo bug).
        assert!(low.min_display_score > med.min_display_score, "low floor must exceed medium");
        assert!(med.min_display_score > high.min_display_score, "medium floor must exceed high");
        // Caps already differ; keep that property.
        assert!(low.max_clips < med.max_clips && med.max_clips < high.max_clips);
    }

    fn envelope(base: f64, peak: f64) -> Vec<f64> {
        let mut v = vec![base; 60];
        v.extend(std::iter::repeat(peak).take(5));
        v.extend(std::iter::repeat(base).take(10));
        v
    }

    #[test]
    fn z_envelope_is_baseline_relative_across_loudness() {
        // Same-shape spike (delta 0.30) on a quiet vs loud steady stream must
        // produce comparable peak z — the loud-stream calibration property.
        let qmax = AudioContext::new(envelope(0.20, 0.50), vec![]).z_envelope()
            .into_iter().fold(f64::MIN, f64::max);
        let lmax = AudioContext::new(envelope(0.50, 0.80), vec![]).z_envelope()
            .into_iter().fold(f64::MIN, f64::max);
        assert!(qmax > 1.0 && lmax > 1.0, "both spikes should register: q={qmax} l={lmax}");
        assert!((qmax - lmax).abs() < qmax.max(lmax) * 0.5,
            "same-shape spikes should give comparable peak z: q={qmax} l={lmax}");
    }

    #[test]
    fn two_gate_caps_a_healthy_set_not_one_not_all() {
        let sel = crate::game_config::SelectorConfig { min_clip_duration: 15, max_clip_duration: 60, min_gap_between_clips: 30 };
        let cfg = CurationConfig::for_duration(99.0 * 60.0, "medium", &sel);
        // 35 well-separated candidates that pass the quality gates and score
        // above the Medium display floor — the bug VOD had 35 candidates collapse
        // to 1 under the old fixed cliff; the two-gate must cap, not collapse.
        let events = ["kill", "escape", "chase", "fight", "death"];
        let emotions = ["hype", "shock", "fear", "rage", "relief"];
        let mut cands: Vec<ClipCandidate> = (0..35).map(|i| {
            let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Chat]);
            c.start_time = (i as f64) * 150.0;
            c.end_time = c.start_time + 25.0;
            c.peak_time = c.start_time + 10.0;
            c.total_score = 0.70; // display ≈ 73, above the medium floor (55)
            c.event_tags = vec![events[i % 5].to_string()];
            c.emotion_tags = vec![emotions[i % 5].to_string()];
            c.similarity_fingerprint = compute_similarity_fingerprint(&c);
            c
        }).collect();
        let kept = apply_two_gate_selection(&mut cands, None, None, 99.0 * 60.0, &cfg);
        assert!(kept.len() >= 5, "must not collapse to ~1, got {}", kept.len());
        assert!(kept.len() <= cfg.max_clips, "must respect the cap, got {} > {}", kept.len(), cfg.max_clips);
    }

    #[test]
    fn two_gate_rejects_dead_air_as_noise() {
        let sel = crate::game_config::SelectorConfig { min_clip_duration: 15, max_clip_duration: 60, min_gap_between_clips: 30 };
        let cfg = CurationConfig::for_duration(99.0 * 60.0, "medium", &sel);
        // Candidates that fail the absolute quality gates → zero clips (no noise).
        let mut cands: Vec<ClipCandidate> = (0..10).map(|i| {
            let mut c = build_test_candidate(vec![SignalSource::Audio]);
            c.start_time = (i as f64) * 150.0;
            c.end_time = c.start_time + 25.0;
            c.hook_strength = 0.05;   // below min_hook
            c.emotional_spike = 0.05; // below min_emotion
            c.total_score = 0.05;
            c
        }).collect();
        let kept = apply_two_gate_selection(&mut cands, None, None, 99.0 * 60.0, &cfg);
        assert_eq!(kept.len(), 0, "dead-air candidates must yield no clips");
    }

    #[test]
    fn loud_steady_stream_with_spikes_yields_clips_end_to_end() {
        // The bug reproduced through the WHOLE pipeline: a loud-throughout stream
        // (baseline 0.45) with many genuine, well-separated spikes (0.90). The old
        // fixed-0.50 cliff collapsed the real loud VOD to 1 clip; calibration +
        // two-gate must surface a healthy set (capped by diversity, not a cliff).
        let dur = 5940.0;
        let mut rms = vec![0.45f64; dur as usize];
        for s in (500usize..5600).step_by(500) {
            for k in 0..4 { rms[s + k] = 0.90; }
        }
        let audio = AudioContext::new(rms, vec![]);
        let sel = crate::game_config::SelectorConfig { min_clip_duration: 15, max_clip_duration: 60, min_gap_between_clips: 30 };
        let (clips, _stats) = select_clips(Some(&audio), None, &[], &[], &[], &[], dur, "medium", &sel);
        assert!(clips.len() >= 4, "loud stream with many real spikes should yield a healthy set, got {}", clips.len());
    }

    #[test]
    fn scene_card_music_rejected_but_gameplay_music_kept() {
        // Cases mirror the real "You sound big" VOD (duration 5938s).
        let dur = 5938.0;
        // Pure music annotations → scene cards (BRB/intro cards), anywhere.
        assert!(is_scene_card_text("(upbeat music)", 141.0, dur));
        assert!(is_scene_card_text("(upbeat music)", 735.0, dur));
        // Music annotation in the outro band, even with idle chatter → ending card.
        assert!(is_scene_card_text("Come on, the SD-screen. Yes. [Piano music] I'm doing it.", 5666.0, dur));
        assert!(is_scene_card_text("I have to have some fabric next to me. [Piano music] yeah", 5758.0, dur));
        // Real speech, no music annotation → kept (the genuine clips).
        assert!(!is_scene_card_text("(Laughter) (Laughter) Oh, yeah. I just added that.", 1922.0, dur));
        assert!(!is_scene_card_text("If he hits you, then he can see Harby's.", 2061.0, dur));
        // Background music WITH speech mid-gameplay (not edge) → kept.
        assert!(!is_scene_card_text("got him (upbeat music) lets go", 3000.0, dur));
        // No transcript and no excerpt → not flagged (other gates handle audio-only).
        let mut audio_only = build_test_candidate(vec![SignalSource::Audio]);
        audio_only.transcript_excerpt = None;
        assert!(!is_scene_card_full(&audio_only, None, dur));
    }

    #[test]
    fn is_music_only_text_detects_scene_cards() {
        assert!(is_music_only_text("(upbeat music)"));
        assert!(is_music_only_text("(upbeat music) (upbeat music) (upbeat music)"));
        assert!(is_music_only_text("[Piano music]"));
        assert!(!is_music_only_text("Come on. [Piano music] I'm doing it.")); // speech present
        assert!(!is_music_only_text("(Laughter) oh yeah")); // no music annotation
        assert!(!is_music_only_text("got him lets go")); // no annotation at all
    }

    #[test]
    fn audio_boost_capped_when_uncorroborated() {
        // A BIG spike (z=2.0 → base 0.35): corroborated keeps it; uncorroborated
        // is capped so a loud laugh can't dominate.
        assert!((audio_boost(2.0, true) - 0.35).abs() < 1e-6);
        assert!((audio_boost(2.0, false) - UNCORROBORATED_BOOST_CAP).abs() < 1e-6);
        // A SMALL spike (z=0.4 → base 0.10) sits UNDER the cap → passes through
        // unchanged. This is what spares single-signal talky VODs from re-starving.
        let small = (0.4f64 / 4.0).clamp(0.0, 0.35);
        assert!((audio_boost(0.4, false) - small).abs() < 1e-6, "small boost passes through uncapped");
        assert!((audio_boost(0.4, true) - small).abs() < 1e-6);
    }

    #[test]
    fn corroboration_requires_independent_signal() {
        // ≥2 distinct sources agree → corroborated.
        let multi = build_test_candidate(vec![SignalSource::Audio, SignalSource::Chat]);
        assert!(is_corroborated(&multi));
        // viewers clipped it (Community) → corroborated even solo.
        let community = build_test_candidate(vec![SignalSource::Community]);
        assert!(is_corroborated(&community));
        // single signal + only keyword tags → NOT corroborated. This is the bug:
        // a loud laugh tagged "hype"/"shock" was sailing through on one signal.
        let mut soft = build_test_candidate(vec![SignalSource::Chat]);
        soft.emotion_tags = vec!["hype".to_string()];
        soft.event_tags = vec!["shock".to_string()];
        assert!(!is_corroborated(&soft));
    }

    fn judged(s: f64, e: f64, score: f64) -> crate::clip_judge::JudgedMoment {
        crate::clip_judge::JudgedMoment {
            start_sec: s, end_sec: e, category: "banter".into(), score,
            reason: "savage deadpan roast".into(),
        }
    }

    #[test]
    fn fusion_noop_when_no_ai_moments() {
        let mut c = vec![build_test_candidate(vec![SignalSource::Audio])];
        c[0].total_score = 0.6;
        fuse_ai_moments(&mut c, &[], 600.0);
        assert!(c[0].ai_score.is_none(), "no AI run → ai_score stays None");
        assert!((c[0].total_score - 0.6).abs() < 1e-9, "score untouched");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn fusion_blends_flagged_and_vetoes_unflagged() {
        let mut c0 = build_test_candidate(vec![SignalSource::Chat]);
        c0.start_time = 100.0; c0.end_time = 130.0; c0.total_score = 0.6;
        let mut c1 = build_test_candidate(vec![SignalSource::Audio]);
        c1.start_time = 500.0; c1.end_time = 530.0; c1.total_score = 0.6;
        let mut cands = vec![c0, c1];
        // One AI moment overlapping c0 only.
        fuse_ai_moments(&mut cands, &[judged(105.0, 125.0, 0.9)], 600.0);
        // flagged: 0.65*0.9 + 0.35*0.6 = 0.795
        assert!((cands[0].total_score - 0.795).abs() < 1e-6);
        assert!((cands[0].ai_score.unwrap() - 0.9).abs() < 1e-9);
        // vetoed (AI passed it over): 0.65*0.15 + 0.35*0.6 = 0.3075
        assert!((cands[1].total_score - 0.3075).abs() < 1e-6);
        assert!((cands[1].ai_score.unwrap() - 0.15).abs() < 1e-9);
    }

    #[test]
    fn fusion_rescues_unmatched_ai_moment_as_semantic() {
        let mut cands = vec![build_test_candidate(vec![SignalSource::Audio])];
        cands[0].start_time = 100.0; cands[0].end_time = 130.0;
        fuse_ai_moments(&mut cands, &[judged(800.0, 825.0, 0.85)], 1000.0);
        assert_eq!(cands.len(), 2, "unmatched AI moment becomes a candidate");
        let rescued = cands.iter().find(|c| c.signal_sources == vec![SignalSource::Semantic]).unwrap();
        assert!((rescued.start_time - 800.0).abs() < 1e-9);
        // rescue: 0.65*0.85 + 0.35*0.40 = 0.6925
        assert!((rescued.total_score - 0.6925).abs() < 1e-6);
        assert_eq!(rescued.transcript_excerpt.as_deref(), Some("savage deadpan roast"));
    }

    #[test]
    fn score_clip_candidate_overrides_dimensions_for_transcript_only() {
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        c.emotion_tags = vec!["shock".to_string()];  // Phase A amendment: tag triggers override
        score_clip_candidate(&mut c);

        // After scoring, the dimensions should reflect the override.
        assert!((c.context_simplicity - 0.50).abs() < 1e-6,
            "context_simplicity should be 0.50, got {}", c.context_simplicity);
        assert!((c.emotional_spike - 0.40).abs() < 1e-6,
            "emotional_spike should be 0.40, got {}", c.emotional_spike);
    }

    #[test]
    fn score_clip_candidate_does_not_override_for_audio_only() {
        let mut c = build_test_candidate(vec![SignalSource::Audio]);
        let original_context = c.context_simplicity;
        let original_emotion = c.emotional_spike;

        score_clip_candidate(&mut c);

        // Audio-only clips keep their original dimension values.
        assert!((c.context_simplicity - original_context).abs() < 1e-6,
            "context_simplicity should be unchanged for audio-only");
        assert!((c.emotional_spike - original_emotion).abs() < 1e-6,
            "emotional_spike should be unchanged for audio-only");
    }

    #[test]
    fn score_clip_candidate_does_not_override_for_multi_signal_with_transcript() {
        let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Transcript]);
        let original_context = c.context_simplicity;
        let original_emotion = c.emotional_spike;

        score_clip_candidate(&mut c);

        // Transcript+audio is multi-signal — no override.
        assert!((c.context_simplicity - original_context).abs() < 1e-6);
        assert!((c.emotional_spike - original_emotion).abs() < 1e-6);
    }

    #[test]
    fn score_clip_candidate_caps_transcript_only_at_65_percent() {
        // Build a candidate with extreme dimension values so the un-capped
        // total would land well above 0.65 even after the Task 3 override.
        // Hook 0.99 alone contributes 0.30. With the override-set context=0.5
        // and emotion=0.4, plus extreme other dims, total approaches the
        // pre-cap ceiling (0.99). The cap should clamp it to 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        c.emotion_tags = vec!["shock".to_string()];  // Phase A amendment: tag triggers override
        c.hook_strength = 0.99;
        c.payoff_clarity = 0.99;
        c.event_reaction_alignment = 0.99;
        c.replay_value = 0.99;
        // (context_simplicity and emotional_spike will be overridden by Task 3)

        score_clip_candidate(&mut c);

        assert!(c.total_score <= 0.65 + 1e-6,
            "transcript-only total_score should be capped at 0.65, got {}", c.total_score);
    }

    #[test]
    fn score_clip_candidate_does_not_cap_multi_signal() {
        // Multi-signal candidate with the same extreme dim values should
        // be allowed to score well above 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Transcript]);
        c.hook_strength = 0.99;
        c.payoff_clarity = 0.99;
        c.event_reaction_alignment = 0.99;
        c.replay_value = 0.99;

        score_clip_candidate(&mut c);

        assert!(c.total_score > 0.65,
            "multi-signal total_score should not be capped, got {}", c.total_score);
    }

    #[test]
    fn score_clip_candidate_phase_b_boilerplate_lands_below_65() {
        // Replays the exact dimension fingerprint observed in Phase B for
        // transcript-only candidates rated boring/meh by the user:
        //   align=0.47, context=0.88, emotion=0.7625, payoff=0.55, replay=0.5475
        // Pre-fix, hook=0.69 (the "Drainage channel" rated-meh clip) produced
        // total_score ≈ 0.70. After the fix, the override + cap should
        // bring total_score below 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        c.emotion_tags = vec!["shock".to_string()];  // Phase A amendment: tag triggers override
        c.hook_strength = 0.69;          // Phase B: "Drainage channel" hook
        c.emotional_spike = 0.7625;      // boilerplate value
        c.payoff_clarity = 0.55;
        c.event_reaction_alignment = 0.47;
        c.context_simplicity = 0.88;     // boilerplate value
        c.replay_value = 0.5475;

        score_clip_candidate(&mut c);

        assert!(c.total_score < 0.65,
            "Phase B boilerplate fingerprint should score below 0.65 after fix, got {}",
            c.total_score);
        // Also verify the dimensions were overridden as expected.
        assert!((c.context_simplicity - 0.50).abs() < 1e-6);
        assert!((c.emotional_spike - 0.40).abs() < 1e-6);
    }

    #[test]
    fn score_clip_candidate_overrides_dimensions_for_audio_only_with_shock_tag() {
        // Audio-only clips with shock-family tags should ALSO get the override.
        // Mirrors the audio-only "Chat about games" Phase B clip pattern:
        // signal_sources=[Audio], tags include "shock" or "jumpscare".
        let mut c = build_test_candidate(vec![SignalSource::Audio]);
        c.event_tags = vec!["jumpscare".to_string(), "audio-spike".to_string()];
        c.emotion_tags = vec!["shock".to_string()];

        score_clip_candidate(&mut c);

        assert!((c.context_simplicity - 0.50).abs() < 1e-6,
            "context_simplicity should be 0.50 for audio-only-with-shock-tag, got {}", c.context_simplicity);
        assert!((c.emotional_spike - 0.40).abs() < 1e-6,
            "emotional_spike should be 0.40, got {}", c.emotional_spike);
    }

    #[test]
    fn score_clip_candidate_does_not_override_for_audio_only_without_shock_tag() {
        // Audio-only with chase/encounter tags (NOT shock-family) keeps original dims.
        let mut c = build_test_candidate(vec![SignalSource::Audio]);
        c.event_tags = vec!["chase".to_string(), "encounter".to_string()];
        c.emotion_tags = vec!["hype".to_string()];
        let original_context = c.context_simplicity;
        let original_emotion = c.emotional_spike;

        score_clip_candidate(&mut c);

        assert!((c.context_simplicity - original_context).abs() < 1e-6,
            "audio-only without shock-family tag should keep context, got {}", c.context_simplicity);
        assert!((c.emotional_spike - original_emotion).abs() < 1e-6);
    }

    #[test]
    fn score_clip_candidate_caps_audio_only_with_shock_tag_at_65_percent() {
        let mut c = build_test_candidate(vec![SignalSource::Audio]);
        c.event_tags = vec!["jumpscare".to_string()];
        c.emotion_tags = vec!["shock".to_string()];
        c.hook_strength = 0.99;
        c.payoff_clarity = 0.99;
        c.event_reaction_alignment = 0.99;
        c.replay_value = 0.99;

        score_clip_candidate(&mut c);

        assert!(c.total_score <= 0.65 + 1e-6,
            "audio-only-with-shock-tag total_score should be capped at 0.65, got {}", c.total_score);
    }
}
