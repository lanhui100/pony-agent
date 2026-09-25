<#
.SYNOPSIS
  Validates that all version files across the repository remain in sync.
.DESCRIPTION
  Read-only verification script. Checks:
  - Tauri desktop shell version matches across:
    1. package.json (.version)
    2. src-tauri/Cargo.toml ([package] version)
    3. src-tauri/tauri.conf.json (.version)
    4. .version.json (.tauri.major.minor.patch)
  - Core library version matches across:
    1. crates/pony-agent-core/Cargo.toml ([package] version)
    2. .version.json (.core.major.minor.patch)

  Exit code 0: all versions in sync.
  Exit code 1: mismatch detected.
#>

$ErrorActionPreference = "Stop"

$RepoRoot    = Resolve-Path "$PSScriptRoot/.."
$VersionFile = "$RepoRoot/.version.json"
$CoreCargo   = "$RepoRoot/crates/pony-agent-core/Cargo.toml"
$TauriCargo  = "$RepoRoot/src-tauri/Cargo.toml"
$PackageJson = "$RepoRoot/package.json"
$TauriConf   = "$RepoRoot/src-tauri/tauri.conf.json"

function Extract-CargoVersion($filePath) {
  if (-not (Test-Path -LiteralPath $filePath)) {
    throw "File not found: $filePath"
  }
  $content = Get-Content -Raw -LiteralPath $filePath
  if ($content -match '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"') {
    return $Matches[1]
  }
  throw "Could not extract version from $filePath"
}

$hasError = $false

# 1. Read .version.json baseline
if (-not (Test-Path -LiteralPath $VersionFile)) {
  Write-Host "[check-version-sync] ERROR: .version.json not found." -ForegroundColor Red
  exit 1
}
$state = Get-Content -Raw -LiteralPath $VersionFile | ConvertFrom-Json
$stateCoreVer  = "$($state.core.major).$($state.core.minor).$($state.core.patch)"
$stateTauriVer = "$($state.tauri.major).$($state.tauri.minor).$($state.tauri.patch)"

# 2. Check Core versions
$coreCargoVer = Extract-CargoVersion $CoreCargo
if ($coreCargoVer -ne $stateCoreVer) {
  Write-Host "[check-version-sync] Core mismatch: crates/pony-agent-core/Cargo.toml ($coreCargoVer) != .version.json ($stateCoreVer)" -ForegroundColor Red
  $hasError = $true
} else {
  Write-Host "[check-version-sync] Core version ok: $stateCoreVer" -ForegroundColor Green
}

# 3. Check Tauri / Frontend versions
$tauriCargoVer = Extract-CargoVersion $TauriCargo
$pkg = Get-Content -Raw -LiteralPath $PackageJson | ConvertFrom-Json
$pkgVer = $pkg.version

$conf = Get-Content -Raw -LiteralPath $TauriConf | ConvertFrom-Json
$confVer = $conf.version

$tauriVersions = @{
  ".version.json"                = $stateTauriVer
  "src-tauri/Cargo.toml"         = $tauriCargoVer
  "package.json"                 = $pkgVer
  "src-tauri/tauri.conf.json"    = $confVer
}

$uniqueTauri = $tauriVersions.Values | Select-Object -Unique
if ($uniqueTauri.Count -ne 1) {
  Write-Host "[check-version-sync] Tauri version mismatch across files:" -ForegroundColor Red
  foreach ($k in $tauriVersions.Keys) {
    Write-Host "  - ${k}: $($tauriVersions[$k])" -ForegroundColor Red
  }
  $hasError = $true
} else {
  Write-Host "[check-version-sync] Tauri version ok (4 places in sync): $stateTauriVer" -ForegroundColor Green
}

if ($hasError) {
  Write-Host "[check-version-sync] FAILED: Repository version files are out of sync." -ForegroundColor Red
  exit 1
}

Write-Host "[check-version-sync] PASSED: All version files are synchronized." -ForegroundColor Green
exit 0
