# Integration smoke test for soritong NSIS installer.
[CmdletBinding()]
param([string]$Version)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'version.ps1')
$Version = Get-SoritongVersion $Version

$setup = Join-Path $root "dist\soritong-$Version-windows-x64-setup.exe"
if (-not (Test-Path $setup)) {
    throw "Installer not found at $setup. Run ./scripts/release.ps1 first."
}

$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\soritong'
if (Test-Path $key) {
    throw "An existing soritong installation is registered at $key; smoke test will not replace it"
}

$destination = Join-Path $root "target\installer-smoke\$([guid]::NewGuid())"
[IO.Directory]::CreateDirectory($destination) | Out-Null

function Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }

try {
    Step "Installing silently to $destination"
    $proc = Start-Process -FilePath $setup -ArgumentList "/S", "/D=$destination" -PassThru
    if (-not $proc.WaitForExit(60000)) {
        $proc | Stop-Process -Force
        throw 'Installer smoke test timed out'
    }
    if ($proc.ExitCode -ne 0) {
        throw "Installer failed with exit code $($proc.ExitCode)"
    }

    Step "Verifying registry registration"
    if (-not (Test-Path $key)) {
        throw "Uninstall registry key $key was not created"
    }
    $reg = Get-ItemProperty $key
    if ($reg.DisplayVersion -cne $Version) {
        throw "Installed version mismatch: expected $Version, found $($reg.DisplayVersion)"
    }

    Step "Verifying installed payloads and runtime DLLs"
    $installedExe = Join-Path $destination 'soritong-app.exe'
    if (-not (Test-Path $installedExe)) {
        throw "Installed executable not found at $installedExe"
    }
    $directMl = Join-Path $destination 'DirectML.dll'
    if (-not (Test-Path $directMl)) {
        throw "DirectML.dll is missing next to installed exe"
    }
    $msvcp = Join-Path $destination 'msvcp140.dll'
    if (-not (Test-Path $msvcp)) {
        throw "msvcp140.dll is missing next to installed exe"
    }

    Step "Briefly launching installed application"
    $appProc = Start-Process -FilePath $installedExe -PassThru
    Start-Sleep -Seconds 2
    $alive = Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue
    if (-not $alive) {
        throw "Installed executable exited unexpectedly upon launch"
    }
    $appProc | Stop-Process -Force
    Start-Sleep -Milliseconds 300
} finally {
    $uninstaller = Join-Path $destination 'uninstall.exe'
    if (Test-Path $uninstaller) {
        Step "Running uninstaller silently"
        $unProc = Start-Process -FilePath $uninstaller -ArgumentList "/S", "_?=$destination" -PassThru
        if (-not $unProc.WaitForExit(30000) -or $unProc.ExitCode -ne 0) {
            throw "Uninstaller failed with exit code $($unProc.ExitCode)"
        }
        for ($i = 0; $i -lt 50 -and (Test-Path $key); $i++) { Start-Sleep -Milliseconds 200 }
    }
    if (Test-Path $destination) {
        Remove-Item $destination -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if (Test-Path $key) {
    throw "Uninstall registry key remained after uninstallation"
}
if (Test-Path $installedExe) {
    throw "Installed executable remained after uninstallation"
}

Write-Host "Installer silent install, registry, runtime DLLs, process launch, and uninstall smoke tests passed cleanly." -ForegroundColor Green
