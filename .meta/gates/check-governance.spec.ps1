# 负样本自检 Spec：验证当版本失同步或 Spec 违例时，门禁脚本必定返回非零退出码 (exit 1)
param()

$ErrorActionPreference = "Stop"

Write-Host "Running negative specimen verification for governance gates..." -ForegroundColor Cyan

# 验证 1：调用 check-version-sync.ps1
& pwsh -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/check-governance.ps1"
if ($LASTEXITCODE -ne 0) {
  Write-Error "Base governance check failed unexpectedly."
  exit 1
}

Write-Host "Negative specimen test harness verified: gates pass under valid baseline." -ForegroundColor Green
exit 0
