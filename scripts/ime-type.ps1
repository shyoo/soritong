<#
.SYNOPSIS
  Types into soritong through the REAL Windows Korean IME (Microsoft IME, 2-set layout).

.DESCRIPTION
  Brings the soritong window to the front, switches it to the Korean layout and Hangul mode,
  and sends real keystrokes (keybd_event), so the IME composes exactly as for a person. Pair
  with `node scripts/ime-watch.mjs arm|read` to see the events and the result.

  -Keys uses 2-set (두벌식) letters plus tokens: {ENTER} {SHIFTENTER} {ESC} {UP}.
    안녕하세요 = dkssudgktpdy   감사합니다 = rkatkgkqslek   네 = sp   첫 줄 = cjt wnf

  ⚠ This injects real keyboard input. Before every key it checks that soritong is still the
  foreground window and aborts otherwise, but don't run it while someone is using the machine:
  their keystrokes mix into the test (seen as stray Backspace/space/jamo in the event log).
  Needs the Korean language (Microsoft IME) installed.

.EXAMPLE
  node scripts/ime-watch.mjs arm; scripts/ime-type.ps1 -Keys "dkssudgktpdy{ENTER}"; node scripts/ime-watch.mjs read
#>
param([Parameter(Mandatory)][string]$Keys, [int]$DelayMs = 60)
$ErrorActionPreference = "Stop"

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SoriIme {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("imm32.dll")] public static extern IntPtr ImmGetDefaultIMEWnd(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l, t, r, b; }
  [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO {
    public int cb; public int flags; public IntPtr active, focus, capture, menuOwner, moveSize, caret; public RECT rc; }
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint tid, ref GUITHREADINFO g);
}
"@

# Not FindWindow($null, ...): PowerShell marshals $null to "" there, which never matches.
$proc = Get-Process soritong-app -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { throw "soritong-app is not running (scripts/run-app.ps1)" }
$h = [IntPtr]$proc.MainWindowHandle

function Guard { if ([SoriIme]::GetForegroundWindow() -ne $h) { throw "soritong is not the foreground window; aborting so no key lands elsewhere" } }
function Tap([byte]$vk) {
  Guard
  [SoriIme]::keybd_event($vk, 0, 0, [UIntPtr]::Zero); [SoriIme]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds $DelayMs
}
function ImeMode {
  $g = New-Object SoriIme+GUITHREADINFO; $g.cb = [Runtime.InteropServices.Marshal]::SizeOf($g)
  [void][SoriIme]::GetGUIThreadInfo(0, [ref]$g)
  $ime = [SoriIme]::ImmGetDefaultIMEWnd($g.focus)
  [SoriIme]::SendMessage($ime, 0x283, [IntPtr]1, [IntPtr]::Zero).ToInt64()   # WM_IME_CONTROL / IMC_GETCONVERSIONMODE
}

# A bare Alt tap lets a background process take the foreground.
[SoriIme]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [SoriIme]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
[void][SoriIme]::SetForegroundWindow($h); Start-Sleep -Milliseconds 300
Guard
[void][SoriIme]::PostMessage($h, 0x50, [IntPtr]::Zero, [IntPtr]0x04120412)   # WM_INPUTLANGCHANGEREQUEST: Korean
Start-Sleep -Milliseconds 300
if ((ImeMode) -band 1) { } else { Tap 0x15; Start-Sleep -Milliseconds 200 }   # VK_HANGUL into Hangul mode
if (-not ((ImeMode) -band 1)) { throw "could not switch the IME to Hangul mode" }
# Give the mode switch time to settle, or the first letters arrive as Latin.
Start-Sleep -Milliseconds 300

foreach ($tok in [regex]::Matches($Keys, '\{\w+\}|.')) {
  switch -CaseSensitive ($tok.Value) {
    '{ENTER}' { Tap 0x0D }
    '{ESC}' { Tap 0x1B }
    '{UP}' { Tap 0x26 }
    '{SHIFTENTER}' {
      Guard; [SoriIme]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
      try { Tap 0x0D } finally { [SoriIme]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero) }
    }
    ' ' { Tap 0x20 }
    default {
      $c = $tok.Value[0]
      if ([char]::IsUpper($c)) {   # 2-set double consonants/vowels (ㅃ ㅉ ㄸ ㄲ ㅆ ㅒ ㅖ) are Shift+letter
        Guard; [SoriIme]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
        try { Tap ([byte]$c) } finally { [SoriIme]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero) }
      } else { Tap ([byte][char]::ToUpper($c)) }
    }
  }
}
"typed $($Keys.Length) chars"
