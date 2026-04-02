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

#[derive(serde::Serialize)]
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

/// Send a Discord webhook notification that a user hit the rate limit.
async fn notify_rate_limit_hit(username: &str, user_id: &str) {
    let webhook_url = match std::env::var("DISCORD_WEBHOOK_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => return, // no webhook configured — silently skip
    };

    let payload = serde_json::json!({
        "embeds": [{
            "title": "Bug Report Rate Limit Hit",
            "description": format!(
                "User **{}** (ID: `{}`) has hit the 5 reports/day cap.",
                username, user_id
            ),
            "color": 16744448 // orange
        }]
    });

    let _ = reqwest::Client::new()
        .post(&webhook_url)
        .json(&payload)
        .send()
        .await;
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
        // Fire-and-forget Discord notification
        let uname = username.clone();
        let uid = user_id.clone();
        tokio::spawn(async move { notify_rate_limit_hit(&uname, &uid).await });

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

    // 5. Build issue body
    let body = format!(
        "## Bug Report (auto-submitted)\n\n\
         **Reporter:** {username} (`{user_id}`)\n\
         **Page:** {page}\n\
         **Severity:** {severity}\n\
         **App Version:** {version}\n\
         **OS:** {os} ({arch})\n\n\
         ### Description\n{description}\n\n\
         ### Steps to Reproduce\n{steps}\n\n\
         ### Expected Behavior\n{expected}\n\n\
         ### Recent Logs (scrubbed)\n\
         <details>\n<summary>Last 100 log lines</summary>\n\n\
         ```\n{logs}\n```\n\n\
         </details>",
        username = username,
        user_id = user_id,
        page = report.page,
        severity = report.severity,
        version = version,
        os = os,
        arch = arch,
        description = report.description,
        steps = report.steps,
        expected = report.expected,
        logs = log_tail,
    );

    // 6. POST to GitHub Issues API
    let gh_token = std::env::var("GITHUB_BUG_TOKEN").map_err(|_| {
        "GITHUB_BUG_TOKEN not configured — cannot submit bug report".to_string()
    })?;

    let severity_label = format!("severity:{}", report.severity.to_lowercase());
    let payload = serde_json::json!({
        "title": report.title,
        "body": body,
        "labels": ["bug", "auto-reported", severity_label],
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.github.com/repos/nsvlordslug/ClipGoblin/issues")
        .header("Authorization", format!("Bearer {}", gh_token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", format!("ClipGoblin/{}", version))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        log::error!("[BugReport] GitHub API error {}: {}", status, text);
        return Ok(BugReportResult {
            success: false,
            issue_url: None,
            error: Some(format!("GitHub API error ({})", status)),
        });
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))?;

    let issue_url = json["html_url"].as_str().unwrap_or("").to_string();

    // 7. Increment rate limit counter
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let new_count = (current_count + 1).to_string();
        db::save_setting(&conn, &rl_key, &new_count)
            .map_err(|e| format!("DB: {}", e))?;
    }

    log::info!("[BugReport] Submitted: {} → {}", report.title, issue_url);

    Ok(BugReportResult {
        success: true,
        issue_url: Some(issue_url),
        error: None,
    })
}
