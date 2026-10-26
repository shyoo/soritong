<#
.SYNOPSIS
  Checks the global "show window" shortcut: minimizes soritong, presses the shortcut, and
  reports whether the window came back in front. Default Ctrl+Alt+Space.

.DESCRIPTION
  The shortcut is a registered hotkey, so the OS consumes it and no other app sees the keys.
  Pair with `node -e` / scripts/cdp.mjs to confirm the input box got focus.
#>
param([byte[]]$Modifiers = @(0x11, 0x12), [byte]$Key = 0x20)   # VK_CONTROL, VK_MENU; VK_SPACE
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SoriHotkey {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
}
"@
$proc = Get-Process soritong-app -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { throw "soritong-app is not running (scripts/run-app.ps1)" }
$h = [IntPtr]$proc.MainWindowHandle

[void][SoriHotkey]::ShowWindow($h, 6); Start-Sleep -Milliseconds 500   # SW_MINIMIZE
"minimized: $([SoriHotkey]::IsIconic($h))"
$down = @($Modifiers) + $Key
foreach ($vk in $down) { [SoriHotkey]::keybd_event($vk, 0, 0, [UIntPtr]::Zero) }
for ($i = $down.Count - 1; $i -ge 0; $i--) { [SoriHotkey]::keybd_event($down[$i], 0, 2, [UIntPtr]::Zero) }
Start-Sleep -Milliseconds 800
$ok = -not [SoriHotkey]::IsIconic($h) -and ([SoriHotkey]::GetForegroundWindow() -eq $h)
"after shortcut: restored and in front: $ok"
if (-not $ok) { exit 1 }
