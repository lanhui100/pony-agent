param(
  [Parameter(Mandatory = $false)]
  [ValidateSet("pre-commit", "pre-push")]
  [string]$Mode = "pre-commit"
)

$ErrorActionPreference = "Stop"

$blockedPathPatterns = @(
  '^src-tauri/target(?:/|$)',
  '^src-tauri/target-codex-[^/]+(?:/|$)',
  '^src-tauri/target-test[^/]*(?:/|$)',
  '^src-tauri/target-check-tests[^/]*(?:/|$)',
  '^target(?:/|$)',
  '^target-codex-[^/]+(?:/|$)',
  '^target-check[^/]*(?:/|$)',
  '^target-test[^/]*(?:/|$)',
  '^coverage(?:/|$)',
  '^dist(?:/|$)',
  '^outputs(?:/|$)',
  '^playwright-report(?:/|$)',
  '^test-results(?:/|$)',
  '^tmp(?:/|$)',
  '^attachments(?:/|$)',
  '^sessions(?:/|$)'
)

$blockedExtensions = @(".rlib", ".lib", ".a", ".pdb")
$sizeLimitBytes = 20MB

function Get-StagedPaths {
  $output = git diff --cached --name-only --diff-filter=AM
  if (-not $output) {
    return @()
  }

  return $output |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_ -ne "" }
}

function Test-BlockedPath([string]$path) {
  $normalized = $path.Replace('\', '/')
  foreach ($pattern in $blockedPathPatterns) {
    if ($normalized -match $pattern) {
      return $true
    }
  }

  return $false
}

function Test-BlockedExtension([string]$path) {
  $extension = [System.IO.Path]::GetExtension($path)
  if ([string]::IsNullOrWhiteSpace($extension)) {
    return $false
  }

  return $blockedExtensions -contains $extension.ToLowerInvariant()
}

function Get-PathSize([string]$path) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    return 0
  }

  return (Get-Item -LiteralPath $path).Length
}

$stagedPaths = Get-StagedPaths
if ($stagedPaths.Count -eq 0) {
  exit 0
}

$violations = New-Object System.Collections.Generic.List[string]

foreach ($path in $stagedPaths) {
  if (Test-BlockedPath $path) {
    $violations.Add("禁止提交构建/运行产物目录: $path")
    continue
  }

  if (Test-BlockedExtension $path) {
    $violations.Add("禁止提交二进制构建产物: $path")
    continue
  }

  $size = Get-PathSize $path
  if ($size -gt $sizeLimitBytes) {
    $sizeMb = [math]::Round($size / 1MB, 2)
    $violations.Add("禁止提交超过 20MB 的文件: $path (${sizeMb}MB)")
  }
}

if ($violations.Count -gt 0) {
  Write-Host ""
  Write-Host "[pony-agent] Git guard 阻止了本次提交。" -ForegroundColor Red
  Write-Host "原因：" -ForegroundColor Yellow
  foreach ($item in $violations) {
    Write-Host "  - $item" -ForegroundColor Yellow
  }
  Write-Host ""
  Write-Host "建议：" -ForegroundColor Cyan
  Write-Host "  1. 把构建产物移出暂存区。"
  Write-Host "  2. 确认目标目录已被 .gitignore 覆盖。"
  Write-Host "  3. 如需保留真正的大资源，请单独评审其版本管理策略。"
  exit 1
}

# ---------------------------------------------------------------------------
# Version auto-bump (pre-commit only)
# ---------------------------------------------------------------------------
# When committing to core or tauri, auto-bump the patch version of the
# corresponding component so every commit carries a unique version number.
if ($Mode -eq "pre-commit") {
  $bumpScript = Join-Path $PSScriptRoot "bump-version.ps1"
  if (Test-Path -LiteralPath $bumpScript) {
    $hasCore  = ($stagedPaths | Where-Object { $_ -match '^crates/pony-agent-core/' }).Count -gt 0
    $hasTauri = ($stagedPaths | Where-Object { $_ -match '^src-tauri/' -or $_ -match '^src/' }).Count -gt 0

    if ($hasCore -or $hasTauri) {
      Write-Host ""
      Write-Host "[pony-agent] Version auto-bump: detecting changed components..." -ForegroundColor Cyan
      & $bumpScript -AutoDetect -Stage
      if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) {
        Write-Host "[pony-agent] Warning: version bump exited with code $LASTEXITCODE" -ForegroundColor Yellow
      }
    }
  }
}

exit 0
