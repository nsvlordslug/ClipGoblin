//! In-app bug reporter — creates GitHub Issues with system info + scrubbed logs.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tauri::State;

use crate::db;
use crate::log_scrubber;
use crate::DbConn;

// ── Types ──

#[derive(serde::Deserialize)]
pub struct BugReport {
    title: String,
    description: String,
    steps: String,
    expected: String,
    page: String,
    severity: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportResult {
    success: bool,
    issue_url: Option<String>,
    error: Option<String>,
}

// ── Helpers ──

/// Return the Tauri log directory (platform-specific).
/// tauri-plugin-log writes to {data_dir}/{bundle_id}/logs/
fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // %APPDATA%/com.clipgoblin.desktop/logs
        dirs::data_dir().map(|d| d.join("com.clipgoblin.desktop").join("logs"))
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|d| {
            d.join("Library/Logs/com.clipgoblin.desktop")
        })
    }
    #[cfg(target_os = "linux")]
    {
        dirs::data_dir().map(|d| d.join("com.clipgoblin.desktop").join("logs"))
    }
}

/// Read the last `n` lines from the most recent log file.
fn tail_latest_log(n: usize) -> String {
    let dir = match log_dir() {
        Some(d) if d.exists() => d,
        _ => return "(no log directory found)".to_string(),
    };

    // Find the most recently modified .log file
    let mut logs: Vec<_> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();

    logs.sort_by_key(|e| {
        std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok()))
    });

    let path = match logs.first() {
        Some(e) => e.path(),
        None => return "(no log files found)".to_string(),
    };

    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => return format!("(failed to open log: {})", e),
    };

    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
    let start = all_lines.len().saturating_sub(n);
    let tail = &all_lines[start..];

    log_scrubber::scrub_logs(&tail.join("\n"))
}

/// Get the rate limit counter key for today.
fn rate_limit_key(user_id: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    format!("bug_report_count_{}_{}", user_id, today)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BugReportPayload<'a> {
    title: &'a str,
    description: &'a str,
    steps: &'a str,
    expected: &'a str,
    page: &'a str,
    severity: &'a str,
    reporter_username: &'a str,
    reporter_user_id: &'a str,
    app_version: &'static str,
    os: &'static str,
    arch: &'static str,
    logs: &'a str,
}

// ── Command ──

#[tauri::command]
pub async fn submit_bug_report(
    report: BugReport,
    db: State<'_, DbConn>,
) -> Result<BugReportResult, String> {
    // 1. Get user info from DB
    let (user_id, username) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let uid = db::get_setting(&conn, "twitch_user_id")
            .map_err(|e| format!("DB: {}", e))?
            .unwrap_or_else(|| "anonymous".to_string());
        let uname = db::get_setting(&conn, "twitch_username")
            .map_err(|e| format!("DB: {}", e))?
            .unwrap_or_else(|| "unknown".to_string());
        (uid, uname)
    };

    // 2. Rate limit: 5 per user per day
    let rl_key = rate_limit_key(&user_id);
    let current_count: u32 = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, &rl_key)
            .map_err(|e| format!("DB: {}", e))?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };

    if current_count >= 5 {
        return Ok(BugReportResult {
            success: false,
            issue_url: None,
            error: Some("Rate limit reached (5 reports per day). Please try again tomorrow.".into()),
        });
    }

    // 3. System info
    let version = env!("CARGO_PKG_VERSION");
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    // 4. Scrubbed logs
    let log_tail = tail_latest_log(100);

    // 5. Submit through the Worker so release binaries never contain GitHub or
    // Discord credentials. The Worker independently validates and rate-limits.
    let payload = BugReportPayload {
        title: &report.title,
        description: &report.description,
        steps: &report.steps,
        expected: &report.expected,
        page: &report.page,
        severity: &report.severity,
        reporter_username: &username,
        reporter_user_id: &user_id,
        app_version: version,
        os,
        arch,
        logs: &log_tail,
    };
    let client = reqwest::Client::builder()
        .use_native_tls()
        .http1_only()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Failed to build bug report client: {e}"))?;
    let url = format!("{}/reports/bug", crate::auth_proxy::PROXY_BASE);
    let resp = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Bug report service request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let service_error = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown_error".to_string());
        log::error!(
            "[BugReport] Report service error {}: {}",
            status,
            service_error
        );
        return Ok(BugReportResult {
            success: false,
            issue_url: None,
            error: Some(format!("Bug report service error ({status})")),
        });
    }

    let result: BugReportResult = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse bug report response: {e}"))?;
    if !result.success {
        return Ok(result);
    }

    // 7. Increment rate limit counter
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let new_count = (current_count + 1).to_string();
        db::save_setting(&conn, &rl_key, &new_count)
            .map_err(|e| format!("DB: {}", e))?;
    }

    log::info!("[BugReport] Submitted: {}", report.title);

    Ok(result)
}
