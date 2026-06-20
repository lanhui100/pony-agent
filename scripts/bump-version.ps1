<#
.SYNOPSIS
  Bumps the development version for pony-agent core and/or tauri components.

.DESCRIPTION
  Manages independent semantic versions for the core library and tauri desktop shell.
  Reads from .version.json, bumps the requested segment, writes updated versions
  back to all relevant files (Cargo.toml, package.json, .version.json), and records
  the bump in the version history.

.PARAMETER Target
  Which component to bump: "core", "tauri", or "all" (default).

.PARAMETER Segment
  Which semver segment to bump: "patch" (default), "minor", or "major".

.PARAMETER AutoDetect
  If set, detect which component(s) to bump based on staged git files.
  When used, -Target is ignored.

.PARAMETER Message
  Optional commit message annotation for the version history log.

.PARAMETER DryRun
  Show what would be changed without writing any files.

.PARAMETER Stage
  If set, run `git add` on all modified version files after bumping.

.EXAMPLE
  # Bump core patch version
  .\scripts\bump-version.ps1 -Target core

.EXAMPLE
  # Bump both core and tauri minor version
  .\scripts\bump-version.ps1 -Target all -Segment minor

.EXAMPLE
  # Auto-detect from staged files and bump in dry-run mode
  .\scripts\bump-version.ps1 -AutoDetect -DryRun

.EXAMPLE
  # Bump tauri patch and git-stage all changed files
  .\scripts\bump-version.ps1 -Target tauri -Stage
#>

param(
  [ValidateSet("core", "tauri", "all")]
  [string]$Target = "all",

  [ValidateSet("patch", "minor", "major")]
  [string]$Segment = "patch",

  [switch]$AutoDetect,
  [string]$Message = "",
  [switch]$DryRun,
  [switch]$Stage
)

$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
$RepoRoot = Resolve-Path "$PSScriptRoot/.."
$VersionFile = "$RepoRoot/.version.json"
$CoreCargo   = "$RepoRoot/crates/pony-agent-core/Cargo.toml"
$TauriCargo  = "$RepoRoot/src-tauri/Cargo.toml"
$PackageJson = "$RepoRoot/package.json"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
function Get-VersionString($major, $minor, $patch) {
  return "$major.$minor.$patch"
}

function Format-Timestamp {
  return (Get-Date -Format "yyyy-MM-ddTHH:mm:ssK")
}

function Get-ShortCommit {
  $commit = git rev-parse --short HEAD 2>$null
  if (-not $commit) { $commit = "0000000" }
  return $commit
}

# ---------------------------------------------------------------------------
# Read current versions
# ---------------------------------------------------------------------------
if (-not (Test-Path -LiteralPath $VersionFile)) {
  Write-Error ".version.json not found at $VersionFile — run from repo root or init first."
  exit 1
}

$state = Get-Content -Raw -LiteralPath $VersionFile | ConvertFrom-Json

$coreVer    = $state.core
$tauriVer   = $state.tauri
$lastBump   = $state.lastBump

# Build a clean ArrayList for history — handles empty-array falsiness in PS
$history = [System.Collections.Generic.List[object]]::new()
if ($null -ne $state.history -and $state.history -is [array] -and $state.history.Count -gt 0) {
  foreach ($h in $state.history) {
    $history.Add($h) | Out-Null
  }
}

# ---------------------------------------------------------------------------
# Auto-detect which target(s) to bump
# ---------------------------------------------------------------------------
if ($AutoDetect) {
  $staged = git diff --cached --name-only --diff-filter=AM 2>$null
  if (-not $staged) {
    Write-Host "[bump-version] No staged files — nothing to auto-detect." -ForegroundColor Yellow
    exit 0
  }

  $stagedStr = ($staged | Out-String)
  $bumpCore  = $stagedStr -match 'crates/pony-agent-core/'
  $bumpTauri = ($stagedStr -match 'src-tauri/') -or ($stagedStr -match '^src/')

  if ($bumpCore -and $bumpTauri) {
    $Target = "all"
    Write-Host "[bump-version] Auto-detect: core + tauri files changed → bump all" -ForegroundColor Cyan
  }
  elseif ($bumpCore) {
    $Target = "core"
    Write-Host "[bump-version] Auto-detect: core files changed → bump core" -ForegroundColor Cyan
  }
  elseif ($bumpTauri) {
    $Target = "tauri"
    Write-Host "[bump-version] Auto-detect: tauri/frontend files changed → bump tauri" -ForegroundColor Cyan
  }
  else {
    Write-Host "[bump-version] No core/tauri files among staged changes — skipping bump." -ForegroundColor Yellow
    exit 0
  }
}

# ---------------------------------------------------------------------------
# Determine which components to bump
# ---------------------------------------------------------------------------
$bumpTargets = @()
if ($Target -eq "all") {
  $bumpTargets = @("core", "tauri")
}
else {
  $bumpTargets = @($Target)
}

# ---------------------------------------------------------------------------
# Bump versions
# ---------------------------------------------------------------------------
$changedFiles = @()
$bumpLog = @()

foreach ($comp in $bumpTargets) {
  $ver = if ($comp -eq "core") { $coreVer } else { $tauriVer }

  $oldMajor = [int]$ver.major
  $oldMinor = [int]$ver.minor
  $oldPatch = [int]$ver.patch
  $oldStr   = Get-VersionString $oldMajor $oldMinor $oldPatch

  switch ($Segment) {
    "major" {
      $newMajor = $oldMajor + 1
      $newMinor = 0
      $newPatch = 0
    }
    "minor" {
      $newMajor = $oldMajor
      $newMinor = $oldMinor + 1
      $newPatch = 0
    }
    "patch" {
      $newMajor = $oldMajor
      $newMinor = $oldMinor
      $newPatch = $oldPatch + 1
    }
  }

  $newStr = Get-VersionString $newMajor $newMinor $newPatch

  if ($comp -eq "core") {
    $state.core.major = $newMajor
    $state.core.minor = $newMinor
    $state.core.patch = $newPatch
    $coreVer = $state.core
  }
  else {
    $state.tauri.major = $newMajor
    $state.tauri.minor = $newMinor
    $state.tauri.patch = $newPatch
    $tauriVer = $state.tauri
  }

  $bumpLog += @{
    component = $comp
    from      = $oldStr
    to        = $newStr
    segment   = $Segment
  }

  Write-Host "  [$comp] $oldStr → $newStr ($Segment bump)" -ForegroundColor Green
}

$state.lastBump = Format-Timestamp

# Build version history entry
$historyEntry = @{
  timestamp = Format-Timestamp
  commit    = Get-ShortCommit
  bumps     = @($bumpLog)
}
if ($Message) {
  $historyEntry | Add-Member -NotePropertyName "message" -NotePropertyValue $Message
}
$history.Add($historyEntry) | Out-Null
$state.history = $history.ToArray()

if ($DryRun) {
  Write-Host ""
  Write-Host "[bump-version] DRY RUN — no files were written." -ForegroundColor Yellow
  Write-Host "  .version.json would be updated with above bumps."
  foreach ($comp in $bumpTargets) {
    $ver = if ($comp -eq "core") { $coreVer } else { $tauriVer }
    $verStr = Get-VersionString $ver.major $ver.minor $ver.patch
    Write-Host "  $($comp): $verStr"
  }
  exit 0
}

# ---------------------------------------------------------------------------
# Write updated version files
# ---------------------------------------------------------------------------

# 1) .version.json
$stateJson = $state | ConvertTo-Json -Depth 10
Set-Content -LiteralPath $VersionFile -Value $stateJson
$changedFiles += $VersionFile
Write-Host "[bump-version] Updated .version.json" -ForegroundColor DarkGray

# 2) Write to Cargo.toml files
function Update-CargoVersion($cargoPath, $comp, $major, $minor, $patch) {
  if (-not (Test-Path -LiteralPath $cargoPath)) {
    Write-Warning "Cargo.toml not found: $cargoPath — skipping $comp"
    return $false
  }

  $verStr = Get-VersionString $major $minor $patch
  $content = Get-Content -Raw -LiteralPath $cargoPath

  # Match: version = "X.Y.Z" in the [package] section
  if ($content -match '(?m)^version\s*=\s*"\d+\.\d+\.\d+"') {
    $newContent = $content -replace '(?m)^(version\s*=\s*)"\d+\.\d+\.\d+"', "`${1}`"$verStr"""
    Set-Content -LiteralPath $cargoPath -Value $newContent
    $script:changedFiles += $cargoPath
    Write-Host "[bump-version] Updated $cargoPath → $verStr" -ForegroundColor DarkGray
    return $true
  }
  else {
    Write-Warning "Could not find version field in $cargoPath — skipping $comp"
    return $false
  }
}

# Update core Cargo.toml
if ($bumpTargets -contains "core") {
  Update-CargoVersion $CoreCargo "core" $coreVer.major $coreVer.minor $coreVer.patch
}

# Update tauri Cargo.toml
if ($bumpTargets -contains "tauri") {
  Update-CargoVersion $TauriCargo "tauri" $tauriVer.major $tauriVer.minor $tauriVer.patch
}

# 3) Sync package.json version with tauri version
if ($bumpTargets -contains "tauri") {
  $tauriVerStr = Get-VersionString $tauriVer.major $tauriVer.minor $tauriVer.patch
  $pkgJson = Get-Content -Raw -LiteralPath $PackageJson | ConvertFrom-Json
  if ($pkgJson.version -ne $tauriVerStr) {
    $pkgJson.version = $tauriVerStr
    $pkgJsonStr = $pkgJson | ConvertTo-Json -Depth 10
    Set-Content -LiteralPath $PackageJson -Value $pkgJsonStr
    $changedFiles += $PackageJson
    Write-Host "[bump-version] Synced package.json → $tauriVerStr" -ForegroundColor DarkGray
  }
}

# ---------------------------------------------------------------------------
# Stage version files (optional)
# ---------------------------------------------------------------------------
if ($Stage -and $changedFiles.Count -gt 0) {
  foreach ($f in $changedFiles) {
    $rel = $f.Substring($RepoRoot.Path.Length).TrimStart('/', '\')
    git add $f 2>$null
    Write-Host "[bump-version] Staged: $rel" -ForegroundColor DarkGray
  }
}

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "╔══════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║         Version bump complete                ║" -ForegroundColor Cyan
Write-Host "╠══════════════════════════════════════════════╣" -ForegroundColor Cyan
foreach ($entry in $bumpLog) {
  $label = $entry.component.PadRight(6)
  Write-Host "║  $label  $($entry.from) → $($entry.to)  ║" -ForegroundColor Cyan
}
Write-Host "╚══════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""
Write-Host "Changed files:"
$repoPath = $RepoRoot.Path.TrimEnd('/', '\')
foreach ($f in $changedFiles) {
  $rel = $f
  if ($f.StartsWith($repoPath, [StringComparison]::OrdinalIgnoreCase)) {
    $rel = $f.Substring($repoPath.Length).TrimStart('/', '\')
  }
  Write-Host "  - $rel" -ForegroundColor Gray
}
Write-Host ""
