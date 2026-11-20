param(
  [switch]$Deep,
  [switch]$Incremental,
  [switch]$WithFrontend
)
# ── POSIX 落点转派（通用适配单一真源：scripts/lib/posix-dispatch.ps1）──
# 非 Windows 主机 → 交给 clean-targets.sh；Windows 原生路径不受影响。
. "$PSScriptRoot/lib/posix-dispatch.ps1"
$ponyForward = @()
if ($Deep) { $ponyForward += '--deep' }
if ($Incremental) { $ponyForward += '--incremental' }
if ($WithFrontend) { $ponyForward += '--with-frontend' }
Invoke-PonyPosixTwin -Name 'clean-targets' -ForwardArgs $ponyForward

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

# ── 增量档（-Incremental）：只删各 target 槽位下的 debug/release/incremental ───────────
# 理由与 Linux 版一致：213k 行单 crate 的增量目录实测可达 70G+；删它只损失下一次增量复用，
# deps/build 与 target/release 全部保留。与 -Deep 互斥（本档不删任何 rlib / 二进制）。
if ($Incremental) {
  $slots = Get-ChildItem $root -Directory -Filter "target*" -ErrorAction SilentlyContinue
  $freed = 0
  $removed = 0
  foreach ($slot in $slots) {
    foreach ($profile in @("debug", "release")) {
      $inc = Join-Path $slot.FullName (Join-Path $profile "incremental")
      if (-not (Test-Path -LiteralPath $inc)) { continue }
      $size = 0
      try {
        $size = (Get-ChildItem $inc -Recurse -File -ErrorAction SilentlyContinue |
          Measure-Object -Property Length -Sum).Sum
      } catch {}
      try {
        Remove-Item -LiteralPath $inc -Recurse -Force -ErrorAction Stop
        $freed += $size
        $removed++
        $rel = $inc.Substring($root.Length).TrimStart('\', '/')
        Write-Host ("[DEL ] {0,-45} {1,8:N1} MB" -f $rel, ($size / 1MB))
      } catch {
        Write-Host ("[SKIP] {0,-45} <- {1}" -f $inc, $_.Exception.Message)
      }
    }
  }
  if ($removed -eq 0) {
    Write-Host "No incremental directories found."
  } else {
    Write-Host ("--- Removed {0} incremental dirs, freed {1:N1} MB (deps kept) ---" -f $removed, ($freed / 1MB))
  }
  exit 0
}

# ── 前端产物档（-WithFrontend）：dist / coverage / playwright-report / test-results ────
if ($WithFrontend) {
  foreach ($name in @("dist", "coverage", "playwright-report", "test-results")) {
    $p = Join-Path $root $name
    if (-not (Test-Path -LiteralPath $p)) { continue }
    try {
      Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop
      Write-Host "[DEL ] $name"
    } catch {
      Write-Host ("[SKIP] {0} <- {1}" -f $name, $_.Exception.Message)
    }
  }
}

# Fixed-name target directories.
$fixedTargets = @(
  "target-check",
  "target-test",
  "target-perf",
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
