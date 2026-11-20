<#
.SYNOPSIS
  POSIX 落点转派器 —— 本仓 `*.ps1` 「通用适配」的单一真源。

.DESCRIPTION
  本仓的历史入口全部是 PowerShell（`npm run dev:tauri` 曾直接写 `powershell -File`），
  在 Linux/macOS 上直接「命令不存在」或跑出与 Windows 不同的行为。两类入口因此并存：

    - `scripts/*.ps1`：Windows 原生实现（Job Object / WebView2 / 注册表等平台逻辑不可替代）；
    - `scripts/*.sh` / `scripts/*.mjs`：同名 POSIX 落点（真源同权，行为对齐）。

  本文件让 ps1 在**非 Windows 主机**上把控制权交给 POSIX 落点，而不是假装失败：
  入口仍然是同一个文件，参数由调用点**显式翻译**（命名参数在 PS 里无法从 $args 还原，
  所以不做「猜参数」这种静默错配）。

  没有 POSIX 落点的脚本（当前仅 `bump-version.ps1`）必须由调用点传 `-WindowsOnly`：
  非 Windows 主机上 fail-closed（exit 4）并给出可执行的下一步，绝不「找不到就算过」。

  映射表漂移由 `scripts/check-ps1-portability.sh` 机械拦截（新增 ps1 不登记即红）。

.NOTES
  环境变量：
    PONY_FORCE_POSIX_DISPATCH=1  —— 在 Windows 上强制走 POSIX 分支（自检用）。
#>

# 单一映射表：ps1 基名 → POSIX 落点（相对 scripts/）。新增 ps1 必须在此登记。
$PonyPosixTwins = @{
  'clean-targets'               = 'clean-targets.sh'
  'normalize-generated-schemas' = 'normalize-generated-schemas.sh'
  'start-tauri-dev'             = 'start-tauri-dev.sh'
  'invoke-rust-target'          = 'invoke-rust-target.mjs'
  'check-version-sync'          = 'check-version-sync.mjs'
}

function Test-PonyWindowsHost {
  if ($env:PONY_FORCE_POSIX_DISPATCH -eq '1') { return $false }
  # Windows PowerShell 5.1 只存在于 Windows，且没有 $IsWindows。
  if ($PSVersionTable.PSVersion.Major -lt 6) { return $true }
  try { return [bool]$IsWindows } catch { return ($env:OS -eq 'Windows_NT') }
}

function Invoke-PonyPosixTwin {
  param(
    [Parameter(Mandatory = $true)][string]$Name,
    [string[]]$ForwardArgs = @(),
    [switch]$WindowsOnly,
    # 逻辑重复（PowerShell 版与 .mjs 版是同一套判定）的脚本用这个开关：**所有平台**都转派到单一
    # Node 真源，本文件退化成薄壳。这样「一份逻辑两份实现必然漂移」这一类缺陷从根上消失；
    # 只有真含 Windows 独有逻辑（Job Object / WebView2 / 符号链接策略）的脚本才保留原生实现。
    [switch]$AlwaysDelegate
  )

  if (-not $AlwaysDelegate -and (Test-PonyWindowsHost)) { return }

  # $PSScriptRoot 在「被 dot-source 的文件」里的取值随宿主实现有差异（本文件目录 vs 调用方目录），
  # 所以候选目录两个都试、命中即用；都不命中则 fail-closed（绝不猜路径）。
  $candidateDirs = @()
  if ($PSScriptRoot) {
    $candidateDirs += (Split-Path -Parent $PSScriptRoot)   # 本文件在 scripts/lib ⇒ 父目录 = scripts
    $candidateDirs += $PSScriptRoot                        # 宿主把 $PSScriptRoot 解析成调用方目录时的情形
  }

  if ($WindowsOnly) {
    $msg = "[$Name] 非 Windows 主机：本脚本暂无 POSIX 落点（$Name.sh / $Name.mjs 未实现）。"
    $msg += "`n  这是已登记的缺口（scripts/check-ps1-portability.sh 白名单），请改走跨平台入口："
    $msg += "`n    - 版本 bump / 发版：npm run release（node 版真源 scripts/release.mjs）"
    $msg += "`n    - 其余构建任务：node scripts/run-portable.mjs <task>（任务表见该文件）"
    Write-Error $msg
    exit 4
  }

  $twinRel = $PonyPosixTwins[$Name]
  if (-not $twinRel) {
    Write-Error "[$Name] 转派失败：scripts/lib/posix-dispatch.ps1 的 `$PonyPosixTwins 未登记该脚本（新增 ps1 必须登记 POSIX 落点）。"
    exit 4
  }

  $twin = $null
  foreach ($dir in $candidateDirs) {
    if (-not $dir) { continue }
    $probe = Join-Path $dir $twinRel
    if (Test-Path -LiteralPath $probe) { $twin = $probe; break }
  }
  if (-not $twin) {
    Write-Error "[$Name] 转派失败：POSIX 落点不存在 → scripts/$twinRel（候选目录：$($candidateDirs -join ' / ')）"
    exit 4
  }

  $runner = if ($twinRel.EndsWith('.mjs')) { 'node' } else { 'bash' }
  if (-not (Get-Command $runner -ErrorAction SilentlyContinue)) {
    Write-Error "[$Name] 转派失败：非 Windows 主机上找不到 $runner（POSIX 落点 $twinRel 无法执行）。"
    exit 4
  }

  $where = if (Test-PonyWindowsHost) { '单真源薄壳（Windows 亦转派）' } else { '非 Windows 主机' }
  Write-Host "[$Name] $where → 转派 ${runner} scripts/$twinRel $($ForwardArgs -join ' ')"
  & $runner $twin @ForwardArgs
  exit $LASTEXITCODE
}
