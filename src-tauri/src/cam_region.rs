//! Pure helpers for the per-VOD cam region feature.
//!
//! - `CamRegion` — normalized (0..1) source-frame rectangle, parsed from
//!   JSON stored in `vods.cam_region_norm` / `clips.cam_region_norm_override`.
//! - `CamFitMode` — how the cropped source region maps into the cam slot.
//! - `resolve_effective_region` — applies the override/VOD/setting precedence.
//! - `to_crop_expr` — formats a `CamRegion` into the ffmpeg `crop=...` argument.
//!
//! No DB, no IPC, no ffmpeg invocation — those live in `commands/cam_region.rs`
//! and `vertical_crop.rs` respectively. This module is colocated-tested.

use serde::{Deserialize, Serialize};

/// Minimum allowed region dimension (5% of source frame). Anything smaller is
/// rejected at parse time — prevents accidental zero-size crops from breaking
/// the ffmpeg filter graph.
pub const MIN_REGION_DIM: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CamRegion {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CamFitMode {
    Fit,
    Fill,
    Stretch,
}

impl Default for CamFitMode {
    fn default() -> Self {
        CamFitMode::Fit
    }
}

impl CamFitMode {
    /// Parse a DB string. NULL or unknown values default to Fit.
    pub fn from_db(s: Option<&str>) -> Self {
        match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("fill") => CamFitMode::Fill,
            Some("stretch") => CamFitMode::Stretch,
            _ => CamFitMode::Fit,
        }
    }

    /// Lowercase string for DB storage.
    pub fn as_db_str(self) -> &'static str {
        match self {
            CamFitMode::Fit => "fit",
            CamFitMode::Fill => "fill",
            CamFitMode::Stretch => "stretch",
        }
    }
}

impl CamRegion {
    /// Parse a JSON string like `{"x":0.12,"y":0.78,"w":0.22,"h":0.22}`.
    /// Clamps all four values to `[0.0, 1.0]`. Rejects regions where `w` or `h`
    /// is below `MIN_REGION_DIM` (returns None — caller should fall back to
    /// dup-source behavior).
    pub fn parse_norm_json(s: &str) -> Option<Self> {
        let mut r: CamRegion = serde_json::from_str(s).ok()?;
        r.x = r.x.clamp(0.0, 1.0);
        r.y = r.y.clamp(0.0, 1.0);
        r.w = r.w.clamp(0.0, 1.0);
        r.h = r.h.clamp(0.0, 1.0);
        if r.w < MIN_REGION_DIM || r.h < MIN_REGION_DIM {
            return None;
        }
        Some(r)
    }

    /// Serialize to the canonical JSON form for DB storage.
    pub fn to_norm_json(&self) -> String {
        // Hand-format to keep the JSON minimal and predictable (no scientific
        // notation, fixed 3-decimal precision). Easier to eyeball in the DB.
        format!(
            "{{\"x\":{:.3},\"y\":{:.3},\"w\":{:.3},\"h\":{:.3}}}",
            self.x, self.y, self.w, self.h
        )
    }

    /// Format the ffmpeg crop expression. Uses `iw`/`ih` so the resolver
    /// doesn't need to know the source resolution upfront.
    /// Example: `{x:0.12,y:0.78,w:0.22,h:0.22}` -> `"iw*0.22:ih*0.22:iw*0.12:ih*0.78"`.
    pub fn to_crop_expr(&self) -> String {
        format!(
            "iw*{:.4}:ih*{:.4}:iw*{:.4}:ih*{:.4}",
            self.w, self.h, self.x, self.y
        )
    }
}

/// Decide which region (if any) to use at export time.
///
/// Precedence:
/// 1. If `allow_override` is true AND `clip_override_json` parses, use it.
/// 2. Else if `vod_region_json` parses, use it.
/// 3. Else None (export falls back to dup-source).
///
/// Invalid JSON in either field is silently ignored (logged at the caller).
pub fn resolve_effective_region(
    vod_region_json: Option<&str>,
    clip_override_json: Option<&str>,
    allow_override: bool,
) -> Option<CamRegion> {
    if allow_override {
        if let Some(json) = clip_override_json {
            if let Some(r) = CamRegion::parse_norm_json(json) {
                return Some(r);
            }
        }
    }
    if let Some(json) = vod_region_json {
        return CamRegion::parse_norm_json(json);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- CamRegion::parse_norm_json --

    #[test]
    fn parse_valid_round_trips() {
        let r = CamRegion::parse_norm_json(r#"{"x":0.12,"y":0.78,"w":0.22,"h":0.22}"#).unwrap();
        assert_eq!(r, CamRegion { x: 0.12, y: 0.78, w: 0.22, h: 0.22 });
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(CamRegion::parse_norm_json("not json").is_none());
        assert!(CamRegion::parse_norm_json("{ ").is_none());
        assert!(CamRegion::parse_norm_json("[]").is_none());
    }

    #[test]
    fn parse_missing_field_returns_none() {
        assert!(CamRegion::parse_norm_json(r#"{"x":0.1,"y":0.1,"w":0.5}"#).is_none());
    }

    #[test]
    fn parse_clamps_positive_out_of_range() {
        let r = CamRegion::parse_norm_json(r#"{"x":1.5,"y":1.5,"w":0.5,"h":0.5}"#).unwrap();
        assert_eq!(r.x, 1.0);
        assert_eq!(r.y, 1.0);
    }

    #[test]
    fn parse_clamps_negative_then_min_check_rejects() {
        // Negative h clamps to 0.0, which is below MIN_REGION_DIM -> reject.
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.5,"h":-0.3}"#).is_none());
    }

    #[test]
    fn parse_rejects_below_min_dim() {
        // w just below 5%
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.04,"h":0.5}"#).is_none());
        // h just below 5%
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.5,"h":0.04}"#).is_none());
    }

    #[test]
    fn parse_accepts_exactly_min_dim() {
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.05,"h":0.05}"#).is_some());
    }

    // -- CamRegion::to_norm_json --

    #[test]
    fn to_norm_json_canonical_form() {
        let r = CamRegion { x: 0.123456, y: 0.789, w: 0.25, h: 0.25 };
        assert_eq!(r.to_norm_json(), r#"{"x":0.123,"y":0.789,"w":0.250,"h":0.250}"#);
    }

    #[test]
    fn to_norm_json_round_trips() {
        let original = CamRegion { x: 0.1, y: 0.7, w: 0.25, h: 0.25 };
        let serialized = original.to_norm_json();
        let parsed = CamRegion::parse_norm_json(&serialized).unwrap();
        assert_eq!(parsed, original);
    }

    // -- CamRegion::to_crop_expr --

    #[test]
    fn to_crop_expr_matches_spec_example() {
        let r = CamRegion { x: 0.12, y: 0.78, w: 0.22, h: 0.22 };
        assert_eq!(r.to_crop_expr(), "iw*0.2200:ih*0.2200:iw*0.1200:ih*0.7800");
    }

    #[test]
    fn to_crop_expr_uses_iw_ih_not_pixels() {
        let r = CamRegion { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
        let expr = r.to_crop_expr();
        assert!(expr.starts_with("iw*"), "must use iw multiplier: {expr}");
        assert!(expr.contains(":ih*"), "must use ih multiplier: {expr}");
    }

    // -- CamFitMode --

    #[test]
    fn cam_fit_mode_from_db_defaults_to_fit() {
        assert_eq!(CamFitMode::from_db(None), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("")), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("xyz")), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("fit")), CamFitMode::Fit);
    }

    #[test]
    fn cam_fit_mode_from_db_parses_fill_stretch() {
        assert_eq!(CamFitMode::from_db(Some("fill")), CamFitMode::Fill);
        assert_eq!(CamFitMode::from_db(Some("STRETCH")), CamFitMode::Stretch);
        assert_eq!(CamFitMode::from_db(Some("  Fill  ")), CamFitMode::Fill);
    }

    #[test]
    fn cam_fit_mode_db_str_round_trips() {
        for m in [CamFitMode::Fit, CamFitMode::Fill, CamFitMode::Stretch] {
            assert_eq!(CamFitMode::from_db(Some(m.as_db_str())), m);
        }
    }

    // -- resolve_effective_region --

    const SAMPLE_JSON_VOD: &str = r#"{"x":0.1,"y":0.7,"w":0.25,"h":0.25}"#;
    const SAMPLE_JSON_OVERRIDE: &str = r#"{"x":0.5,"y":0.5,"w":0.20,"h":0.20}"#;

    #[test]
    fn resolve_uses_vod_when_no_override() {
        let r = resolve_effective_region(Some(SAMPLE_JSON_VOD), None, true).unwrap();
        assert_eq!(r.x, 0.1);
    }

    #[test]
    fn resolve_uses_override_when_setting_on_and_override_set() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some(SAMPLE_JSON_OVERRIDE),
            true,
        ).unwrap();
        assert_eq!(r.x, 0.5, "override should win");
    }

    #[test]
    fn resolve_ignores_override_when_setting_off() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some(SAMPLE_JSON_OVERRIDE),
            false,
        ).unwrap();
        assert_eq!(r.x, 0.1, "override must be ignored when toggle off");
    }

    #[test]
    fn resolve_returns_none_when_nothing_set() {
        assert!(resolve_effective_region(None, None, true).is_none());
        assert!(resolve_effective_region(None, None, false).is_none());
        assert!(resolve_effective_region(None, Some(SAMPLE_JSON_OVERRIDE), false).is_none());
    }

    #[test]
    fn resolve_falls_back_to_vod_when_override_invalid() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some("garbage"),
            true,
        ).unwrap();
        assert_eq!(r.x, 0.1);
    }
}
