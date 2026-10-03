# Presses keys in a window on the hidden desktop (hidden.ps1's) by posting it the key messages a
# keyboard sends, for testing --window's controls unattended. Waits for the window titled -Title,
# then plays -Script: "KEY:SECONDS" steps, each holding the key that long and letting go for as
# long, KEY a .NET Keys name (Enter, X, Left, ...) or "wait".
#
#   powershell -File tools/gx/keys.ps1 -Script "wait:8,Enter:0.2,wait:2,X:0.2" -Timeout 60
param(
    [string]$Desktop = 'ssbm-hidden',
    [string]$Title = 'ssbm-rs',
    [Parameter(Mandatory = $true)][string]$Script,
    [int]$Timeout = 60
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Keys2 {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr OpenDesktop(string name, int flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desk);
    [DllImport("user32.dll")] static extern bool EnumDesktopWindows(IntPtr desk, EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", SetLastError = true)] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    public static IntPtr Find(string desktop, string title) {
        // DESKTOP_READOBJECTS | DESKTOP_ENUMERATE
        IntPtr desk = OpenDesktop(desktop, 0, false, 0x0001 | 0x0040);
        if (desk == IntPtr.Zero) return IntPtr.Zero;
        IntPtr found = IntPtr.Zero;
        EnumDesktopWindows(desk, (h, l) => {
            var s = new StringBuilder(256);
            GetWindowText(h, s, 256);
            if (s.ToString() == title) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        CloseDesktop(desk);
        return found;
    }
    // WM_KEYDOWN and WM_KEYUP, with the scan code a keyboard's message carries.
    public static string Key(IntPtr h, uint vk, bool down) {
        uint scan = MapVirtualKey(vk, 0);
        uint ext = (vk >= 0x21 && vk <= 0x2E) ? 1u : 0u;
        long l = 1 | (scan << 16) | (ext << 24);
        if (!down) l |= (1L << 30) | (1L << 31);
        bool ok = PostMessage(h, down ? 0x0100u : 0x0101u, (IntPtr)vk, (IntPtr)l);
        return ok ? "" : " (PostMessage failed: " + Marshal.GetLastWin32Error() + ")";
    }
}
'@
$deadline = (Get-Date).AddSeconds($Timeout)
$hwnd = [IntPtr]::Zero
while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 250
    $hwnd = [Keys2]::Find($Desktop, $Title)
}
if ($hwnd -eq [IntPtr]::Zero) { Write-Output "no window titled $Title"; exit 1 }
Write-Output "window $hwnd"
foreach ($step in $Script.Split(',')) {
    $key, $secs = $step.Split(':')
    $ms = [int]([double]$secs * 1000)
    if ($key -eq 'wait') { Start-Sleep -Milliseconds $ms; continue }
    $vk = [uint32][System.Windows.Forms.Keys]::$key
    $e1 = [Keys2]::Key($hwnd, $vk, $true)
    Start-Sleep -Milliseconds $ms
    $e2 = [Keys2]::Key($hwnd, $vk, $false)
    Start-Sleep -Milliseconds $ms
    Write-Output "pressed $key$e1$e2"
}
