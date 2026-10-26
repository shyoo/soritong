<#
.SYNOPSIS
    Build a release of soritong into dist\.

.DESCRIPTION
    Runs checks, builds optimised binaries, stages runtime DLLs,
    packages the NSIS installer and portable ZIP, and produces SHA256SUMS.txt.
    Works from any current directory in PowerShell 7 (`pwsh`).

.PARAMETER SkipTests
    Skip running tests before building.

.PARAMETER StopRunning
    Stop running soritong-app processes if running from target\release or dist\.

.EXAMPLE
    pwsh -ExecutionPolicy Bypass -File scripts\release.ps1
#>
[CmdletBinding()]
param(
    [switch]$SkipTests,
    [switch]$StopRunning,
    [string]$Version = $env:SORITONG_VERSION
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }

function Invoke-Cargo {
    & $script:cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args -join ' ') failed (exit $LASTEXITCODE)" }
}

# 1. Locate cargo
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    throw "cargo not found. Please install Rust toolchain."
}

# 2. Resolve version
. (Join-Path $PSScriptRoot 'version.ps1')
$version = Get-SoritongVersion $Version
$env:SORITONG_VERSION = $version

$dist = Join-Path $root 'dist'
$builtApp = Join-Path $root 'target\release\soritong-app.exe'

# 3. Check for running processes that lock target files
$locked = @(Get-Process -Name 'soritong-app', 'soritong' -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and ($_.Path -ieq $builtApp -or $_.Path -like (Join-Path $dist '*'))
})
if ($locked.Count -gt 0) {
    if ($StopRunning) {
        Step 'Stopping running soritong instances'
        $locked | Stop-Process -Force
        Start-Sleep -Milliseconds 300
    } else {
        throw "soritong is running from $($locked[0].Path). Close it or re-run with -StopRunning."
    }
}

# 4. Run tests
if (-not $SkipTests) {
    Step 'Running cargo test --workspace --locked'
    Invoke-Cargo test --workspace --locked

    Step 'Running UI tests with node'
    $node = (Get-Command node -ErrorAction SilentlyContinue).Source
    if (-not $node) { throw "node not found for UI tests" }
    & $node --test crates/soritong-app/ui-tests/keys.test.mjs crates/soritong-app/ui-tests/wizard.test.mjs
    if ($LASTEXITCODE -ne 0) { throw "UI tests failed (exit $LASTEXITCODE)" }
}

# 5. Build release binaries
Step "Building release binaries (v$version)"
Invoke-Cargo build --release --locked -p soritong-app

# 6. Locate runtime DLLs
Step 'Staging runtime DLLs'
$resourcesDir = Join-Path $root 'crates\soritong-app\resources'
New-Item -ItemType Directory -Force $resourcesDir | Out-Null

$directMl = Join-Path $root 'target\release\DirectML.dll'
if (-not (Test-Path $directMl)) {
    throw "DirectML.dll not found in target\release. Build did not produce DirectML."
}
Copy-Item $directMl (Join-Path $resourcesDir 'DirectML.dll') -Force

function Get-VcRedistDlls {
    $searchPaths = @(
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT",
        "${env:ProgramFiles}\Microsoft Visual Studio\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT"
    )
    foreach ($pattern in $searchPaths) {
        $dirs = @(Get-Item -Path $pattern -ErrorAction SilentlyContinue)
        if ($dirs.Count -gt 0) {
            $dlls = @(Get-ChildItem -Path $dirs[0].FullName -Filter "*.dll")
            if ($dlls.Count -gt 0) { return $dlls }
        }
    }
    # Fallback to System32
    $names = @('msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
    $dlls = @()
    foreach ($name in $names) {
        $p = Join-Path $env:SystemRoot "System32\$name"
        if (Test-Path $p) { $dlls += Get-Item $p }
    }
    return $dlls
}

$vcDlls = Get-VcRedistDlls
if (-not $vcDlls -or $vcDlls.Count -eq 0) {
    throw "Could not locate MSVC CRT runtime DLLs to redistribute."
}
foreach ($dll in $vcDlls) {
    Copy-Item $dll.FullName (Join-Path $resourcesDir $dll.Name) -Force
}

# 7. Build Tauri NSIS bundle
Step 'Building NSIS installer via Tauri'
$tauriCli = (Get-Command cargo-tauri -ErrorAction SilentlyContinue).Source
if (-not $tauriCli) {
    $candidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo-tauri.exe'
    if (Test-Path $candidate) { $tauriCli = $candidate }
}
if (-not $tauriCli) {
    throw "cargo-tauri not found. Install with: cargo install tauri-cli --version '^2' --locked"
}

# Package NSIS bundle with the release version
$tempConfig = Join-Path $root 'target\tauri.release.conf.json'
@{ version = $version } | ConvertTo-Json | Set-Content -Path $tempConfig -Encoding utf8
try {
    & $tauriCli bundle --bundles nsis -c $tempConfig
    if ($LASTEXITCODE -ne 0) { throw "Tauri NSIS bundling failed (exit $LASTEXITCODE)" }
} finally {
    Remove-Item $tempConfig -ErrorAction SilentlyContinue
}

# 8. Package distribution artifacts in dist/
Step 'Packaging dist/ artifacts'
New-Item -ItemType Directory -Force $dist | Out-Null

$setupSrc = @(Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter "*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1)
if (-not $setupSrc) { throw "NSIS installer was not found in target\release\bundle\nsis" }

$setupDest = Join-Path $dist "soritong-$version-windows-x64-setup.exe"
Copy-Item $setupSrc[0].FullName $setupDest -Force

# Portable ZIP package
Step 'Packaging portable ZIP'
$portableStaging = Join-Path $root "target\portable-staging-$version"
if (Test-Path $portableStaging) { Remove-Item $portableStaging -Recurse -Force }
New-Item -ItemType Directory -Force $portableStaging | Out-Null

Copy-Item $builtApp (Join-Path $portableStaging 'soritong.exe') -Force

# Copy all runtime DLLs
Copy-Item $directMl (Join-Path $portableStaging 'DirectML.dll') -Force
foreach ($dll in $vcDlls) {
    Copy-Item $dll.FullName (Join-Path $portableStaging $dll.Name) -Force
}

# Copy documentation
Copy-Item (Join-Path $root 'README.md') (Join-Path $portableStaging 'README.md') -Force
if (Test-Path (Join-Path $root 'README.ko.md')) {
    Copy-Item (Join-Path $root 'README.ko.md') (Join-Path $portableStaging 'README.ko.md') -Force
}
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $portableStaging 'LICENSE') -Force
if (Test-Path (Join-Path $root 'THIRD_PARTY_NOTICES.md')) {
    Copy-Item (Join-Path $root 'THIRD_PARTY_NOTICES.md') (Join-Path $portableStaging 'THIRD_PARTY_NOTICES.md') -Force
}

$zipDest = Join-Path $dist "soritong-$version-windows-x64-portable.zip"
if (Test-Path $zipDest) { Remove-Item $zipDest -Force }
Compress-Archive -Path (Join-Path $portableStaging '*') -DestinationPath $zipDest
Remove-Item $portableStaging -Recurse -Force -ErrorAction SilentlyContinue

# Clean up staged DLLs from crates/soritong-app/resources/ so git status stays clean
Get-ChildItem -Path $resourcesDir -Filter "*.dll" | Remove-Item -Force -ErrorAction SilentlyContinue

# 9. Compute checksums
Step 'Computing SHA256 checksums'
$checksumFiles = @($setupDest, $zipDest)
$sums = foreach ($file in $checksumFiles) {
    '{0}  {1}' -f (Get-FileHash $file -Algorithm SHA256).Hash.ToLower(), (Split-Path -Leaf $file)
}
$sumsPath = Join-Path $dist 'SHA256SUMS.txt'
Set-Content -Path $sumsPath -Value $sums -Encoding ascii

# 10. Summary report
Write-Host ""
Write-Host "soritong v$version release packaging complete:" -ForegroundColor Green
foreach ($file in ($checksumFiles + $sumsPath)) {
    $item = Get-Item $file
    $mb = [math]::Round($item.Length / 1MB, 2)
    Write-Host "  $(Split-Path -Leaf $file) ($mb MB)"
}
