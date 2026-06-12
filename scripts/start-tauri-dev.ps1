$ErrorActionPreference = "Stop"

$workspace = Split-Path -Parent $PSScriptRoot
$workspaceNormalized = [System.IO.Path]::GetFullPath($workspace)
$workspaceLower = $workspaceNormalized.ToLowerInvariant()

function Test-IsWorkspaceProcess {
  param(
    [string]$CommandLine
  )

  if ([string]::IsNullOrWhiteSpace($CommandLine)) {
    return $false
  }

  return $CommandLine.ToLowerInvariant().Contains($workspaceLower)
}

$processes = Get-CimInstance Win32_Process | Where-Object {
  if ($_.Name -eq "pony-agent.exe") {
    return $true
  }

  if ($_.Name -notin @("node.exe", "cargo.exe")) {
    return $false
  }

  $cmd = $_.CommandLine
  if (-not (Test-IsWorkspaceProcess -CommandLine $cmd)) {
    return $false
  }

  $cmdLower = $cmd.ToLowerInvariant()

  return (
    $cmdLower.Contains("vite") -or
    $cmdLower.Contains("@tauri-apps\\cli\\tauri.js") -or
    $cmdLower.Contains("run dev:tauri") -or
    $cmdLower.Contains('cargo.exe" run') -or
    $cmdLower.Contains("cargo.exe run")
  )
}

foreach ($process in $processes) {
  try {
    Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
  } catch {
    Write-Warning "failed to stop pid $($process.ProcessId): $($_.Exception.Message)"
  }
}

Start-Sleep -Seconds 2

$env:CARGO_BUILD_JOBS = "2"
$env:CARGO_INCREMENTAL = "1"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:PATH = "$HOME\.cargo\bin;$env:PATH"

& tauri dev
