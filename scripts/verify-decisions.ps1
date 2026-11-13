$ErrorActionPreference = "Stop"

# 决策记录门禁的 Windows / CI / release 入口。**判定真源永远是脚本而非本文件**：
#   - 语料仍在 `docs/archive/decisions`（旧布局）⇒ 转发 `scripts/check-decisions.mjs`（R1–R7）；
#   - 语料已迁移到 `.agents/notes`（2026-11-12 ADR 语料迁移）⇒ 转发 AGENTS.md 声明的判定真源
#     `.agents/skills/write-adr/verify-note.sh` + `.meta/gates/archive-verify.sh check`。
# 两条分支都是 fail-closed：语料与判定脚本都找不到 ⇒ 拒绝（exit 1），不做「找不到就算过」。
#
# 背景：`check-decisions.mjs` 校验的是「NNNN 编号 + README 索引」形态的旧语料；语料迁到
# `.agents/notes`（日期命名 + 无编号索引）后它必然报「未找到决策目录」，而这会同时红掉
# CI verify 与 release 的第一道门（实测 2026-11-12 release v0.1.191 即因此失败）。

$repoRoot = Split-Path -Parent $PSScriptRoot
$legacyCorpus = Join-Path $repoRoot "docs/archive/decisions"
$legacyCorpusAlt = Join-Path $repoRoot "docs/decisions"
$noteGate = Join-Path $repoRoot ".agents/skills/write-adr/verify-note.sh"
$sealGate = Join-Path $repoRoot ".meta/gates/archive-verify.sh"

Push-Location $repoRoot
try {
  if ((Test-Path $legacyCorpus) -or (Test-Path $legacyCorpusAlt)) {
    $node = Get-Command node -ErrorAction SilentlyContinue
    if (-not $node) {
      Write-Host "[verify-decisions] 未找到 node，无法判定决策记录（fail-closed 拒绝提交）。" -ForegroundColor Red
      exit 1
    }
    & node (Join-Path $PSScriptRoot "check-decisions.mjs") @args
    exit $LASTEXITCODE
  }

  if (-not (Test-Path $noteGate)) {
    Write-Host "[verify-decisions] 未找到任何决策语料判定入口：docs/archive/decisions 与 .agents/skills/write-adr/verify-note.sh 均不存在（fail-closed）。" -ForegroundColor Red
    exit 1
  }

  $bash = Get-Command bash -ErrorAction SilentlyContinue
  if (-not $bash) {
    Write-Host "[verify-decisions] 未找到 bash，无法运行 .agents/notes 语料判定（fail-closed）。" -ForegroundColor Red
    exit 1
  }

  & bash $noteGate
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  if (Test-Path $sealGate) {
    & bash $sealGate check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  }

  Write-Host "[verify-decisions] PASS（语料 .agents/notes：verify-note.sh + archive-verify.sh check）"
  exit 0
}
finally {
  Pop-Location
}
