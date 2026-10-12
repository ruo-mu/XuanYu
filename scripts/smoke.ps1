param(
  [switch]$SkipFrontend,
  [switch]$SkipRust,
  [switch]$SkipBuild,
  [switch]$FullRust,
  [switch]$RequireClean,
  [switch]$NoCleanRootData,
  [string]$Toolchain = "stable-x86_64-pc-windows-gnu"
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$envScript = "D:\develop\environment\xuanyu-agent-app-env.ps1"
if (Test-Path -LiteralPath $envScript) {
  . $envScript
}

Set-Location -LiteralPath $repoRoot

function Invoke-Step {
  param(
    [string]$Name,
    [scriptblock]$Body
  )

  $start = Get-Date
  Write-Host ""
  Write-Host "==> $Name"
  & $Body
  $elapsed = (Get-Date) - $start
  Write-Host ("<== {0} ({1:n1}s)" -f $Name, $elapsed.TotalSeconds)
}

function Assert-Command {
  param([string]$Name)

  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    throw "Required command '$Name' was not found on PATH."
  }
}

function Invoke-Cargo {
  param([string[]]$CargoArgs)

  if ([string]::IsNullOrWhiteSpace($Toolchain)) {
    & cargo @CargoArgs
  } else {
    & cargo "+$Toolchain" @CargoArgs
  }
}

function Clear-RootDataDirectory {
  if ($NoCleanRootData) {
    return
  }

  $expected = [System.IO.Path]::GetFullPath((Join-Path $repoRoot ".xuanyu"))
  if (-not (Test-Path -LiteralPath $expected)) {
    return
  }

  $resolved = (Resolve-Path -LiteralPath $expected).Path
  if ($resolved -ne $expected) {
    throw "Refusing to remove unexpected path: $resolved"
  }

  Write-Host "Removing test data directory $resolved"
  Remove-Item -LiteralPath $resolved -Recurse -Force
}

Invoke-Step "toolchain availability" {
  Assert-Command git
  Assert-Command pnpm
  Assert-Command node
  Assert-Command cargo
  git --version
  node --version
  pnpm --version
  if ([string]::IsNullOrWhiteSpace($Toolchain)) {
    cargo --version
  } else {
    cargo "+$Toolchain" --version
  }
}

Invoke-Step "workspace status" {
  $status = git status --porcelain
  if ($status) {
    $status | ForEach-Object { Write-Host $_ }
    if ($RequireClean) {
      throw "Workspace has uncommitted changes."
    }
  } else {
    Write-Host "clean"
  }
}

Clear-RootDataDirectory

if (-not $SkipFrontend) {
  Invoke-Step "frontend typecheck" {
    pnpm run typecheck -- --pretty false
  }

  if (-not $SkipBuild) {
    Invoke-Step "frontend production build" {
      pnpm run build
    }
  }
}

  if (-not $SkipRust) {
    $packages = @("xuanyu-core", "xuanyu-execution", "xuanyu-eventbus", "xuanyu-storage", "xuanyu-agent-bridge")
    if ($FullRust) {
      $packages = @(
        "xuanyu-protocol",
        "xuanyu-extensions",
        "xuanyu-eventbus",
        "xuanyu-storage",
        "xuanyu-governance",
        "xuanyu-session",
        "xuanyu-execution",
        "xuanyu-memory",
        "xuanyu-agents",
        "xuanyu-agent-bridge",
        "xuanyu-tools",
        "xuanyu-channels",
        "xuanyu-cli",
        "xuanyu-desktop-host",
        "xuanyu-core"
      )
    }

  foreach ($package in $packages) {
    Invoke-Step "cargo test -p $package" {
      Invoke-Cargo @("test", "-p", $package)
    }
  }

  Invoke-Step "headless smoke turn" {
    $smokeData = Join-Path $repoRoot ".xuanyu-smoke"
    if (Test-Path -LiteralPath $smokeData) {
      Remove-Item -LiteralPath $smokeData -Recurse -Force
    }
    Invoke-Cargo @("run", "-p", "xuanyu-cli", "--", "--data-dir", $smokeData, "smoke-turn")
  }
}

Clear-RootDataDirectory

Write-Host ""
Write-Host "Smoke checks completed."
