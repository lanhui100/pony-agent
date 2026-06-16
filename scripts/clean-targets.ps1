param(
  [switch]$Deep
)

# Clean pony-agent workspace's disposable cargo target directories.
#
# Design notes:
# - dev uses the workspace-root target/, always kept (both light and deep skip it).
# - check / test use target-check / target-test, rebuildable, cleaned in light mode too.
# - codex / agent sessions historically created target-codex-* / target-test-session-* dirs
#   with semantic suffixes. We scan BOTH the project root and src-tauri/ with wildcards,
#   instead of a hardcoded list. (Bug fix: the old clean scripts only scanned src-tauri/
#   for target-codex-*, but those dirs actually live in the project root, so they were
#   never matched and accumulated to ~7GB.)
# - Deep mode additionally cleans target/ (main build dir; triggers full recompile).
# - Directories locked by a running process are skipped with a [SKIP] line, instead of
#   aborting mid-delete on a file lock.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Fixed-name target directories.
$fixedTargets = @(
  "target-check",
  "target-test",
  "target-test-exact-a",
  "target-test-exact-b",
  "target-test-exact-c",
  "target-check-tests",
  "target-check-tests-a",
  "target-check-tests-b",
  "target-check-tests-c",
  "target-check-tests-pa020-a",
  "target-check-tests-pa020-b"
)

if ($Deep) {
  $fixedTargets = @("target") + $fixedTargets
}

# Wildcard patterns: codex / agent one-shot session dirs (fix: scan project root, not only src-tauri).
$wildcardPatterns = @(
  "target-codex-*",
  "target-test-session-*",
  "target-test-exact-*",
  "target-check-tests-*"
)

$searchRoots = @($root, (Join-Path $root "src-tauri"))
$wildcardTargets = @()
foreach ($searchRoot in $searchRoots) {
  if (-not (Test-Path $searchRoot)) { continue }
  foreach ($pattern in $wildcardPatterns) {
    $wildcardTargets += Get-ChildItem $searchRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
      ForEach-Object { $_.FullName }
  }
}

$allTargets = ($fixedTargets + $wildcardTargets) | ForEach-Object {
  if ([System.IO.Path]::IsPathRooted($_)) { $_ } else { Join-Path $root $_ }
} | Sort-Object -Unique

$freed = 0
$deleted = 0
$skipped = 0

foreach ($t in $allTargets) {
  if (-not (Test-Path $t)) { continue }

  $size = 0
  try {
    $size = (Get-ChildItem $t -Recurse -File -ErrorAction SilentlyContinue |
      Measure-Object -Property Length -Sum).Sum
  } catch {}

  $rel = $t
  if ($t.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
    $rel = $t.Substring($root.Length).TrimStart('\', '/')
  }

  try {
    Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction Stop
    $freed += $size
    $deleted++
    Write-Host ("[DEL ] {0,-45} {1,8:N1} MB" -f $rel, ($size / 1MB))
  } catch {
    $skipped++
    Write-Host ("[SKIP] {0,-45} {1,8:N1} MB  <- {2}" -f $rel, ($size / 1MB), $_.Exception.Message)
  }
}

if ($deleted -eq 0 -and $skipped -eq 0) {
  Write-Host "No disposable target directories found."
} else {
  Write-Host ("--- Deleted {0} dirs, freed {1:N1} MB; skipped {2} (locked) ---" -f $deleted, ($freed / 1MB), $skipped)
}
