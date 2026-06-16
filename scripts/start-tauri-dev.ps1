$ErrorActionPreference = "Stop"

$workspace = Split-Path -Parent $PSScriptRoot
$workspaceNormalized = [System.IO.Path]::GetFullPath($workspace)
$workspaceLower = $workspaceNormalized.ToLowerInvariant()
$previousPid = $null
$pidFile = Join-Path $workspace ".tauri-dev.pid"
if (Test-Path $pidFile) {
  try {
    $previousPid = [int](Get-Content -Raw $pidFile)
  } catch {
    $previousPid = $null
  }

  if ($previousPid -and (Get-Process -Id $previousPid -ErrorAction SilentlyContinue)) {
    try {
      Stop-Process -Id $previousPid -Force -ErrorAction Stop
    } catch {
      Write-Warning "failed to stop previous tauri pid ${previousPid}: $($_.Exception.Message)"
    }
  }
}

$launcherPids = @()
$processRows = Get-CimInstance Win32_Process | Where-Object {
  if ($_.ProcessId -eq $PID) {
    return $false
  }

  if ($_.ProcessId -eq $previousPid) {
    return $true
  }

  $cmd = $_.CommandLine
  if ([string]::IsNullOrWhiteSpace($cmd)) {
    return $false
  }

  $cmdLower = $cmd.ToLowerInvariant()
  if (-not $cmdLower.Contains($workspaceLower)) {
    return $false
  }

  return (
    $cmdLower.Contains("run dev:tauri") -or
    $cmdLower.Contains("npm run dev") -or
    $cmdLower.Contains("tauri dev") -or
    $cmdLower.Contains("@tauri-apps\\cli\\tauri.js") -or
    $cmdLower.Contains('cargo.exe" run') -or
    $cmdLower.Contains("cargo.exe run") -or
    $cmdLower.Contains("pony-agent.exe")
  )
}

foreach ($process in $processRows) {
  $launcherPids += $process.ProcessId
}

foreach ($processId in $launcherPids | Sort-Object -Unique) {
  try {
    Stop-Process -Id $processId -Force -ErrorAction Stop
  } catch {
    Write-Warning "failed to stop pid ${processId}: $($_.Exception.Message)"
  }
}

Start-Sleep -Seconds 1

# ── 端口清理：强制释放 Vite dev server 端口 ──────────────────────────
# 上一个 tauri dev 退出后，Vite 进程可能残留并继续占用 4176 端口。
# 命令行模式匹配会漏掉它（进程名是 node.exe，不包含 "tauri" 或 "npm run dev"），
# 这里通过端口号直杀，保证 beforeDevCommand 中的 vite 能正常启动。
$portPid = $null
$portConn = Get-NetTCPConnection -LocalPort 4176 -ErrorAction SilentlyContinue
if ($portConn -and $portConn.State -in @("Listen", "Established", "Bound")) {
  $portPid = $portConn.OwningProcess
}
if ($portPid) {
  $procName = (Get-Process -Id $portPid -ErrorAction SilentlyContinue).ProcessName
  Stop-Process -Id $portPid -Force -ErrorAction SilentlyContinue
  Write-Host "[port] freed port 4176 (killed $procName PID $portPid)"
}

# 编译调优（jobs / incremental / debug）统一由工作区 .cargo/config.toml 提供，
# 这里不再重复设置，避免两处漂移。
# 显式设置 CARGO_TARGET_DIR 为 target/，与 .cargo/config.toml 的 [build] target-dir 保持一致。
# 双重保障：即使 cargo 未读取到配置文件、或工作路径异常，tauri dev 的编译产物也一定落入 target/，
# 不会因为从 src-tauri/ 或其它子目录触发而意外创建 src-tauri/target/ 等重复目录。
$env:CARGO_TARGET_DIR = "$workspace\target"
$env:PATH = "$HOME\.cargo\bin;$env:PATH"

$npmBin = Join-Path $workspace 'node_modules\.bin'
$tauriShim = Join-Path $npmBin 'tauri.cmd'
if (-not (Test-Path $tauriShim)) {
  throw "无法找到 Tauri CLI: $tauriShim"
}

Set-Content -LiteralPath $pidFile -Value $PID
try {
  & $tauriShim dev
} finally {
  if (Test-Path $pidFile) {
    Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
  }
}
