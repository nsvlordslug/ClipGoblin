use regex::Regex;

/// Scrub sensitive patterns from log text before submitting as a bug report.
pub fn scrub_logs(input: &str) -> String {
    let mut output = input.to_string();

    // API keys (OpenAI sk-*, Anthropic key-*, Google AIza*)
    let api_key_re = Regex::new(
        r"(sk-[a-zA-Z0-9]{20,}|key-[a-zA-Z0-9]{20,}|AIza[a-zA-Z0-9_-]{30,})",
    )
    .unwrap();
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

    // Windows user paths
    let user_path_re = Regex::new(r"C:\\Users\\[^\\]+").unwrap();
    output = user_path_re
        .replace_all(&output, r"C:\Users\[USER]")
        .to_string();

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
    fn test_preserves_normal_text() {
        let input = "Downloaded VOD to E:\\ClipGoblin\\vod123.mp4";
        let output = scrub_logs(input);
        assert_eq!(input, output);
    }
}
