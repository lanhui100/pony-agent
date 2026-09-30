<#
.SYNOPSIS
  Normalizes Tauri-generated schema files to the repository's text conventions.

.DESCRIPTION
  背景（PA-102 §2）：`tauri dev` / `tauri build` 每次运行都会重新生成
  src-tauri/gen/schemas/*.json，且**不写结尾换行**；仓库版本带结尾换行，
  于是工作树恒定出现 4 个"已修改"文件，diff 内容为空
  （只有 `\ No newline at end of file`）。

  决策：这几个文件是 Tauri 的**权限契约**（capabilities / ACL manifest /
  平台 schema）——它们随权限面变更而变更，评审价值高于噪音成本，因此保留
  跟踪（不 gitignore），改为在生成之后统一补齐文本约定：
  - 行尾统一 LF（对齐 .gitattributes 的 `*.json text eol=lf`）；
  - 结尾恰好一个换行。

  本脚本只补文本约定，不改动 JSON 语义；处理前后会比对内容，输出是否发生变更。

.PARAMETER Watch
  以 FileSystemWatcher 常驻监听该目录，等 `tauri dev` 写入后自动规范化，
  从而在整个 dev 会话期间保持工作树干净。Ctrl+C 结束。

.PARAMETER Quiet
  无变更时不输出（供 dev 流程内联调用，减少噪音）。

.EXAMPLE
  pwsh -File scripts/normalize-generated-schemas.ps1
.EXAMPLE
  pwsh -File scripts/normalize-generated-schemas.ps1 -Watch
#>
[CmdletBinding()]
param(
  [switch]$Watch,
  [switch]$Quiet
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path "$PSScriptRoot/.."
$SchemaDir = Join-Path $RepoRoot "src-tauri/gen/schemas"

if (-not (Test-Path -LiteralPath $SchemaDir)) {
  if (-not $Quiet) { Write-Host "[normalize-schemas] 目录不存在，跳过：$SchemaDir" -ForegroundColor DarkGray }
  exit 0
}

function Invoke-NormalizeOnce {
  param([string]$Directory, [switch]$Silent)

  $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
  $changed = @()
  $files = @(Get-ChildItem -LiteralPath $Directory -File -Filter '*.json' -ErrorAction SilentlyContinue)

  foreach ($file in $files) {
    # 可能正被 Tauri 写入：短暂重试，避免读到半截内容
    $text = $null
    for ($attempt = 0; $attempt -lt 5; $attempt++) {
      try {
        $text = [System.IO.File]::ReadAllText($file.FullName)
        break
      } catch {
        Start-Sleep -Milliseconds 120
      }
    }
    if ($null -eq $text) { continue }

    $normalized = ($text -replace "`r`n", "`n" -replace "`r", "`n").TrimEnd("`n") + "`n"
    if ($normalized -ne $text) {
      for ($attempt = 0; $attempt -lt 5; $attempt++) {
        try {
          [System.IO.File]::WriteAllText($file.FullName, $normalized, $utf8NoBom)
          $changed += $file.Name
          break
        } catch {
          Start-Sleep -Milliseconds 120
        }
      }
    }
  }

  if ($changed.Count -gt 0 -and -not $Silent) {
    Write-Host "[normalize-schemas] 已规范化 $($changed.Count) 个文件：$($changed -join ', ')" -ForegroundColor Green
  }
  return $changed.Count
}

if ($Watch) {
  Write-Host "[normalize-schemas] 监听中：$SchemaDir（Ctrl+C 结束）" -ForegroundColor Cyan
  Invoke-NormalizeOnce -Directory $SchemaDir -Silent:$Quiet | Out-Null

  # 用轮询而非 FileSystemWatcher：本脚本要在后台 Job 里长时间运行，
  # 而 Register-ObjectEvent -Action 在 Job 的 runspace 中不会可靠触发
  # （实测事件永不送达，文件不会被修复）。轮询 1s 开销可忽略且行为确定。
  try {
    while ($true) {
      $dirty = $false
      foreach ($file in @(Get-ChildItem -LiteralPath $SchemaDir -File -Filter '*.json' -ErrorAction SilentlyContinue)) {
        $bytes = $null
        try { $bytes = [System.IO.File]::ReadAllBytes($file.FullName) } catch { continue }
        if ($null -eq $bytes) { continue }
        if ($bytes.Length -eq 0) { $dirty = $true; break }
        # 结尾不是 LF，或正文中存在 CRLF，即需要规范化
        if ($bytes[$bytes.Length - 1] -ne 10) { $dirty = $true; break }
        for ($i = 0; $i -lt $bytes.Length - 1; $i++) {
          if ($bytes[$i] -eq 13 -and $bytes[$i + 1] -eq 10) { $dirty = $true; break }
        }
        if ($dirty) { break }
      }
      if ($dirty) {
        Invoke-NormalizeOnce -Directory $SchemaDir -Silent:$Quiet | Out-Null
      }
      Start-Sleep -Seconds 1
    }
  } finally {
    Invoke-NormalizeOnce -Directory $SchemaDir -Silent:$Quiet | Out-Null
    Write-Host "[normalize-schemas] 已停止监听并做最终规范化。" -ForegroundColor Cyan
  }
  exit 0
}

$count = Invoke-NormalizeOnce -Directory $SchemaDir -Silent:$Quiet
if ($count -eq 0 -and -not $Quiet) {
  Write-Host "[normalize-schemas] 无需变更（已符合文本约定）。" -ForegroundColor DarkGray
}
exit 0
