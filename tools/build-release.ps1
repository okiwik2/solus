# Builds the release installer: dist\SolusSetup-<version>.exe (+ latest.json when a signing key exists)
# usage: powershell -ExecutionPolicy Bypass -File tools\build-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Building Solus $version"

cargo build --release --bin solus --bin solus-sign
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# third-party notices: static text + every compiled Rust crate and its licence
$crates = cargo tree -e normal --prefix none --format '{p}|{l}' 2>$null | ForEach-Object { ($_ -replace ' \(\*\)', '') -replace ' \([^)]*\)', '' } | Sort-Object -Unique | Where-Object { $_ -notmatch '^solus ' }
$static = Get-Content -Raw -Encoding utf8 (Join-Path $root 'installer\notices-header.txt')
$lines = $crates | ForEach-Object { $p = $_.Split('|'); "  {0,-48} {1}" -f $p[0], $p[1] }
($static + "`r`n" + ($lines -join "`r`n") + "`r`n") | Out-File -Encoding utf8 (Join-Path $root 'THIRD-PARTY-NOTICES.txt')

$iscc = Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'
if (-not (Test-Path $iscc)) { $iscc = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe' }
& $iscc /Q "/DAppVersion=$version" (Join-Path $root 'installer\solus.iss')
if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
$setup = Join-Path $root "dist\SolusSetup-$version.exe"
Write-Host ("Installer: {0} ({1:N1} MB)" -f $setup, ((Get-Item $setup).Length / 1MB))

# update feed for GitHub Releases (signed with the release key, which never leaves this PC)
$key = Join-Path $env:APPDATA 'Solus Release\signing.key'
if (Test-Path $key) {
  $url = "https://github.com/okiwik2/solus/releases/download/v$version/SolusSetup-$version.exe"
  $manifest = (& (Join-Path $root 'target\release\solus-sign.exe') manifest $key $setup $version $url "Solus $version") -join "`n"
  if ($LASTEXITCODE -ne 0) { throw 'signing failed' }
  # no byte-order mark: Windows PowerShell's utf8 encoding adds one, which JSON parsers reject
  [IO.File]::WriteAllText((Join-Path $root 'dist\latest.json'), $manifest + "`n", (New-Object Text.UTF8Encoding $false))
  Write-Host 'Signed update manifest: dist\latest.json'
}
