param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("pull", "push")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$AppConfigPath,

    [Parameter(Mandatory = $true)]
    [string]$RepoRoot
)

<#
.SYNOPSIS
    Sync SuperControlPanel config.json between the app config directory and the Git-managed repo.

.DESCRIPTION
    This script copies:
      - in 'pull' mode:  RepoRoot\app-config\config.json  -->  AppConfigPath
      - in 'push' mode:  AppConfigPath  -->  RepoRoot\app-config\config.json

    Typical usage on each device:

      # Once: set paths (adjust to your environment)
      $env:SPC_APP_CONFIG = "C:\Path\To\SuperControlPanel\config.json"
      $env:SPC_CONFIG_REPO = "D:\config-sync"

      # Before using the app on a new device
      pwsh -File .\scripts\sync-app-config.ps1 -Mode pull -AppConfigPath $env:SPC_APP_CONFIG -RepoRoot $env:SPC_CONFIG_REPO

      # After changing settings you want to share
      pwsh -File .\scripts\sync-app-config.ps1 -Mode push -AppConfigPath $env:SPC_APP_CONFIG -RepoRoot $env:SPC_CONFIG_REPO

#>

$ErrorActionPreference = "Stop"

$repoConfigPath = Join-Path -Path $RepoRoot -ChildPath "app-config\config.json"

if (-not (Test-Path -Path (Split-Path -Path $AppConfigPath -Parent))) {
    New-Item -ItemType Directory -Path (Split-Path -Path $AppConfigPath -Parent) -Force | Out-Null
}

if (-not (Test-Path -Path (Split-Path -Path $repoConfigPath -Parent))) {
    New-Item -ItemType Directory -Path (Split-Path -Path $repoConfigPath -Parent) -Force | Out-Null
}

switch ($Mode) {
    "pull" {
        if (-not (Test-Path -Path $repoConfigPath)) {
            throw "Repo config not found at '$repoConfigPath'. Make sure you've cloned the repo and committed app-config/config.json."
        }
        Copy-Item -Path $repoConfigPath -Destination $AppConfigPath -Force
        Write-Host "Pulled config from repo to app path:"
        Write-Host "  $repoConfigPath -> $AppConfigPath"
    }
    "push" {
        if (-not (Test-Path -Path $AppConfigPath)) {
            throw "App config not found at '$AppConfigPath'. Launch SuperControlPanel once and save settings, or run in pull mode first."
        }
        Copy-Item -Path $AppConfigPath -Destination $repoConfigPath -Force
        Write-Host "Pushed config from app path to repo:"
        Write-Host "  $AppConfigPath -> $repoConfigPath"
    }
}

