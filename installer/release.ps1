# Bumps the version, builds the installer, tags, and publishes a GitHub release.
#   .\installer\release.ps1              0.1.0 -> 0.1.1
#   .\installer\release.ps1 -Bump minor  0.1.3 -> 0.2.0
#   .\installer\release.ps1 -Bump major  0.2.1 -> 1.0.0
param([ValidateSet('patch', 'minor', 'major')][string]$Bump = 'patch')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Set-Location $root

# The binary is built from the working tree, so it must match what gets tagged.
if (git status --porcelain --untracked-files=no) { throw 'Commit or stash your changes first.' }

$toml = Get-Content Cargo.toml -Raw
if ($toml -notmatch '(?m)^version = "(\d+)\.(\d+)\.(\d+)"') { throw 'No version in Cargo.toml' }
[int]$maj, [int]$min, [int]$pat = $Matches[1..3]
switch ($Bump) {
    'major' { $maj++; $min = 0; $pat = 0 }
    'minor' { $min++; $pat = 0 }
    'patch' { $pat++ }
}
$ver = "$maj.$min.$pat"
Write-Host "Releasing v$ver" -ForegroundColor Cyan

[IO.File]::WriteAllText("$root\Cargo.toml", ([regex]'(?m)^version = "[^"]*"').Replace($toml, "version = `"$ver`"", 1))
& "$PSScriptRoot\build.ps1"  # also refreshes Cargo.lock
if ($LASTEXITCODE) { git checkout -- Cargo.toml Cargo.lock; throw 'Build failed; version bump reverted.' }

git commit -m "Release v$ver" -- Cargo.toml Cargo.lock
git tag "v$ver"
git push origin HEAD "v$ver"
if ($LASTEXITCODE) { throw 'Push failed.' }
gh release create "v$ver" "dist\KTerm-Setup-$ver.exe" --title "KTerm $ver" --generate-notes
