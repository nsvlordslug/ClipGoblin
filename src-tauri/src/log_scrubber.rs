use regex::Regex;

/// Scrub sensitive patterns from log text before submitting as a bug report.
pub fn scrub_logs(input: &str) -> String {
    let mut output = input.to_string();

    // API keys (OpenAI sk-*, Anthropic key-*, Google AIza*)
    let api_key_re =
        Regex::new(r"(sk-[a-zA-Z0-9]{20,}|key-[a-zA-Z0-9]{20,}|AIza[a-zA-Z0-9_-]{30,})").unwrap();
    output = api_key_re
        .replace_all(&output, "[REDACTED_API_KEY]")
        .to_string();

    // Token values in key=value or key: value patterns
    let token_re = Regex::new(
        r#"(["']?(?:access_token|refresh_token|token)["']?\s*[:=]\s*["']?)([a-zA-Z0-9_.-]{20,})"#,
    )
    .unwrap();
    output = token_re
        .replace_all(&output, "${1}[REDACTED_TOKEN]")
        .to_string();

    // DPAPI-encrypted blobs
    let dpapi_re = Regex::new(r"dpapi:[A-Za-z0-9+/=]{10,}").unwrap();
    output = dpapi_re
        .replace_all(&output, "[REDACTED_ENCRYPTED]")
        .to_string();

    // Absolute Windows paths on every drive, plus UNC shares. Redact the rest
    // of the line as well because creator folder and media names can contain
    // spaces and arbitrary private text.
    let windows_path_re = Regex::new(r#"(?i)(?:[A-Z]:\\|\\\\[^\\\r\n]+\\)[^"\r\n]*"#).unwrap();
    output = windows_path_re
        .replace_all(&output, "[REDACTED_PATH]")
        .to_string();

    // Common absolute Unix/macOS data locations. Preserve the delimiter so
    // surrounding diagnostic text remains readable.
    let unix_path_re =
        Regex::new(r#"(?m)(^|[\s=:'"(])/(?:Users|home|mnt|media|var|tmp)/[^"\r\n]*"#).unwrap();
    output = unix_path_re
        .replace_all(&output, "${1}[REDACTED_PATH]")
        .to_string();

    // Relative media filenames still identify private content even when no
    // absolute path was logged. Drop that full line rather than attempting to
    // guess where a filename with spaces begins.
    let media_line_re =
        Regex::new(r"(?i)\.(?:mp4|mkv|mov|avi|webm|mp3|wav|m4a|flac|srt|vtt)(?:\b|$)").unwrap();
    output = output
        .lines()
        .map(|line| {
            if media_line_re.is_match(line) {
                "[REDACTED_MEDIA_PATH_LINE]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Named secrets (PROXY_API_KEY, client_secret, api_key)
    let b64_secret_re = Regex::new(
        r#"(["']?(?:PROXY_API_KEY|client_secret|api_key)["']?\s*[:=]\s*["']?)([A-Za-z0-9+/=]{16,})"#,
    )
    .unwrap();
    output = b64_secret_re
        .replace_all(&output, "${1}[REDACTED]")
        .to_string();

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrubs_api_key() {
        let input = "Using key sk-abcdefghijklmnopqrstuvwxyz123456";
        let output = scrub_logs(input);
        assert!(output.contains("[REDACTED_API_KEY]"));
        assert!(!output.contains("abcdefghij"));
    }

    #[test]
    fn test_scrubs_dpapi() {
        let input = "stored: dpapi:SGVsbG8gV29ybGQhIQ==";
        let output = scrub_logs(input);
        assert!(output.contains("[REDACTED_ENCRYPTED]"));
    }

    #[test]
    fn test_scrubs_custom_drive_and_unc_paths() {
        let input = "Downloaded VOD to E:\\Private Clips\\stream title.mp4\nOpened \\\\studio-nas\\captures\\secret.mov";
        let output = scrub_logs(input);
        assert!(!output.contains("Private Clips"));
        assert!(!output.contains("stream title"));
        assert!(!output.contains("studio-nas"));
        assert!(!output.contains("secret.mov"));
        assert!(output.contains("[REDACTED_PATH]"));
    }

    #[test]
    fn test_scrubs_unix_paths_and_relative_media_names() {
        let input = "Opened /home/creator/My Clips/private.mkv\nRendered secret-stream.mp4 successfully\nNormal retry message";
        let output = scrub_logs(input);
        assert!(!output.contains("creator"));
        assert!(!output.contains("secret-stream"));
        assert!(output.contains("Normal retry message"));
    }

    #[test]
    fn test_preserves_normal_diagnostic_text() {
        let input = "Upload retry 2 failed with status 503";
        assert_eq!(scrub_logs(input), input);
    }
}
