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

$env:CARGO_BUILD_JOBS = "2"
$env:CARGO_INCREMENTAL = "1"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
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
