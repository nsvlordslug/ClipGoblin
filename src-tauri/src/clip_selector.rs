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
pub enum SignalSource { Audio, Transcript, Chat }

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
}

impl Default for CurationConfig {
    fn default() -> Self {
        Self::for_duration(30.0 * 60.0, "medium")
    }
}

impl CurationConfig {
    /// Build a config scaled to VOD duration and sensitivity.
    ///
    /// Sensitivity: "low" (fewer, best only), "medium" (balanced), "high" (more clips).
    ///
    /// Clip count target:
    ///   ~4-6 for 30 min, ~8-12 for 1h, ~15-25 for 2h, ~20-35 for 3h+
    /// Formula: max(6, min(35, duration_minutes / 6))
    pub fn for_duration(duration_secs: f64, sensitivity: &str) -> Self {
        let duration_min = (duration_secs / 60.0).max(1.0);
        let duration_hrs = duration_min / 60.0;

        // ── Dynamic clip count ──
        let base_max = ((duration_min / 6.0).round() as usize).clamp(6, 35);
        let (max_clips, sensitivity_mult) = match sensitivity {
            "low"  => ((base_max as f64 * 0.6).round() as usize, 0.6_f64),
            "high" => ((base_max as f64 * 1.4).round() as usize, 1.4_f64),
            _      => (base_max, 1.0_f64),
        };
        let max_clips = max_clips.clamp(4, 40);

        // ── Threshold scaling ──
        // Longer VODs → slightly lower bar so good clips aren't thrown away.
        // Sensitivity also shifts thresholds.
        let duration_factor = 1.0 - (duration_hrs * 0.05).min(0.20); // 0.80–1.0
        let sensitivity_threshold = match sensitivity {
            "low"  => 1.15,  // raise the bar
            "high" => 0.85,  // lower the bar
            _      => 1.0,
        };
        let threshold_scale = duration_factor * sensitivity_threshold;

        let min_total_score = (0.35 * threshold_scale).clamp(0.20, 0.45);
        let min_hook        = (0.25 * threshold_scale).clamp(0.15, 0.35);
        let min_emotion     = (0.20 * threshold_scale).clamp(0.10, 0.30);

        // ── Cooldown scaling ──
        // Shorter cooldown for longer VODs (more content spread out).
        // But never below 30s to prevent same-fight clustering.
        let cooldown = (120.0 - (duration_hrs * 15.0)).clamp(45.0, 120.0);

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
        if lower.contains("no") || lower.contains("what") || lower.contains("oh") { tags.push("shock".to_string()); }
        if lower.contains("go") || lower.contains("yes") || lower.contains("clutch") { tags.push("hype".to_string()); }
        if lower.contains("rage") || lower.contains("dead") || lower.contains("done") { tags.push("frustration".to_string()); }
        if lower.contains("run") || lower.contains("help") || lower.contains("behind") { tags.push("panic".to_string()); }
        RawSignal { center: kw.timestamp, intensity, source: SignalSource::Transcript, tags, transcript_snippet: Some(kw.context.clone()), spike_delta: 0.0 }
    }).collect()
}

pub fn generate_chat_candidates(chat_peaks: &[db::HighlightRow]) -> Vec<RawSignal> {
    chat_peaks.iter().map(|ch| {
        RawSignal { center: (ch.start_seconds + ch.end_seconds) / 2.0, intensity: ch.chat_score * 0.8 + 0.2, source: SignalSource::Chat, tags: vec!["chat-peak".to_string(), "reaction".to_string()], transcript_snippet: ch.transcript_snippet.clone(), spike_delta: 0.0 }
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
    if m.transcript_snippet.is_some() { score += 0.08; }
    score.min(1.0)
    // TODO(v2): facial expression recognition
}

pub fn analyze_payoff_clarity(m: &FusedMoment) -> f64 {
    let mut score = 0.25;
    if m.transcript_snippet.is_some() { score += 0.30; }
    score += (m.signal_sources.len() as f64 * 0.12).min(0.25);
    let has = |tag: &str| m.tags.iter().any(|t| t == tag);
    if has("jumpscare") || has("scream") { score += 0.12; }
    if has("shock") || has("panic") { score += 0.08; }
    score.min(1.0)
    // TODO(v2): game state detection (kill feeds, objectives)
}

pub fn analyze_event_reaction_alignment(m: &FusedMoment) -> f64 {
    match m.signal_sources.len() {
        n if n >= 3 => 0.95,
        2 => if m.signal_sources.contains(&SignalSource::Transcript) { 0.82 } else { 0.72 },
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
    if has("jumpscare") || has("surprise") || has("shock") { score += 0.40; }
    if has("scream") || has("panic") { score += 0.20; }
    if m.transcript_snippet.is_some() { score += 0.12; }
    if m.signal_sources.len() >= 2 { score += 0.10; }
    score.min(1.0)
    // TODO(v2): audio loop analysis
}

pub fn score_clip_candidate(c: &mut ClipCandidate) {
    c.total_score = (c.hook_strength * 0.30)
        + (c.emotional_spike * 0.20)
        + (c.payoff_clarity * 0.20)
        + (c.event_reaction_alignment * 0.15)
        + (c.context_simplicity * 0.10)
        + (c.replay_value * 0.05);
    let bonus = match c.signal_sources.len() { n if n >= 3 => 0.10, 2 => 0.05, _ => 0.0 };
    c.total_score = (c.total_score + bonus).min(0.99);
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
    // If energy is still high at the end, extend to capture full reaction
    let end_energy = a.intensity_in_range((c.end_time - 3.0).max(c.start_time), c.end_time);
    if end_energy > a.avg_rms * 1.5 {
        let extended = (c.end_time + 5.0).min(duration);
        if a.intensity_in_range(c.end_time, extended) > a.avg_rms * 1.2 {
            c.end_time = extended;
        }
    }

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

pub fn select_clips(
    audio: Option<&AudioContext>,
    transcript: Option<&TranscriptResult>,
    chat_peaks: &[db::HighlightRow],
    duration: f64,
    sensitivity: &str,
) -> (Vec<ClipCandidate>, DetectionStats) {
    let cfg = CurationConfig::for_duration(duration, sensitivity);

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
            similarity_fingerprint: String::new(),
            novelty_score: 0.0, diversity_penalty: 0.0, selection_score: 0.0,
            selected_reason: None, rejection_reason: None,
        };
        score_clip_candidate(&mut c);
        c.similarity_fingerprint = compute_similarity_fingerprint(&c);
        c
    }).collect();

    let candidates_found = candidates.len();

    // ── Stage 4: Optimize boundaries ──
    for c in &mut candidates { optimize_clip_boundaries(c, audio, duration); }

    // ── Stage 5: Reject low-quality clips (thresholds scaled by config) ──
    for c in &mut candidates { evaluate_rejection(c, audio, &cfg); }
    let rejected = candidates.iter().filter(|c| c.rejection_reason.is_some()).count();
    candidates.retain(|c| c.rejection_reason.is_none());
    if rejected > 0 { log::info!("Clip selector: rejected {} weak clips", rejected); }

    // ── Stage 6: Suppress duplicates ──
    let before_dedup = candidates.len();
    suppress_duplicate_candidates(&mut candidates);
    let duplicates_suppressed = before_dedup - candidates.len();

    // ── Stage 6b: Enforce minimum gap (30s) — merge or drop heavy overlap ──
    enforce_minimum_gap(&mut candidates, 30.0);

    // ── Stage 7: Diversity-aware final selection with temporal cooldown ──
    log::info!("Clip selector: cooldown={}s distinctness={:.2} penalty={:.2} max_type={} max_clips={}",
        cfg.cooldown_window, cfg.cooldown_distinctness_threshold, cfg.cooldown_penalty,
        cfg.max_same_type, cfg.max_clips);
    let final_clips = diversify_final_selection(&candidates, duration, &cfg);

    log::info!("Clip selector: final {} clips from {} candidates (scores: {})",
        final_clips.len(), candidates_found,
        final_clips.iter().map(|c| format!("{:.0}%", c.total_score * 100.0)).collect::<Vec<_>>().join(", "));

    let stats = DetectionStats {
        candidates_found,
        candidates_rejected: rejected,
        duplicates_suppressed,
        clips_selected: final_clips.len(),
        sensitivity: sensitivity.to_string(),
    };

    (final_clips, stats)
}
