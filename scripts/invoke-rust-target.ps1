param(
  [Parameter(Mandatory = $true, Position = 0)]
  [ValidateSet("check", "test", "exact-a", "exact-b", "exact-c")]
  [string]$Slot,

  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$CargoArgs
)

$targetMap = @{
  "check" = "target-check"
  "test" = "target-test"
  "exact-a" = "target-test-exact-a"
  "exact-b" = "target-test-exact-b"
  "exact-c" = "target-test-exact-c"
}

$targetDir = $targetMap[$Slot]
$root = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $root "src-tauri/Cargo.toml"

if (-not $CargoArgs -or $CargoArgs.Count -eq 0) {
  Write-Error "Missing cargo subcommand. Pass 'check' or 'test' followed by any extra cargo arguments."
  exit 1
}

$subcommand = $CargoArgs[0]
$extraArgs = if ($CargoArgs.Count -gt 1) { $CargoArgs[1..($CargoArgs.Count - 1)] } else { @() }

$cargoCommand = @(
  "cargo",
  $subcommand,
  "--manifest-path", $manifestPath,
  "--target-dir", (Join-Path $root $targetDir)
)

if ($extraArgs.Count -gt 0) {
  $cargoCommand += $extraArgs
}

& $cargoCommand[0] $cargoCommand[1..($cargoCommand.Length - 1)]
exit $LASTEXITCODE
