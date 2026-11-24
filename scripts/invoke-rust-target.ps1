# invoke-rust-target.ps1 —— 历史 Windows 入口，现已收敛为 **Node 单真源薄壳**。
#
# 为什么删掉原生实现：本文件与 scripts/invoke-rust-target.mjs 是同一套槽位映射 + cargo 调用，
# 两份实现必然漂移——本轮实测的漂移后果就是「.mjs 已吃并发预算与全局 flock，.ps1 仍在裸调 cargo」
# （Windows / windows-latest CI 因此绕过构建治理）。收敛为薄壳后该缺陷类别从根上消失。
#
# 落点：scripts/invoke-rust-target.mjs（槽位表 + lane 包装 + 退出码契约的唯一真源）。
# 非 Windows 主机上同样走这里（dispatcher 与 -AlwaysDelegate 共同保证）。

param(
  [Parameter(Mandatory = $true, Position = 0)]
  [ValidateSet("check", "test", "release", "exact-a", "exact-b", "exact-c")]
  [string]$Slot,

  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$CargoArgs
)

. "$PSScriptRoot/lib/posix-dispatch.ps1"
$ponyForward = @()
if ($Slot) { $ponyForward += $Slot }
if ($CargoArgs) { $ponyForward += $CargoArgs }
Invoke-PonyPosixTwin -Name 'invoke-rust-target' -ForwardArgs $ponyForward -AlwaysDelegate

# 到不了这里：dispatcher 在 -AlwaysDelegate 下必定 exit。
Write-Error "[invoke-rust-target] dispatcher 未接管（scripts/lib/posix-dispatch.ps1 被改动？）"
exit 4
