<#
.SYNOPSIS
  Validates that all version files across the repository remain in sync.

.DESCRIPTION
  历史 Windows 入口，现已收敛为 **Node 单真源薄壳**：判定逻辑（.version.json /
  crates/pony-agent-core/Cargo.toml / src-tauri/Cargo.toml / src-tauri/tauri.conf.json /
  package.json 五处版本一致性）唯一真源是 `scripts/check-version-sync.mjs`。

  为什么删掉原生实现：PowerShell 版与 .mjs 版是同一套判定，两份实现必然漂移；本仓的
  `npm run version:check` 一直走 .mjs，于是 .ps1 分支长期无人验证，属于「假绿」风险面。
  收敛后 Windows / Linux / CI 跑的是同一份判定。
#>

$ErrorActionPreference = "Stop"

. "$PSScriptRoot/lib/posix-dispatch.ps1"
Invoke-PonyPosixTwin -Name 'check-version-sync' -ForwardArgs $args -AlwaysDelegate

# 到不了这里：dispatcher 在 -AlwaysDelegate 下必定 exit。
Write-Error "[check-version-sync] dispatcher 未接管（scripts/lib/posix-dispatch.ps1 被改动？）"
exit 4
