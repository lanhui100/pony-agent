<#
.SYNOPSIS
  Validates the OpenSpec canonical spec library and any active changes.

.DESCRIPTION
  Read-only verification script. Runs the repository's OpenSpec CLI in strict mode.

  背景（PA-102）：canonical spec 曾在归档时只搬位置、不转格式，导致 39 / 53 份
  `openspec/specs/*/spec.md` 仍为 delta 格式（缺 `## Purpose` / `## Requirements`），
  而 strict 校验从未在 CI 中执行，漂移无人发现。本脚本把该门禁固化。

  覆盖：全部 canonical specs（openspec/specs/*）+ active changes + 归档 changes。

  Exit code 0: 全部通过。
  Exit code 1: 存在校验失败，或 CLI 调用本身异常。

  实现注意：OpenSpec CLI 会向 stderr 打印进度（"- Validating..."）。在 Windows
  PowerShell 5.1 下，native 命令的 stderr 会被提升为 ErrorRecord，配合
  `$ErrorActionPreference = 'Stop'` 会直接中断脚本——因此调用处必须临时降级为
  'Continue'。判据以解析出的 Totals 行为准，不依赖 stderr 文本。
#>

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path "$PSScriptRoot/.."
$Cli = Join-Path $RepoRoot "node_modules/@fission-ai/openspec/bin/openspec.js"

if (-not (Test-Path -LiteralPath $Cli)) {
  Write-Host "[check-openspec] ERROR: OpenSpec CLI not found at $Cli" -ForegroundColor Red
  Write-Host "[check-openspec] 请先运行 npm ci。" -ForegroundColor Red
  exit 1
}

Push-Location $RepoRoot
try {
  # native stderr 不得中断脚本（PS 5.1 行为差异，见文件头）
  $previous = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $raw = & node $Cli validate --all --strict 2>&1
  $exit = $LASTEXITCODE
  $ErrorActionPreference = $previous
} finally {
  Pop-Location
}

# 剔除 stderr 混入的非字符串记录，只保留 CLI 文本行
$stdout = @($raw | Where-Object { $_ -is [string] })

$totalsLine = ($stdout | Where-Object { $_ -match '^Totals:' } | Select-Object -Last 1)
$passed = 0
$failed = 0
$parsed = $false
if ($totalsLine -and $totalsLine -match 'Totals:\s*(\d+)\s+passed,\s*(\d+)\s+failed') {
  $passed = [int]$Matches[1]
  $failed = [int]$Matches[2]
  $parsed = $true
}

if (-not $parsed) {
  Write-Host "[check-openspec] ERROR: 无法解析 OpenSpec 校验输出（CLI 可能未正常运行）。" -ForegroundColor Red
  $stdout | Select-Object -Last 15 | ForEach-Object { Write-Host "  $_" -ForegroundColor DarkGray }
  exit 1
}

if ($failed -gt 0 -or $exit -ne 0) {
  Write-Host ""
  Write-Host "[check-openspec] OpenSpec 严格校验失败：$passed passed / $failed failed" -ForegroundColor Red
  Write-Host ""
  # 不逐项打印失败清单：OpenSpec 的 ✓/✗ 符号在不同控制台编码下会被转码，
  # StartsWith 匹配不可靠，宁可不打印也不打错（曾出现误列通过项）。
  # 需要明细时直接运行：npm run openspec -- validate --all --strict
  Write-Host "  失败明细：npm run openspec -- validate --all --strict" -ForegroundColor DarkGray
  Write-Host ""
  Write-Host "canonical spec 必须含 '## Purpose' 与 '## Requirements' 两个二级标题。" -ForegroundColor Cyan
  Write-Host "格式与判据见 management/task-system/03_TASKS/PA-102-canonical-spec-normalization-and-repo-hygiene.md。" -ForegroundColor Cyan
  exit 1
}

Write-Host "[check-openspec] PASSED: $passed specs valid (0 failed)." -ForegroundColor Green
exit 0
