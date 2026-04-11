param([string]$Version)
if (-not $Version) { Write-Host "Usage: bump-version.ps1 <version>"; exit 1 }
Write-Host "Bumping ClipGoblin to v$Version..."
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
# package.json
$pkg = [System.IO.File]::ReadAllText("$PSScriptRoot\package.json").TrimStart([char]0xFEFF)
$pkg = $pkg -replace '"version":\s*"[^"]*"', "`"version`": `"$Version`""
[System.IO.File]::WriteAllText("$PSScriptRoot\package.json", $pkg, $utf8NoBom)
Write-Host "  package.json -> $Version"
# tauri.conf.json
$tcj = [System.IO.File]::ReadAllText("$PSScriptRoot\src-tauri\tauri.conf.json").TrimStart([char]0xFEFF)
$tcj = $tcj -replace '"version":\s*"[^"]*"', "`"version`": `"$Version`""
[System.IO.File]::WriteAllText("$PSScriptRoot\src-tauri\tauri.conf.json", $tcj, $utf8NoBom)
Write-Host "  tauri.conf.json -> $Version"
# Cargo.toml (only the top-level version)
$cargo = [System.IO.File]::ReadAllText("$PSScriptRoot\src-tauri\Cargo.toml").TrimStart([char]0xFEFF)
$cargo = $cargo -replace '(?m)^version\s*=\s*"[^"]*"', "version = `"$Version`""
[System.IO.File]::WriteAllText("$PSScriptRoot\src-tauri\Cargo.toml", $cargo, $utf8NoBom)
Write-Host "  Cargo.toml -> $Version"
Write-Host "`nDone! All files updated to v$Version"
