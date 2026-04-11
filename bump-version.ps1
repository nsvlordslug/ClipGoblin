param([Parameter(Mandatory)][string]$Version)
Write-Host "Bumping ClipGoblin to v$Version..." -ForegroundColor Cyan
# package.json
$pkg = Get-Content package.json -Raw | ConvertFrom-Json
$pkg.version = $Version
$pkg | ConvertTo-Json -Depth 10 | Set-Content package.json -Encoding UTF8
Write-Host "  package.json -> $Version" -ForegroundColor Green
# tauri.conf.json
$tc = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$tc.version = $Version
$tc | ConvertTo-Json -Depth 10 | Set-Content src-tauri/tauri.conf.json -Encoding UTF8
Write-Host "  tauri.conf.json -> $Version" -ForegroundColor Green
# Cargo.toml (regex replace just the package version line)
$cargo = Get-Content src-tauri/Cargo.toml -Raw
$cargo = $cargo -replace '(?m)^(version\s*=\s*")[\d.]+(")', "`${1}$Version`${2}"
Set-Content src-tauri/Cargo.toml $cargo -Encoding UTF8 -NoNewline
Write-Host "  Cargo.toml -> $Version" -ForegroundColor Green
Write-Host "`nDone! All files updated to v$Version" -ForegroundColor Cyan
