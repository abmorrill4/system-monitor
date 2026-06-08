# Remove the elevated SMART helper scheduled task and its cached data.
# Run in an elevated (Administrator) PowerShell:
#   pwsh -File scripts/uninstall-smart-helper.ps1

[CmdletBinding()]
param([switch]$KeepData)

$ErrorActionPreference = 'Stop'
$TaskName = 'system-monitor-smart-cache'

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
if (-not $isAdmin) {
    Write-Error 'This script must run in an elevated (Administrator) PowerShell.'
    exit 1
}

if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    Write-Host "Removed scheduled task '$TaskName'."
} else {
    Write-Host "Scheduled task '$TaskName' not found (already removed)."
}

if (-not $KeepData) {
    $dir = Join-Path $env:ProgramData 'system-monitor'
    if (Test-Path $dir) {
        Remove-Item -Recurse -Force $dir
        Write-Host "Removed $dir."
    }
}
