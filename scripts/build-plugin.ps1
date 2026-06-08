# Build dist/system-monitor.plugin: a Windows x64 release build of the MCP
# server bundled with the plugin manifest, MCP config, and skill. No Node
# required. Run from anywhere; paths are resolved relative to the repo.
#
#   pwsh scripts/build-plugin.ps1
#
# Requires: a Rust toolchain (cargo) on PATH. On Windows with the GNU toolchain,
# a full mingw-w64 (e.g. WinLibs) must also be on PATH for linking.

#requires -Version 5
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host 'Building release binary...'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    $exe = Join-Path $root 'target\release\system-monitor.exe'
    if (-not (Test-Path $exe)) { throw "missing build output: $exe" }

    # Stage the binary at bin/ so the .mcp.json reference
    # (${CLAUDE_PLUGIN_ROOT}/bin/system-monitor.exe) resolves for both the
    # packaged plugin and local from-source runs.
    $bin = Join-Path $root 'bin'
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item $exe (Join-Path $bin 'system-monitor.exe') -Force

    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force $dist | Out-Null
    $stage = Join-Path $dist 'stage'
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force (Join-Path $stage 'bin') | Out-Null

    foreach ($item in '.claude-plugin', '.mcp.json', 'skills', 'README.md', 'LICENSE') {
        Copy-Item (Join-Path $root $item) (Join-Path $stage $item) -Recurse -Force
    }
    Copy-Item (Join-Path $bin 'system-monitor.exe') (Join-Path $stage 'bin\system-monitor.exe') -Force

    $out = Join-Path $dist 'system-monitor.plugin'
    $zip = Join-Path $dist 'system-monitor.zip'
    if (Test-Path $zip) { Remove-Item -Force $zip }
    if (Test-Path $out) { Remove-Item -Force $out }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
    Rename-Item $zip $out
    Remove-Item -Recurse -Force $stage

    $mb = '{0:N2}' -f ((Get-Item $out).Length / 1MB)
    Write-Host "Built dist\system-monitor.plugin ($mb MB)"
} finally {
    Pop-Location
}
