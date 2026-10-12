param(
  [string]$HostAddress = "127.0.0.1",
  [int]$Port = 5173
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$envScript = "D:\develop\environment\xuanyu-agent-app-env.ps1"
if (Test-Path -LiteralPath $envScript) {
  . $envScript
}

Set-Location -LiteralPath $repoRoot
$env:HOST = $HostAddress
$env:PORT = [string]$Port

pnpm run dev
