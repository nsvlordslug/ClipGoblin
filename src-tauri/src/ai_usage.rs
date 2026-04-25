//! AI usage logging and cost estimation (Phase 6.0).
//!
//! Every LLM API call records a row in `ai_usage_log` with token counts and
//! computed cost. The frontend reads aggregated values from this table to
//! show per-VOD cost previews next to the BYOK toggles in Settings, and to
//! drive a pre-analyze confirmation modal.
//!
//! Pricing is hardcoded per provider+model. Real Anthropic / OpenAI / Gemini
//! pricing is published in $/M tokens and changes occasionally — keep the
//! constants here in one place so updates are a single edit.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::ai_provider::Provider;

/// One AI API call's recorded usage. Constructed at the call site after
/// the API responds, then handed to `log_usage` which writes the row and
/// computes cost.
pub struct UsageEntry<'a> {
    /// Stable feature label so reports can group by purpose.
    /// Examples: "title_regen" / "caption_regen" / "money_quote" / "title_save".
    pub feature: &'a str,
    pub provider: Provider,
    pub model: &'a str,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Optional VOD context (set when the call is triggered by an analyze).
    pub vod_id: Option<&'a str>,
    /// Optional clip context (set when the call targets a specific clip).
    pub clip_id: Option<&'a str>,
    /// Free-form metadata. JSON-encoded by the caller if structured.
    pub context: Option<&'a str>,
}

/// Cost in USD per 1K tokens for (input, output) at the given provider+model.
/// Returns a sensible default for unknown models so logging never fails.
///
/// Pricing snapshot 2026-04-24. Update when providers change rates.
pub fn token_cost_per_1k(provider: Provider, model: &str) -> (f64, f64) {
    let m = model.to_lowercase();
    match provider {
        Provider::Claude => {
            if m.contains("haiku") {
                (0.0008, 0.004)
            } else if m.contains("sonnet") {
                (0.003, 0.015)
            } else if m.contains("opus") {
                (0.015, 0.075)
            } else {
                (0.003, 0.015) // sensible default to Sonnet pricing
            }
        }
        Provider::OpenAI => {
            if m.contains("4o-mini") || m.contains("4-mini") {
                (0.00015, 0.0006)
            } else if m.contains("4o") {
                (0.0025, 0.01)
            } else if m.contains("o1") {
                (0.015, 0.06)
            } else {
                (0.0025, 0.01)
            }
        }
        Provider::Gemini => {
            if m.contains("flash") {
                (0.000075, 0.0003)
            } else if m.contains("pro") {
                (0.00125, 0.005)
            } else {
                (0.000075, 0.0003)
            }
        }
        Provider::Free => (0.0, 0.0),
    }
}

/// Compute USD cost for a call given token counts and provider+model.
pub fn compute_cost(provider: Provider, model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let (in_per_1k, out_per_1k) = token_cost_per_1k(provider, model);
    (tokens_in as f64 / 1000.0) * in_per_1k + (tokens_out as f64 / 1000.0) * out_per_1k
}

/// Insert a usage entry. Computes cost from the per-1K pricing table.
/// Returns the cost computed (so callers can log it inline if they want).
/// Errors are swallowed — a logging failure should never break a real
/// LLM call's user-visible behavior; just emit a warn-level log.
pub fn log_usage(conn: &Connection, entry: UsageEntry) -> f64 {
    let cost = compute_cost(entry.provider, entry.model, entry.tokens_in, entry.tokens_out);
    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().to_rfc3339();
    let provider_str = entry.provider.as_str();

    let result = conn.execute(
        "INSERT INTO ai_usage_log
            (id, timestamp, feature, provider, model, tokens_in, tokens_out, cost_usd, vod_id, clip_id, context)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            timestamp,
            entry.feature,
            provider_str,
            entry.model,
            entry.tokens_in as i64,
            entry.tokens_out as i64,
            cost,
            entry.vod_id,
            entry.clip_id,
            entry.context,
        ],
    );
    if let Err(e) = result {
        log::warn!("ai_usage::log_usage insert failed: {} (feature={})", e, entry.feature);
    }
    cost
}

/// Aggregated cost summary returned to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct CostSummary {
    /// Average cost per VOD analyze across the last N analyses (USD).
    /// 0.0 if no analyses are logged yet.
    pub avg_per_analyze_usd: f64,
    /// Total spent over the last 30 days (USD).
    pub total_30d_usd: f64,
    /// Number of distinct VOD analyses contributing to avg_per_analyze_usd.
    pub vod_count: u32,
}

/// Compute rolling-average cost per VOD analyze using the last
/// `lookback_vods` distinct VODs. Also returns the trailing-30-day total.
pub fn estimate_cost(conn: &Connection, lookback_vods: u32) -> CostSummary {
    // Per-VOD totals from the last N distinct VODs.
    let avg = conn
        .query_row(
            "SELECT AVG(vod_total) FROM (
                SELECT SUM(cost_usd) AS vod_total
                FROM ai_usage_log
                WHERE vod_id IS NOT NULL
                GROUP BY vod_id
                ORDER BY MAX(timestamp) DESC
                LIMIT ?1
            )",
            params![lookback_vods as i64],
            |row| row.get::<_, Option<f64>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0.0);

    let vod_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                SELECT vod_id FROM ai_usage_log
                WHERE vod_id IS NOT NULL
                GROUP BY vod_id
                ORDER BY MAX(timestamp) DESC
                LIMIT ?1
            )",
            params![lookback_vods as i64],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0) as u32;

    // 30-day total (all calls, vod-tagged or not).
    let thirty_days_ago = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let total_30d = conn
        .query_row(
            "SELECT SUM(cost_usd) FROM ai_usage_log WHERE timestamp >= ?1",
            params![thirty_days_ago],
            |row| row.get::<_, Option<f64>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0.0);

    CostSummary {
        avg_per_analyze_usd: avg,
        total_30d_usd: total_30d,
        vod_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_calc_haiku() {
        // 1k in + 1k out = 0.0008 + 0.004 = 0.0048
        let cost = compute_cost(Provider::Claude, "claude-haiku-4-5", 1000, 1000);
        assert!((cost - 0.0048).abs() < 1e-6);
    }

    #[test]
    fn cost_calc_sonnet() {
        // 1k in + 1k out = 0.003 + 0.015 = 0.018
        let cost = compute_cost(Provider::Claude, "claude-sonnet-4-6", 1000, 1000);
        assert!((cost - 0.018).abs() < 1e-6);
    }

    #[test]
    fn cost_calc_unknown_model_falls_back() {
        // Unknown model → Sonnet default
        let cost = compute_cost(Provider::Claude, "future-model-9000", 1000, 1000);
        assert!((cost - 0.018).abs() < 1e-6);
    }

    #[test]
    fn cost_calc_free_is_zero() {
        let cost = compute_cost(Provider::Free, "anything", 1_000_000, 1_000_000);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn empty_log_returns_zero_summary() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_log (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                feature TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL,
                tokens_out INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                vod_id TEXT,
                clip_id TEXT,
                context TEXT
            )",
        )
        .unwrap();
        let summary = estimate_cost(&conn, 10);
        assert_eq!(summary.avg_per_analyze_usd, 0.0);
        assert_eq!(summary.total_30d_usd, 0.0);
        assert_eq!(summary.vod_count, 0);
    }

    #[test]
    fn rolling_avg_groups_by_vod() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_usage_log (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                feature TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL,
                tokens_out INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                vod_id TEXT,
                clip_id TEXT,
                context TEXT
            )",
        )
        .unwrap();
        // VOD A: two calls totaling 0.10. VOD B: one call at 0.20.
        // Avg = (0.10 + 0.20) / 2 = 0.15.
        let now = chrono::Utc::now().to_rfc3339();
        for (vod, cost) in [("A", 0.04), ("A", 0.06), ("B", 0.20)] {
            conn.execute(
                "INSERT INTO ai_usage_log VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    now,
                    "title_regen",
                    "claude",
                    "claude-sonnet-4-6",
                    100i64,
                    100i64,
                    cost as f64,
                    vod,
                    Option::<&str>::None,
                    Option::<&str>::None,
                ],
            )
            .unwrap();
        }
        let summary = estimate_cost(&conn, 10);
        assert!((summary.avg_per_analyze_usd - 0.15).abs() < 1e-6);
        assert_eq!(summary.vod_count, 2);
    }
}
