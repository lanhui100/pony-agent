$ErrorActionPreference = "Stop"

# 决策记录门禁（L1 pre-commit 级，纯文本毫秒级检查）
# 校验三项：① Status 行存在性 ② 目录即状态 ③ superseded 指向有效
# 权威规范：docs/decisions/README.md；编写流程：.agents/skills/write-adr/SKILL.md

$decisionsDir = Join-Path $PSScriptRoot "..\docs\decisions"

if (-not (Test-Path -LiteralPath $decisionsDir)) {
  Write-Host "[verify-decisions] 未找到 docs/decisions 目录，跳过。" -ForegroundColor Yellow
  exit 0
}

$adrFiles = Get-ChildItem -LiteralPath $decisionsDir -Recurse -File -Filter *.md |
  Where-Object { $_.Name -ne "README.md" }

$violations = New-Object System.Collections.Generic.List[string]

foreach ($file in $adrFiles) {
  $rel = $file.FullName.Replace((Get-Location).Path + "\", "")

  # ① Status 行存在性（标题下前五行内）
  $statusLine = Get-Content -LiteralPath $file.FullName -TotalCount 5 |
    Where-Object { $_ -match '^Status:' } |
    Select-Object -First 1

  if (-not $statusLine) {
    $violations.Add("缺少 Status 行（标题下五行内）: $rel")
    continue
  }

  # ② 目录即状态：主目录必须 implemented，子目录必须与其同名状态匹配
  $dirName = Split-Path -Parent $file.FullName | Split-Path -Leaf
  if ($dirName -eq "decisions") {
    if ($statusLine -notmatch '^Status: implemented') {
      $violations.Add("主目录 ADR 必须为 implemented: $rel -> $statusLine")
    }
  }
  elseif ($statusLine -notmatch ("^Status: " + [regex]::Escape($dirName))) {
    $violations.Add("目录与状态不一致（目录即状态）: $rel -> $statusLine")
  }

  # ③ superseded 必须指向真实存在的 ADR 编号
  if ($statusLine -match 'superseded by\s*\[?(\d{4})\]?') {
    $targetId = $Matches[1]
    $targetExists = Get-ChildItem -LiteralPath $decisionsDir -Recurse -File -Filter "$targetId-*.md" |
      Where-Object { $_.Name -ne "README.md" }
    if (-not $targetExists) {
      $violations.Add("superseded by $targetId 指向不存在的 ADR: $rel")
    }
  }
}

if ($violations.Count -gt 0) {
  Write-Host ""
  Write-Host "[verify-decisions] 决策记录校验失败。" -ForegroundColor Red
  foreach ($item in $violations) {
    Write-Host "  - $item" -ForegroundColor Yellow
  }
  Write-Host ""
  Write-Host "规范见 docs/decisions/README.md；编写流程见 .agents/skills/write-adr/SKILL.md。" -ForegroundColor Cyan
  exit 1
}

exit 0
