$repoRoot = (Resolve-Path "$PSScriptRoot/../..").Path

# 门禁 1：版本四处同步校验
& pwsh -NoProfile -ExecutionPolicy Bypass -File "$repoRoot/scripts/check-version-sync.ps1"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "[gate-all] 所有治理门禁验证通过！" -ForegroundColor Green
exit 0
