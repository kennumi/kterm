# Builds KTerm in release mode and wraps it in dist\KTerm-Setup-<version>.exe.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
cargo build --release --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6.7+ not found: winget install JRSoftware.InnoSetup' }
& $iscc /Q "$PSScriptRoot\kterm.iss"
exit $LASTEXITCODE
