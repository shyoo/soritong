<#
.SYNOPSIS
  Builds and starts the soritong app for scripted checks, isolated from your own settings.

.DESCRIPTION
  Settings go to -SettingsDir (default: %TEMP%\soritong-try), the WebView2 page is exposed to
  CDP on -Port (used by scripts/*.mjs), and -Demo adds the debug-only fake cloud engine with a
  secret field. The app runs detached; -Stop closes it the way the user would (WM_CLOSE), which
  is what saves the window geometry.

.EXAMPLE
  scripts/run-app.ps1 -Demo; node scripts/app-smoke.mjs shot.png; scripts/run-app.ps1 -Stop
#>
param(
  [string]$SettingsDir = (Join-Path $env:TEMP "soritong-try"),
  [int]$Port = 9333,
  [switch]$Demo,
  [switch]$NoBuild,
  [switch]$Stop
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

function Stop-App {
  $procs = @(Get-Process soritong-app -ErrorAction SilentlyContinue)
  foreach ($p in $procs) { [void]$p.CloseMainWindow() }
  foreach ($p in $procs) { if (-not $p.WaitForExit(5000)) { $p.Kill() } }
  "stopped $($procs.Count) instance(s)"
}

if ($Stop) { Stop-App; return }
if (Get-Process soritong-app -ErrorAction SilentlyContinue) { Stop-App }

if (-not $NoBuild) {
  cargo build -p soritong-app --manifest-path (Join-Path $root "Cargo.toml")
  if ($LASTEXITCODE) { throw "build failed" }
}
New-Item -ItemType Directory -Force $SettingsDir | Out-Null
$env:SORITONG_SETTINGS = Join-Path $SettingsDir "settings.json"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
$env:SORITONG_CDP_PORT = "$Port"
if ($Demo) { $env:SORITONG_DEMO_ENGINES = "1" } else { Remove-Item Env:SORITONG_DEMO_ENGINES -ErrorAction SilentlyContinue }

$exe = Join-Path $root "target\debug\soritong-app.exe"
$log = Join-Path $SettingsDir "app.log"
# Not Start-Process -Redirect*: that makes the app inherit our stdout, so a caller reading this
# script's output (`run-app.ps1 | ...`, agent shells) hangs until the app exits. A hidden cmd
# started by ShellExecute inherits nothing and still captures the log.
Start-Process cmd -WindowStyle Hidden -ArgumentList "/c", "`"`"$exe`" > `"$log`" 2>&1`""
$p = $null
for ($i = 0; $i -lt 50 -and -not $p; $i++) { Start-Sleep -Milliseconds 200; $p = Get-Process soritong-app -ErrorAction SilentlyContinue | Select-Object -First 1 }
if (-not $p) { throw "soritong-app did not start; see $log" }
"started pid $($p.Id); settings $env:SORITONG_SETTINGS; CDP :$Port; log $log"
