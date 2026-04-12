# Replace curl process spawns with reqwest HTTP calls
# Fixes CMD window flicker on Windows (accessibility issue)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding $false

# ── auth_proxy.rs ──────────────────────────────────────────────
$file = "src-tauri\src\auth_proxy.rs"
$lines = [System.IO.File]::ReadAllLines($file)

# Find the line with "let mut curl_cmd = tokio::process::Command::new"
$startIdx = -1
for ($i = 0; $i -lt $lines.Length; $i++) {
    if ($lines[$i].Trim().StartsWith('let mut curl_cmd = tokio::process::Command::new("curl")')) {
        $startIdx = $i
        break
    }
}
if ($startIdx -lt 0) {
    # Maybe already replaced? Check for the old single-line pattern
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if ($lines[$i].Trim().StartsWith('let output = tokio::process::Command::new("curl")')) {
            $startIdx = $i
            break
        }
    }
}

if ($startIdx -ge 0) {
    # Find the end: look for the line with "serde_json::from_str::<TokenResponse>"
    $endIdx = -1
    for ($i = $startIdx; $i -lt $lines.Length; $i++) {
        if ($lines[$i].Contains("serde_json::from_str::<TokenResponse>")) {
            # Include the next line (.map_err...)
            $endIdx = $i + 1
            break
        }
    }

    if ($endIdx -ge 0) {
        $replacement = @(
            '        let resp = self.client'
            '            .post(&url)'
            '            .header("X-Proxy-Key", &self.api_key)'
            '            .header("Content-Type", "application/json")'
            '            .body(body_str)'
            '            .send()'
            '            .await'
            '            .map_err(|e| format!("HTTP request failed: {e}"))?;'
            ''
            '        let status = resp.status();'
            '        let text = resp.text().await'
            '            .map_err(|e| format!("Failed to read response body: {e}"))?;'
            ''
            '        if !status.is_success() {'
            '            return Err(format!("Proxy request failed ({}): {}", status, text));'
            '        }'
            ''
            '        serde_json::from_str::<TokenResponse>(&text)'
            '            .map_err(|e| format!("Failed to parse proxy response: {e}"))'
        )

        $result = $lines[0..($startIdx - 1)] + $replacement + $lines[($endIdx + 1)..($lines.Length - 1)]
        [System.IO.File]::WriteAllLines($file, $result, $utf8)
        Write-Host "  [OK] auth_proxy.rs - replaced curl POST with reqwest"
    } else {
        Write-Host "  [SKIP] auth_proxy.rs - could not find end marker"
    }
} else {
    Write-Host "  [SKIP] auth_proxy.rs - curl_cmd not found (already replaced?)"
}

# ── twitch.rs ──────────────────────────────────────────────────
$file = "src-tauri\src\twitch.rs"
$lines = [System.IO.File]::ReadAllLines($file)

# Find "pub async fn curl_twitch_get"
$fnStart = -1
for ($i = 0; $i -lt $lines.Length; $i++) {
    if ($lines[$i].Contains("pub async fn curl_twitch_get")) {
        $fnStart = $i
        break
    }
}

if ($fnStart -ge 0) {
    # Find the closing brace of this function (next line that is just "}")
    $fnEnd = -1
    $braceDepth = 0
    for ($i = $fnStart; $i -lt $lines.Length; $i++) {
        foreach ($ch in $lines[$i].ToCharArray()) {
            if ($ch -eq '{') { $braceDepth++ }
            if ($ch -eq '}') { $braceDepth-- }
        }
        if ($braceDepth -eq 0 -and $i -gt $fnStart) {
            $fnEnd = $i
            break
        }
    }

    if ($fnEnd -ge 0) {
        $replacement = @(
            'pub async fn curl_twitch_get(url: &str, access_token: &str) -> Result<String, String> {'
            '    let client = reqwest::Client::builder()'
            '        .use_native_tls()'
            '        .http1_only()'
            '        .timeout(std::time::Duration::from_secs(15))'
            '        .build()'
            '        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;'
            ''
            '    let resp = client'
            '        .get(url)'
            '        .header("Client-Id", client_id())'
            '        .header("Authorization", format!("Bearer {}", access_token))'
            '        .send()'
            '        .await'
            '        .map_err(|e| format!("Failed to fetch: {}", e))?;'
            ''
            '    let status = resp.status();'
            '    let text = resp.text().await'
            '        .map_err(|e| format!("Failed to read response: {}", e))?;'
            ''
            '    if !status.is_success() {'
            '        return Err(format!("Twitch API request failed ({}): {}", status, text));'
            '    }'
            ''
            '    Ok(text)'
            '}'
        )

        $result = $lines[0..($fnStart - 1)] + $replacement + $lines[($fnEnd + 1)..($lines.Length - 1)]
        [System.IO.File]::WriteAllLines($file, $result, $utf8)
        Write-Host "  [OK] twitch.rs - replaced curl GET with reqwest"
    } else {
        Write-Host "  [SKIP] twitch.rs - could not find function end"
    }
} else {
    Write-Host "  [SKIP] twitch.rs - curl_twitch_get not found (already replaced?)"
}

Write-Host ""
Write-Host "Done. Test with:"
Write-Host "  cd src-tauri && cargo build 2>&1 | findstr /i 'error' && cd .."
Write-Host "If clean, build + install + test VOD fetch."
