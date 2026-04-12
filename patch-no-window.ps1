# patch-no-window.ps1 — Run from clipviral project root
# powershell -ExecutionPolicy Bypass -File patch-no-window.ps1
$ErrorActionPreference = "Stop"
$enc = New-Object System.Text.UTF8Encoding $false

function Replace-Lines {
    param([string]$file, [int]$startLine, [int]$endLine, [string[]]$newLines, [string]$label)
    $lines = [System.IO.File]::ReadAllLines($file)
    $before = if ($startLine -gt 1) { $lines[0..($startLine-2)] } else { @() }
    $after = $lines[$endLine..($lines.Length-1)]
    $result = $before + $newLines + $after
    [System.IO.File]::WriteAllLines($file, $result, $enc)
    Write-Host "  [OK] $label (lines $startLine-$endLine)" -ForegroundColor Green
}

Write-Host "`nPatching CREATE_NO_WINDOW flags...`n" -ForegroundColor Cyan

# === 1. hardware.rs lines 45-47 ===
# Old: let output = match Command::new("nvidia-smi")
#          .args([...])
#          .output()
Replace-Lines "src-tauri\src\hardware.rs" 45 47 @(
    '    let mut smi_cmd = Command::new("nvidia-smi");'
    '    smi_cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]);'
    '    #[cfg(target_os = "windows")]'
    '    {'
    '        use std::os::windows::process::CommandExt;'
    '        smi_cmd.creation_flags(0x08000000);'
    '    }'
    '    let output = match smi_cmd.output()'
) "hardware.rs - nvidia-smi"

# === 2. whisper.rs line 130 ===
# Old: if let Ok(output) = Command::new("ffmpeg").arg("-version").output() {
Replace-Lines "src-tauri\src\whisper.rs" 130 130 @(
    '    let mut ver_cmd = Command::new("ffmpeg");'
    '    ver_cmd.arg("-version");'
    '    #[cfg(target_os = "windows")]'
    '    {'
    '        use std::os::windows::process::CommandExt;'
    '        ver_cmd.creation_flags(0x08000000);'
    '    }'
    '    if let Ok(output) = ver_cmd.output() {'
) "whisper.rs - ffmpeg version check"

# === 3. whisper.rs line 143 (now shifted to ~150 after insert above) ===
# Old: let mut child = Command::new(ffmpeg)
#          .args([...])
#          .stdin(Stdio::null())
#          .stdout(Stdio::piped())
#          .stderr(Stdio::null())
#          .spawn()
# Find the actual line dynamically
$wLines = [System.IO.File]::ReadAllLines("src-tauri\src\whisper.rs")
$childStart = -1
$spawnEnd = -1
for ($i = 140; $i -lt $wLines.Length; $i++) {
    if ($wLines[$i] -match '^\s*let mut child = Command::new\(ffmpeg\)') {
        $childStart = $i + 1
    }
    if ($childStart -gt 0 -and $wLines[$i] -match '\.spawn\(\)') {
        $spawnEnd = $i + 1
        break
    }
}
if ($childStart -gt 0 -and $spawnEnd -gt 0) {
    Replace-Lines "src-tauri\src\whisper.rs" $childStart $spawnEnd @(
        '    let mut child_cmd = Command::new(ffmpeg);'
        '    child_cmd.args(['
        '        "-i", audio_path,'
        '        "-ar", "16000",'
        '        "-ac", "1",'
        '        "-f", "f32le",'
        '        "-acodec", "pcm_f32le",'
        '        "pipe:1",'
        '    ])'
        '    .stdin(Stdio::null())'
        '    .stdout(Stdio::piped())'
        '    .stderr(Stdio::null());'
        '    #[cfg(target_os = "windows")]'
        '    {'
        '        use std::os::windows::process::CommandExt;'
        '        child_cmd.creation_flags(0x08000000);'
        '    }'
        '    let mut child = child_cmd.spawn()'
    ) "whisper.rs - ffmpeg child"
} else {
    Write-Host "  [SKIP] whisper.rs ffmpeg child - pattern not found (start=$childStart, end=$spawnEnd)" -ForegroundColor Yellow
}

# === 4. vod.rs line 78 ===
# Old: if let Ok(output) = std::process::Command::new("yt-dlp").arg("--version").output() {
Replace-Lines "src-tauri\src\commands\vod.rs" 78 78 @(
    '    let mut yt_check = std::process::Command::new("yt-dlp");'
    '    yt_check.arg("--version");'
    '    #[cfg(target_os = "windows")]'
    '    {'
    '        use std::os::windows::process::CommandExt;'
    '        yt_check.creation_flags(0x08000000);'
    '    }'
    '    if let Ok(output) = yt_check.output() {'
) "vod.rs - yt-dlp version check"

# === 5. vod.rs python check (find dynamically since lines shifted) ===
$vLines = [System.IO.File]::ReadAllLines("src-tauri\src\commands\vod.rs")
$pyStart = -1
$pyEnd = -1
for ($i = 650; $i -lt $vLines.Length; $i++) {
    if ($vLines[$i] -match 'if let Ok\(check\) = std::process::Command::new\(&python\)') {
        $pyStart = $i + 1
    }
    if ($pyStart -gt 0 -and $vLines[$i] -match '\.output\(\)') {
        $pyEnd = $i + 1
        break
    }
}
if ($pyStart -gt 0 -and $pyEnd -gt 0) {
    Replace-Lines "src-tauri\src\commands\vod.rs" $pyStart $pyEnd @(
        '    let mut py_cmd = std::process::Command::new(&python);'
        '    py_cmd.args(["-c", "import faster_whisper; print(faster_whisper.__version__)"]);'
        '    py_cmd.env("CUDA_VISIBLE_DEVICES", "");'
        '    #[cfg(target_os = "windows")]'
        '    {'
        '        use std::os::windows::process::CommandExt;'
        '        py_cmd.creation_flags(0x08000000);'
        '    }'
        '    if let Ok(check) = py_cmd.output()'
    ) "vod.rs - python check"
} else {
    Write-Host "  [SKIP] vod.rs python check - not found (start=$pyStart, end=$pyEnd)" -ForegroundColor Yellow
}

# === 6. auth_proxy.rs curl ===
$apLines = [System.IO.File]::ReadAllLines("src-tauri\src\auth_proxy.rs")
$curlStart = -1
$curlEnd = -1
for ($i = 130; $i -lt $apLines.Length; $i++) {
    if ($apLines[$i] -match 'let output = tokio::process::Command::new\("curl"\)') {
        $curlStart = $i + 1
    }
    if ($curlStart -gt 0 -and $apLines[$i] -match '^\s*\.output\(\)') {
        $curlEnd = $i + 1
        break
    }
}
if ($curlStart -gt 0 -and $curlEnd -gt 0) {
    Replace-Lines "src-tauri\src\auth_proxy.rs" $curlStart $curlEnd @(
        '        let mut curl_cmd = tokio::process::Command::new("curl");'
        '        curl_cmd.args(['
        '            "-s", "-S",'
        '            "--max-time", "15",'
        '            "-X", "POST",'
        '            "-H", &format!("X-Proxy-Key: {}", self.api_key),'
        '            "-H", "Content-Type: application/json",'
        '            "-d", &body_str,'
        '            &url,'
        '        ]);'
        '        #[cfg(target_os = "windows")]'
        '        {'
        '            use std::os::windows::process::CommandExt;'
        '            curl_cmd.creation_flags(0x08000000);'
        '        }'
        '        let output = curl_cmd.output()'
        '            .await'
    ) "auth_proxy.rs - curl"
} else {
    Write-Host "  [SKIP] auth_proxy.rs curl - not found (start=$curlStart, end=$curlEnd)" -ForegroundColor Yellow
}

# === 7. twitch.rs curl ===
$twLines = [System.IO.File]::ReadAllLines("src-tauri\src\twitch.rs")
$curlStart2 = -1
$curlEnd2 = -1
for ($i = 310; $i -lt $twLines.Length; $i++) {
    if ($twLines[$i] -match 'let output = tokio::process::Command::new\("curl"\)') {
        $curlStart2 = $i + 1
    }
    if ($curlStart2 -gt 0 -and $twLines[$i] -match '^\s*\.output\(\)') {
        $curlEnd2 = $i + 1
        break
    }
}
if ($curlStart2 -gt 0 -and $curlEnd2 -gt 0) {
    Replace-Lines "src-tauri\src\twitch.rs" $curlStart2 $curlEnd2 @(
        '    let mut curl_cmd = tokio::process::Command::new("curl");'
        '    curl_cmd.args(['
        '        "-s", "-S", "--max-time", "15",'
        '        "-H", &format!("Client-Id: {}", client_id()),'
        '        "-H", &format!("Authorization: Bearer {}", access_token),'
        '        url,'
        '    ]);'
        '    #[cfg(target_os = "windows")]'
        '    {'
        '        use std::os::windows::process::CommandExt;'
        '        curl_cmd.creation_flags(0x08000000);'
        '    }'
        '    let output = curl_cmd.output()'
        '        .await'
    ) "twitch.rs - curl"
} else {
    Write-Host "  [SKIP] twitch.rs curl - not found (start=$curlStart2, end=$curlEnd2)" -ForegroundColor Yellow
}

Write-Host "`nAll patches applied. Test with:" -ForegroundColor Cyan
Write-Host "  cargo build 2>&1 | findstr /i `"error`"" -ForegroundColor White
Write-Host "If clean, commit:" -ForegroundColor Cyan
Write-Host "  git add -A && git commit -m `"fix: add CREATE_NO_WINDOW to all process spawns (accessibility)`"" -ForegroundColor White
