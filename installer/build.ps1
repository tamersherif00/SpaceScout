# Builds the release executable and the Windows x64 installer.
# Output: dist\SpaceScout-<version>-x64-setup.exe
# Requires Inno Setup 6:  winget install JRSoftware.InnoSetup

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "SpaceScout $version"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'ISCC.exe not found. Install with: winget install JRSoftware.InnoSetup' }

& $iscc "/DAppVersion=$version" "$PSScriptRoot\spacescout.iss"
if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }

Write-Host "Installer: $root\dist\SpaceScout-$version-x64-setup.exe"
