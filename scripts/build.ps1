$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$envScript = "D:\develop\environment\xuanyu-agent-app-env.ps1"
if (Test-Path -LiteralPath $envScript) {
  . $envScript
}

Set-Location -LiteralPath $repoRoot

pnpm run build
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host ""
Write-Host "Dev build ready. For release package:"
Write-Host "  pnpm dist"
