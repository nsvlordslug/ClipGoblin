//! Tauri commands for setting/clearing the per-VOD cam region, per-clip
//! override, fit mode, and the global allow-per-clip-override toggle.

use serde::Deserialize;
use tauri::State;

use crate::cam_region::{CamFitMode, CamRegion};
use crate::db;
use crate::DbConn;

#[derive(Debug, Deserialize)]
pub struct RegionInput {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RegionInput {
    fn to_region(&self) -> Option<CamRegion> {
        let r = CamRegion { x: self.x, y: self.y, w: self.w, h: self.h };
        // Round-trip through the parser to apply clamping + MIN_REGION_DIM rejection.
        CamRegion::parse_norm_json(&r.to_norm_json())
    }
}

/// Set the VOD-level cam region. Frontend passes the dragged rect as
/// `{ x, y, w, h }` in normalized 0..1 source-frame coords.
#[tauri::command]
pub async fn set_vod_cam_region(
    vod_id: String,
    region: RegionInput,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let r = region.to_region().ok_or_else(|| {
        "Region rejected: out of range or smaller than 5% x 5%".to_string()
    })?;
    let json = r.to_norm_json();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_vod_cam_region(&conn, &vod_id, Some(&json))
        .map_err(|e| format!("DB error: {e}"))
}

/// Clear the VOD-level cam region (NULL it out). Falls back to dup-source export.
#[tauri::command]
pub async fn clear_vod_cam_region(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_vod_cam_region(&conn, &vod_id, None)
        .map_err(|e| format!("DB error: {e}"))
}

/// Set a per-clip cam region override. Only honored when the
/// `allow_per_clip_cam_region_override` setting is true.
#[tauri::command]
pub async fn set_clip_cam_region_override(
    clip_id: String,
    region: RegionInput,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let r = region.to_region().ok_or_else(|| {
        "Region rejected: out of range or smaller than 5% x 5%".to_string()
    })?;
    let json = r.to_norm_json();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_cam_region_override(&conn, &clip_id, Some(&json))
        .map_err(|e| format!("DB error: {e}"))
}

/// Clear the per-clip override; clip will fall back to its VOD's region
/// (or dup-source if the VOD has no region either).
#[tauri::command]
pub async fn clear_clip_cam_region_override(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_cam_region_override(&conn, &clip_id, None)
        .map_err(|e| format!("DB error: {e}"))
}

/// Set the per-clip fit mode. Accepts 'fit', 'fill', or 'stretch'.
/// Unknown values are accepted-but-treated-as-fit at read time, so we don't
/// need to enforce here -- but we normalize via CamFitMode for consistency.
#[tauri::command]
pub async fn set_clip_fit_mode(
    clip_id: String,
    mode: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let normalized = CamFitMode::from_db(Some(&mode)).as_db_str();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_fit_mode(&conn, &clip_id, Some(normalized))
        .map_err(|e| format!("DB error: {e}"))
}

/// Toggle the global `allow_per_clip_cam_region_override` setting.
/// Stored in the existing `settings` k/v table as the string "true" or "false".
#[tauri::command]
pub async fn set_allow_per_clip_override(
    enabled: bool,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let val = if enabled { "true" } else { "false" };
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::save_setting(&conn, "allow_per_clip_cam_region_override", val)
        .map_err(|e| format!("DB error: {e}"))
}

/// Read the global setting. Frontend calls this once on Editor mount to know
/// whether to render the per-clip override sub-row.
#[tauri::command]
pub async fn get_allow_per_clip_override(
    db: State<'_, DbConn>,
) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    let val = db::get_setting(&conn, "allow_per_clip_cam_region_override")
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(matches!(val.as_deref(), Some("true")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_input_round_trips_through_clamp() {
        // Valid region passes through.
        let r = RegionInput { x: 0.1, y: 0.7, w: 0.25, h: 0.25 };
        assert!(r.to_region().is_some());
    }

    #[test]
    fn region_input_below_min_dim_returns_none() {
        // 4% width should be rejected.
        let r = RegionInput { x: 0.0, y: 0.0, w: 0.04, h: 0.5 };
        assert!(r.to_region().is_none());
    }

    #[test]
    fn region_input_out_of_range_clamps_then_rejects_if_too_small() {
        // Negative h clamps to 0 then rejects (below 5% min).
        let r = RegionInput { x: 0.0, y: 0.0, w: 0.5, h: -0.1 };
        assert!(r.to_region().is_none());
    }
}
