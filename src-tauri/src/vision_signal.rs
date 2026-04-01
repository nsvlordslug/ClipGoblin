//! Pluggable vision-model analysis for clip detection.
//!
//! Abstracts over multiple AI vision providers (Claude, Gemini) behind
//! a single [`VisionAnalyzer`] trait.  The rest of the pipeline never
//! touches provider-specific APIs — it calls [`analyze_frames`] and
//! gets back `Vec<SignalSegment>`.
//!
//! # Architecture
//!
//! ```text
//!  Pipeline                   This module                 Provider
//!  ─────────                  ───────────                 ────────
//!  candidate time ranges      FrameBatch                  Claude API
//!        │                       │                        Gemini API
//!        ▼                       ▼                            │
//!  extract frames ──────► analyze_frames() ──► trait impl ────┘
//!       (ffmpeg)                 │                    │
//!                                ▼                    ▼
//!                        Vec<SignalSegment>    VisionResult
//!                         (pipeline types)    (provider-agnostic)
//! ```
//!
//! # BYOK
//!
//! API keys are never stored in this module.  The caller passes
//! them via [`ProviderConfig`] at analysis time.

use crate::error::AppError;
use crate::pipeline::{SignalMetadata, SignalSegment, SignalType, VisionDimensionScores, VisionProvider};

// ═══════════════════════════════════════════════════════════════════
//  Shared types (provider-agnostic)
// ═══════════════════════════════════════════════════════════════════

/// A single frame to be analyzed, with its timestamp and image data.
#[derive(Debug, Clone)]
pub struct FrameCapture {
    /// Timestamp in the VOD (seconds from start).
    pub timestamp: f64,
    /// JPEG image bytes.
    pub jpeg_bytes: Vec<u8>,
}

/// A batch of frames to send in one API call, plus context.
#[derive(Debug, Clone)]
pub struct FrameBatch {
    /// Frames to analyze (max ~20 per batch for token limits).
    pub frames: Vec<FrameCapture>,
    /// VOD / stream title (helps the model understand context).
    pub vod_title: String,
    /// Total VOD duration in seconds.
    pub duration_secs: f64,
    /// Human-readable batch label (e.g. "batch 2/5").
    pub batch_label: String,
    /// Optional context from other signals (audio spike times,
    /// transcript excerpts) to focus the model's attention.
    pub extra_context: String,
}

/// A single highlight detected by the vision model.
///
/// Provider adapters parse their API's response into this
/// common format.  The orchestrator then converts these into
/// [`SignalSegment`] entries.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VisionResult {
    /// Center timestamp of the detected moment.
    pub timestamp: f64,
    /// Suggested clip start time.
    pub clip_start: f64,
    /// Suggested clip end time.
    pub clip_end: f64,
    /// Short title describing the moment.
    pub title: String,
    /// Longer description of what happens and why it's interesting.
    pub description: String,
    /// Per-dimension scores from the model, each 0.0–1.0.
    pub scores: VisionDimensionScores,
    /// Weighted composite score (computed locally, not trusted from model).
    pub composite_score: f64,
    /// Comma-separated semantic tags.
    pub tags: String,
}

/// BYOK provider configuration.  Passed at call time — never persisted.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Which vision backend to use.
    pub provider: VisionProvider,
    /// The user's API key.
    pub api_key: String,
    /// Model identifier override.  `None` uses the provider's default.
    pub model: Option<String>,
}

impl ProviderConfig {
    pub fn new(provider: VisionProvider, api_key: String) -> Self {
        Self { provider, api_key, model: None }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Scoring weights (shared across providers)
// ═══════════════════════════════════════════════════════════════════

/// Weights for computing the composite score from dimension scores.
/// Matched to the scoring prompt sent to every provider.
const W_HOOK: f64 = 0.30;
const W_EMOTION: f64 = 0.25;
const W_PAYOFF: f64 = 0.20;
const W_LOOP: f64 = 0.15;
const W_CONTEXT: f64 = 0.10;

/// Compute the weighted composite from dimension scores.
/// We always recompute this locally — never trust the model's math.
fn composite(s: &VisionDimensionScores) -> f64 {
    (s.hook_strength * W_HOOK
        + s.emotional_spike * W_EMOTION
        + s.payoff_clarity * W_PAYOFF
        + s.loopability * W_LOOP
        + s.context_simplicity * W_CONTEXT)
        .min(0.99)
}

/// Minimum composite score to keep a model result.
const MIN_COMPOSITE: f64 = 0.45;

/// Maximum highlights accepted per batch.
const MAX_PER_BATCH: usize = 10;

// ═══════════════════════════════════════════════════════════════════
//  Trait: VisionAnalyzer
// ═══════════════════════════════════════════════════════════════════

/// Trait that every vision-model provider implements.
///
/// Returns a boxed future instead of using `async fn` in trait,
/// avoiding the `async_trait` proc-macro dependency.
///
/// Implementors handle:
///   1. Building the provider-specific HTTP request
///   2. Sending it and reading the response
///   3. Parsing the response into [`VisionResult`] entries
///
/// The orchestrator handles:
///   - Frame extraction (ffmpeg)
///   - Batching
///   - Converting [`VisionResult`] → [`SignalSegment`]
///   - Error fallback (API failure → continue with local signals)
pub trait VisionAnalyzer: Send + Sync {
    /// Human-readable provider name for logging.
    fn name(&self) -> &'static str;

    /// Which provider enum variant this implements.
    fn provider(&self) -> VisionProvider;

    /// Analyze a batch of frames and return detected highlights.
    ///
    /// # Errors
    ///
    /// Returns `AppError::Api` for network failures, auth errors,
    /// rate limits, or unparseable responses.
    fn analyze_batch<'a>(
        &'a self,
        batch: &'a FrameBatch,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<VisionResult>, AppError>> + Send + 'a>>;
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Create the appropriate analyzer for a provider config.
pub fn create_analyzer(config: &ProviderConfig) -> Box<dyn VisionAnalyzer> {
    match config.provider {
        VisionProvider::Claude => Box::new(ClaudeAnalyzer {
            api_key: config.api_key.clone(),
            model: config.model.clone().unwrap_or_else(|| "claude-sonnet-4-6".into()),
        }),
        VisionProvider::Gemini => Box::new(GeminiAnalyzer {
            api_key: config.api_key.clone(),
            model: config.model.clone().unwrap_or_else(|| "gemini-2.5-flash".into()),
        }),
    }
}

/// Analyze a batch of frames through any provider, returning pipeline-
/// compatible `SignalSegment` entries.
///
/// This is the only function the pipeline orchestrator needs to call.
/// It creates the analyzer, runs the batch, filters by minimum score,
/// and converts results to `SignalSegment`.
pub async fn analyze_frames(
    config: &ProviderConfig,
    batch: &FrameBatch,
) -> Result<Vec<SignalSegment>, AppError> {
    let analyzer = create_analyzer(config);

    log::info!(
        "Vision analysis via {} (model={}, {} frames, {})",
        analyzer.name(),
        match config.provider {
            VisionProvider::Claude => config.model.as_deref().unwrap_or("claude-sonnet-4-6"),
            VisionProvider::Gemini => config.model.as_deref().unwrap_or("gemini-2.5-flash"),
        },
        batch.frames.len(),
        batch.batch_label,
    );

    let mut results = analyzer.analyze_batch(batch).await?;

    // Recompute composite scores locally and filter
    for r in &mut results {
        r.composite_score = composite(&r.scores);
    }
    results.retain(|r| r.composite_score >= MIN_COMPOSITE);
    results.sort_by(|a, b| b.composite_score.partial_cmp(&a.composite_score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(MAX_PER_BATCH);

    let provider = analyzer.provider();
    Ok(results.into_iter().map(|r| result_to_segment(r, provider)).collect())
}

/// Convert a provider-agnostic `VisionResult` into a pipeline `SignalSegment`.
fn result_to_segment(r: VisionResult, provider: VisionProvider) -> SignalSegment {
    let tags: Vec<String> = r.tags
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    SignalSegment {
        signal_type: SignalType::Vision,
        start_time: r.clip_start,
        end_time: r.clip_end,
        score: r.composite_score,
        tags,
        metadata: Some(SignalMetadata::Vision {
            description: r.description,
            model_scores: r.scores,
            provider,
        }),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Shared: system prompt
// ═══════════════════════════════════════════════════════════════════

/// Build the analysis prompt sent to every vision provider.
///
/// Kept in one place so scoring criteria are identical regardless
/// of which model runs.
fn build_prompt(batch: &FrameBatch) -> String {
    format!(
        r#"You are an elite short-form content editor. Analyze these stream frames to find 15-45 second moments that will stop the scroll on TikTok/Shorts/Reels.

Stream: "{title}" | Duration: {dur:.0}s | {batch_label}

{ctx}

SCORE EACH DIMENSION 0.0-1.0:

HOOK_STRENGTH (30%): Does the first frame arrest attention?
- 0.9+: Instant chaos, visible screaming, explosion
- 0.7-0.9: Clear intense action creating "what happens next?"
- <0.5: Calm, needs context — weak hook

EMOTIONAL_SPIKE (25%): Peak emotional intensity?
- 0.9+: Streamer loses it — standing, screaming, head slam, uncontrollable laughter
- 0.7-0.9: Strong visible reaction — shock face, fist pump
- <0.5: Flat energy — weak moment

PAYOFF_CLARITY (20%): Can a viewer instantly understand what happened?
- 0.9+: Crystal clear outcome — epic fail, impossible shot
- 0.7-0.9: Clear outcome, satisfying resolution
- <0.5: Confusing, unclear

LOOPABILITY (15%): Would viewers replay or share?
- 0.9+: Perfect loop potential, viewers NEED to see it again
- 0.7-0.9: Share-worthy, ends on a high note

CONTEXT_SIMPLICITY (10%): Works without game knowledge?
- 0.9+: Pure universal comedy/reaction
- 0.7-0.9: "Person playing game does X" is enough

LOOK FOR: unhinged reactions, epic fails, clutch plays, jump scares, rage moments, genuine emotion.

TITLE: [What Happened] + [Payoff/Outcome]. Be specific, under 60 chars. BAD: "Epic Moment". GOOD: "Missed Swing Leads to Huge Punish".

Return ONLY this JSON:
{{"highlights":[{{"timestamp_seconds":<float>,"clip_start":<float>,"clip_end":<float>,"title":"...","description":"...","hook_strength":<float>,"emotional_spike":<float>,"payoff_clarity":<float>,"loopability":<float>,"context_simplicity":<float>,"tags":"comma,separated"}}]}}
If nothing qualifies: {{"highlights":[]}}"#,
        title = batch.vod_title,
        dur = batch.duration_secs,
        batch_label = batch.batch_label,
        ctx = batch.extra_context,
    )
}

/// Parse the model's JSON response into `VisionResult` entries.
///
/// Handles markdown code fences and extracts the first `{...}` block.
fn parse_model_response(text: &str) -> Result<Vec<VisionResult>, AppError> {
    // Strip markdown fences if present
    let json_str = if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            &text[start..=end]
        } else {
            text
        }
    } else {
        return Ok(Vec::new());
    };

    let parsed: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| AppError::Api(format!(
            "Failed to parse model JSON: {e} — raw: {}",
            &json_str[..json_str.len().min(200)]
        )))?;

    let highlights = parsed["highlights"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|h| {
                    let scores = VisionDimensionScores {
                        hook_strength: h["hook_strength"].as_f64().unwrap_or(0.5),
                        emotional_spike: h["emotional_spike"].as_f64().unwrap_or(0.5),
                        payoff_clarity: h["payoff_clarity"].as_f64().unwrap_or(0.5),
                        loopability: h["loopability"].as_f64().unwrap_or(0.5),
                        context_simplicity: h["context_simplicity"].as_f64().unwrap_or(0.5),
                    };

                    Some(VisionResult {
                        timestamp: h["timestamp_seconds"].as_f64()?,
                        clip_start: h["clip_start"].as_f64()?,
                        clip_end: h["clip_end"].as_f64()?,
                        title: h["title"].as_str()?.to_string(),
                        description: h["description"].as_str().unwrap_or("").to_string(),
                        composite_score: composite(&scores),
                        scores,
                        tags: h["tags"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(highlights)
}

// ═══════════════════════════════════════════════════════════════════
//  Provider: Claude (Anthropic Messages API)
// ═══════════════════════════════════════════════════════════════════

struct ClaudeAnalyzer {
    api_key: String,
    model: String,
}

impl VisionAnalyzer for ClaudeAnalyzer {
    fn name(&self) -> &'static str { "Claude Vision" }
    fn provider(&self) -> VisionProvider { VisionProvider::Claude }

    fn analyze_batch<'a>(
        &'a self,
        batch: &'a FrameBatch,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<VisionResult>, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let client = reqwest::Client::new();

            // Build multimodal content: interleaved images + timestamp labels
            let mut content: Vec<serde_json::Value> = Vec::new();
            for frame in &batch.frames {
                let b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &frame.jpeg_bytes,
                );
                content.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/jpeg",
                        "data": b64
                    }
                }));
                let mins = (frame.timestamp as u32) / 60;
                let secs = (frame.timestamp as u32) % 60;
                content.push(serde_json::json!({
                    "type": "text",
                    "text": format!("[Frame at {}:{:02} / {:.0}s]", mins, secs, frame.timestamp)
                }));
            }

            // Append the analysis prompt
            content.push(serde_json::json!({
                "type": "text",
                "text": build_prompt(batch)
            }));

            let body = serde_json::json!({
                "model": &self.model,
                "max_tokens": 8192,
                "messages": [{ "role": "user", "content": content }]
            });

            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::Api(format!("Claude request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("Claude API {status}: {body}")));
            }

            let resp_json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Api(format!("Claude response parse: {e}")))?;

            let text = resp_json["content"][0]["text"]
                .as_str()
                .ok_or_else(|| AppError::Api("No text in Claude response".into()))?;

            parse_model_response(text)
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Provider: Gemini (Google Generative AI API)
// ═══════════════════════════════════════════════════════════════════

struct GeminiAnalyzer {
    api_key: String,
    model: String,
}

impl VisionAnalyzer for GeminiAnalyzer {
    fn name(&self) -> &'static str { "Gemini Vision" }
    fn provider(&self) -> VisionProvider { VisionProvider::Gemini }

    fn analyze_batch<'a>(
        &'a self,
        batch: &'a FrameBatch,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<VisionResult>, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let client = reqwest::Client::new();

            // Build Gemini multimodal parts: images + text
            let mut parts: Vec<serde_json::Value> = Vec::new();
            for frame in &batch.frames {
                let b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &frame.jpeg_bytes,
                );
                parts.push(serde_json::json!({
                    "inlineData": {
                        "mimeType": "image/jpeg",
                        "data": b64
                    }
                }));
                let mins = (frame.timestamp as u32) / 60;
                let secs = (frame.timestamp as u32) % 60;
                parts.push(serde_json::json!({
                    "text": format!("[Frame at {}:{:02} / {:.0}s]", mins, secs, frame.timestamp)
                }));
            }

            parts.push(serde_json::json!({
                "text": build_prompt(batch)
            }));

            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                self.model
            );

            let body = serde_json::json!({
                "contents": [{ "parts": parts }],
                "generationConfig": {
                    "temperature": 0.3,
                    "maxOutputTokens": 8192,
                    "responseMimeType": "application/json"
                }
            });

            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .header("x-goog-api-key", &self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| AppError::Api(format!("Gemini request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("Gemini API {status}: {body}")));
            }

            let resp_json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Api(format!("Gemini response parse: {e}")))?;

            // Gemini nests text inside candidates[0].content.parts[0].text
            let text = resp_json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .ok_or_else(|| AppError::Api("No text in Gemini response".into()))?;

            parse_model_response(text)
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Composite scoring ──

    #[test]
    fn composite_weights_sum_to_one() {
        let sum = W_HOOK + W_EMOTION + W_PAYOFF + W_LOOP + W_CONTEXT;
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn composite_score_capped_at_099() {
        let perfect = VisionDimensionScores {
            hook_strength: 1.0,
            emotional_spike: 1.0,
            payoff_clarity: 1.0,
            loopability: 1.0,
            context_simplicity: 1.0,
        };
        assert!(composite(&perfect) <= 0.99);
    }

    #[test]
    fn composite_matches_manual_calculation() {
        let s = VisionDimensionScores {
            hook_strength: 0.8,
            emotional_spike: 0.7,
            payoff_clarity: 0.6,
            loopability: 0.5,
            context_simplicity: 0.9,
        };
        let expected = 0.8 * 0.30 + 0.7 * 0.25 + 0.6 * 0.20 + 0.5 * 0.15 + 0.9 * 0.10;
        assert!((composite(&s) - expected).abs() < 1e-9);
    }

    // ── Response parser ──

    #[test]
    fn parse_valid_json_response() {
        let json = r#"{"highlights":[{
            "timestamp_seconds": 120.0,
            "clip_start": 118.0,
            "clip_end": 140.0,
            "title": "Epic Fail at Bridge",
            "description": "Player falls off bridge",
            "hook_strength": 0.85,
            "emotional_spike": 0.90,
            "payoff_clarity": 0.75,
            "loopability": 0.60,
            "context_simplicity": 0.80,
            "tags": "fail,reaction,shock"
        }]}"#;

        let results = parse_model_response(json).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].timestamp - 120.0).abs() < 0.01);
        assert_eq!(results[0].title, "Epic Fail at Bridge");
        assert!((results[0].scores.hook_strength - 0.85).abs() < 0.01);
        assert!(results[0].composite_score > 0.0);
    }

    #[test]
    fn parse_markdown_wrapped_response() {
        let text = "Here are the highlights:\n```json\n{\"highlights\":[]}\n```\nDone.";
        let results = parse_model_response(text).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn parse_empty_highlights() {
        let json = r#"{"highlights":[]}"#;
        let results = parse_model_response(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn parse_no_json_returns_empty() {
        let results = parse_model_response("No highlights found.").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn parse_missing_required_fields_skips_entry() {
        let json = r#"{"highlights":[{
            "hook_strength": 0.9,
            "emotional_spike": 0.8
        }]}"#;
        // Missing timestamp_seconds, clip_start, clip_end, title → None from filter_map
        let results = parse_model_response(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn parse_defaults_missing_scores_to_05() {
        let json = r#"{"highlights":[{
            "timestamp_seconds": 10.0,
            "clip_start": 8.0,
            "clip_end": 25.0,
            "title": "Test Moment",
            "description": "test",
            "tags": "test"
        }]}"#;
        let results = parse_model_response(json).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].scores.hook_strength - 0.5).abs() < 0.01);
        assert!((results[0].scores.emotional_spike - 0.5).abs() < 0.01);
    }

    // ── Result → SignalSegment conversion ──

    #[test]
    fn result_converts_to_signal_segment() {
        let result = VisionResult {
            timestamp: 30.0,
            clip_start: 28.0,
            clip_end: 50.0,
            title: "Test".into(),
            description: "A test moment".into(),
            scores: VisionDimensionScores {
                hook_strength: 0.8,
                emotional_spike: 0.7,
                payoff_clarity: 0.6,
                loopability: 0.5,
                context_simplicity: 0.9,
            },
            composite_score: 0.72,
            tags: "reaction,shock".into(),
        };

        let seg = result_to_segment(result, VisionProvider::Claude);
        assert_eq!(seg.signal_type, SignalType::Vision);
        assert!((seg.start_time - 28.0).abs() < 0.01);
        assert!((seg.end_time - 50.0).abs() < 0.01);
        assert_eq!(seg.tags, vec!["reaction", "shock"]);

        match &seg.metadata {
            Some(SignalMetadata::Vision { description, model_scores, provider }) => {
                assert_eq!(description, "A test moment");
                assert!((model_scores.hook_strength - 0.8).abs() < 0.01);
                assert_eq!(*provider, VisionProvider::Claude);
            }
            other => panic!("Expected Vision metadata, got {:?}", other),
        }
    }

    #[test]
    fn empty_tags_handled() {
        let result = VisionResult {
            timestamp: 10.0,
            clip_start: 8.0,
            clip_end: 25.0,
            title: "T".into(),
            description: "D".into(),
            scores: VisionDimensionScores::default(),
            composite_score: 0.5,
            tags: "".into(),
        };
        let seg = result_to_segment(result, VisionProvider::Gemini);
        assert!(seg.tags.is_empty());
    }

    // ── Provider factory ──

    #[test]
    fn create_analyzer_claude() {
        let config = ProviderConfig::new(VisionProvider::Claude, "sk-test".into());
        let analyzer = create_analyzer(&config);
        assert_eq!(analyzer.name(), "Claude Vision");
        assert_eq!(analyzer.provider(), VisionProvider::Claude);
    }

    #[test]
    fn create_analyzer_gemini() {
        let config = ProviderConfig::new(VisionProvider::Gemini, "AIza-test".into());
        let analyzer = create_analyzer(&config);
        assert_eq!(analyzer.name(), "Gemini Vision");
        assert_eq!(analyzer.provider(), VisionProvider::Gemini);
    }

    #[test]
    fn provider_config_model_override() {
        let config = ProviderConfig::new(VisionProvider::Claude, "key".into())
            .with_model("claude-opus-4-6");
        assert_eq!(config.model.as_deref(), Some("claude-opus-4-6"));
    }

    // ── Build prompt ──

    #[test]
    fn prompt_contains_stream_title() {
        let batch = FrameBatch {
            frames: vec![],
            vod_title: "My Test Stream".into(),
            duration_secs: 3600.0,
            batch_label: "batch 1/1".into(),
            extra_context: "".into(),
        };
        let prompt = build_prompt(&batch);
        assert!(prompt.contains("My Test Stream"));
        assert!(prompt.contains("3600"));
        assert!(prompt.contains("HOOK_STRENGTH"));
        assert!(prompt.contains("highlights"));
    }
}
