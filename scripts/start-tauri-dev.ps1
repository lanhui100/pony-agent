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
# 残留进程扫描在受限环境（服务会话/沙箱，WMI root\cimv2 拒绝访问）下降级为
# 跳过清理并继续启动——清理失败不应阻断 dev 启动；端口占用由下方端口清理兜底。
try {
  $processRows = Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
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
} catch {
  Write-Warning "process scan skipped (Get-CimInstance unavailable in this context): $($_.Exception.Message)"
}

# 兜底：直接按进程名扫描 pony-agent，避免 WMI 在权限或提权边界下 CommandLine 为空导致漏杀
Get-Process -Name "pony-agent" -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $PID } | ForEach-Object {
  $launcherPids += $_.Id
}

# 兜底：清理属于当前应用 EBWebView 的残留 msedgewebview2 进程，防止 WebView2 锁定用户数据目录导致崩溃
try {
  Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -like "*com.pony.agent*" } |
    ForEach-Object { $launcherPids += $_.ProcessId }
} catch {}

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
Get-NetTCPConnection -LocalPort 4176 -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty OwningProcess -Unique |
  ForEach-Object {
    if ($_ -and $_ -ne $PID) {
      $procName = (Get-Process -Id $_ -ErrorAction SilentlyContinue).ProcessName
      Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue
      Write-Host "[port] freed port 4176 (killed $procName PID $_)"
    }
  }

# 编译调优（jobs / incremental / debug）统一由工作区 .cargo/config.toml 提供，
# 这里不再重复设置，避免两处漂移。
# 显式设置 CARGO_TARGET_DIR 为 target/，与 .cargo/config.toml 的 [build] target-dir 保持一致。
# 双重保障：即使 cargo 未读取到配置文件、或工作路径异常，tauri dev 的编译产物也一定落入 target/，
# 不会因为从 src-tauri/ 或其它子目录触发而意外创建 src-tauri/target/ 等重复目录。
$env:CARGO_TARGET_DIR = "$workspace\target"
$env:PATH = "$HOME\.cargo\bin;$env:PATH"

# ── npm 环境净化：剔除 pnpm 注入的未知 npm 配置 ──────────────────────
# pnpm 启动的终端会向子进程注入 npm_config_manage_package_manager_versions
# （pnpm 专属键，npm 不识别），导致 BeforeDevCommand 的 `npm run dev` 每次输出
# "Unknown env config" 告警。在脚本作用域内移除，保证 tauri dev 全程无该噪音。
# （外层 `npm run dev:tauri` 自身的同类告警发生在本脚本启动前，仓库侧无法消除，
# 仅当宿主 shell 由 pnpm 注入环境时出现。）
# 写 $null 等价于删除该环境变量，且不产生错误记录、不依赖 ErrorAction 偏好。
$env:npm_config_manage_package_manager_versions = $null

$npmBin = Join-Path $workspace 'node_modules\.bin'
$tauriShim = Join-Path $npmBin 'tauri.cmd'
if (-not (Test-Path $tauriShim)) {
  throw "无法找到 Tauri CLI: $tauriShim"
}

Set-Content -LiteralPath $pidFile -Value $PID

# ── 生成物文本约定（PA-102 §2）──────────────────────────────────────
# `tauri dev` 每次重新生成 src-tauri/gen/schemas/*.json 且不写结尾换行，
# 导致工作树在整个 dev 会话期间恒定出现 4 个空 diff 的"已修改"文件。
# 这些文件是 Tauri 权限契约，保留跟踪（不 gitignore），改为后台监听补齐
# LF + 结尾换行。监听失败不影响 dev 启动，退出时兜底再规范化一次。
$normalizer = Join-Path $PSScriptRoot "normalize-generated-schemas.ps1"
$normalizerJob = $null
$normalizerShell = $null
if (Test-Path -LiteralPath $normalizer) {
  try {
    $normalizerShell = Start-Job -ScriptBlock {
      param($scriptPath, $root)
      & pwsh -NoProfile -ExecutionPolicy Bypass -File $scriptPath -Watch -Quiet
    } -ArgumentList $normalizer, $workspace
    $normalizerJob = $normalizerShell.Id
  } catch {
    Write-Warning "schema 规范化监听未启动（不影响 dev）：$($_.Exception.Message)"
  }
}

$viteProcess = $null
try {
  # ── 原生启动 Vite dev server ─────────────────────────────────────────
  # 避免通过 Node @tauri-apps/cli 管道间接嵌套启动 cargo 导致 Windows Job Object
  # 约束下 WebView2 报 0x80070578 (无效窗口句柄) 故障。
  $viteBin = Join-Path $workspace 'node_modules\vite\bin\vite.js'
  if (Test-Path $viteBin) {
    Write-Host "[dev] starting Vite dev server directly via node ..."
    $viteProcess = Start-Process -FilePath "node.exe" -ArgumentList "`"$viteBin`"", "--configLoader", "native" -WorkingDirectory $workspace -PassThru -NoNewWindow
  } else {
    Write-Host "[dev] starting Vite dev server via npm ..."
    $viteProcess = Start-Process -FilePath "npm.cmd" -ArgumentList "run", "dev" -WorkingDirectory $workspace -PassThru -NoNewWindow
  }

  # 等待 4176 端口就绪（超时 30 秒）
  $ready = $false
  for ($i = 0; $i -lt 100; $i++) {
    try {
      $tcp = New-Object System.Net.Sockets.TcpClient
      $iar = $tcp.BeginConnect("127.0.0.1", 4176, $null, $null)
      $success = $iar.AsyncWaitHandle.WaitOne(300, $false)
      if ($success -and $tcp.Connected) {
        $tcp.EndConnect($iar)
        $tcp.Close()
        $ready = $true
        break
      }
      $tcp.Close()
    } catch {}
    Start-Sleep -Milliseconds 300
  }
  if (-not $ready) {
    throw "Vite dev server failed to start within timeout"
  }
  Write-Host "[dev] Vite dev server ready on http://127.0.0.1:4176"

  # ── 原生调起桌面端 ─────────────────────────────────────────────────
  $cargoPath = Join-Path $HOME '.cargo\bin\cargo.exe'
  if (-not (Test-Path $cargoPath)) {
    $cargoPath = 'cargo'
  }
  & $cargoPath run --manifest-path "$workspace\src-tauri\Cargo.toml" --no-default-features
} finally {
  if (Test-Path $pidFile) {
    Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
  }
  # 确保 Vite 进程退出并释放端口
  if ($viteProcess -and -not $viteProcess.HasExited) {
    try {
      Stop-Process -Id $viteProcess.Id -Force -ErrorAction SilentlyContinue
    } catch {}
  }
  Get-NetTCPConnection -LocalPort 4176 -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty OwningProcess -Unique |
    ForEach-Object {
      if ($_ -and $_ -ne $PID) {
        Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue
      }
    }
  # 先停监听（它自身退出前会做一次最终规范化），再兜底规范化一次
  if ($normalizerJob) {
    Stop-Job -Id $normalizerJob -ErrorAction SilentlyContinue
    Remove-Job -Id $normalizerJob -Force -ErrorAction SilentlyContinue
  }
  if (Test-Path -LiteralPath $normalizer) {
    & pwsh -NoProfile -ExecutionPolicy Bypass -File $normalizer -Quiet | Out-Null
  }
}
