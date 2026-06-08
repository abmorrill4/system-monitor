# Install the elevated SMART helper: a scheduled task that runs
# `system-monitor --refresh-smart-cache` as SYSTEM every 15 minutes (and at
# startup), writing a drive-health cache the non-elevated MCP server reads.
# This lets "drive health" work without running Claude itself as Administrator.
#
# Run this ONCE, in an elevated (Administrator) PowerShell:
#   pwsh -File scripts/install-smart-helper.ps1
#
# Requires smartmontools (winget install smartmontools.smartmontools).

[CmdletBinding()]
param(
    # Path to system-monitor.exe. Defaults to the binary shipped next to this
    # script (works from the repo and from an installed plugin).
    [string]$BinaryPath = (Join-Path (Split-Path -Parent $PSScriptRoot) 'bin\system-monitor.exe'),
    [int]$IntervalMinutes = 15
)

$ErrorActionPreference = 'Stop'
$TaskName = 'system-monitor-smart-cache'

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
if (-not $isAdmin) {
    Write-Error 'This script must run in an elevated (Administrator) PowerShell. Right-click PowerShell -> Run as administrator, then re-run.'
    exit 1
}

if (-not (Test-Path $BinaryPath)) {
    Write-Error "system-monitor.exe not found at: $BinaryPath`nPass -BinaryPath <path-to-system-monitor.exe>."
    exit 1
}

# Warn (don't fail) if smartctl isn't installed yet - the refresh will report it.
$smartctl = Get-Command smartctl -ErrorAction SilentlyContinue
if (-not $smartctl -and -not (Test-Path 'C:\Program Files\smartmontools\bin\smartctl.exe')) {
    Write-Warning 'smartctl not found. Install it with: winget install smartmontools.smartmontools'
}

# Copy the binary to a stable, all-users location (the plugin cache path is
# versioned and changes on update; a scheduled task needs a fixed path).
$installDir = Join-Path $env:ProgramData 'system-monitor\bin'
New-Item -ItemType Directory -Force $installDir | Out-Null
$dest = Join-Path $installDir 'system-monitor.exe'
Copy-Item $BinaryPath $dest -Force
Write-Host "Installed helper binary -> $dest"

# Register the scheduled task: run as SYSTEM, highest privileges, at startup and
# repeating every $IntervalMinutes.
$action = New-ScheduledTaskAction -Execute $dest -Argument '--refresh-smart-cache'
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date) `
    -RepetitionInterval (New-TimeSpan -Minutes $IntervalMinutes) `
    -RepetitionDuration ([TimeSpan]::FromDays(3650))
$atStartup = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger @($trigger, $atStartup) `
    -Principal $principal -Settings $settings -Force `
    -Description 'Refreshes the system-monitor SMART drive-health cache as SYSTEM.' | Out-Null
Write-Host "Registered scheduled task '$TaskName' (every $IntervalMinutes min + at startup)."

# Seed the cache immediately (we're already elevated) and surface any error.
Write-Host 'Seeding the SMART cache now...'
& $dest --refresh-smart-cache
if ($LASTEXITCODE -ne 0) {
    Write-Warning 'Initial SMART scan failed (see message above). The task is installed and will retry on schedule; install smartmontools if missing.'
} else {
    Write-Host "Done. Cache: $($env:ProgramData)\system-monitor\smart-cache.json"
    Write-Host 'Drive health now works without running Claude elevated.'
}
