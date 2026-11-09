$ErrorActionPreference = "Stop"

# 决策记录门禁的 Windows / pre-commit 入口。**判定真源在 scripts/check-decisions.mjs**，
# 本文件只做转发——一份判据两份实现必然漂移；且 pwsh 在 Linux 开发机上并不存在，
# 纯 ps1 实现等于本机没有这道门（2026-11-09 的 0107 目录状态漂移就是这么漏掉的）。
# 规则清单与退出码语义见 check-decisions.mjs 头注释。

$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) {
  Write-Host "[verify-decisions] 未找到 node，无法判定决策记录（fail-closed 拒绝提交）。" -ForegroundColor Red
  exit 1
}

& node (Join-Path $PSScriptRoot "check-decisions.mjs") @args
exit $LASTEXITCODE
