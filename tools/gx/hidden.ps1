# Runs a program on a desktop of its own, which nobody sees: its windows never show or take the
# focus. Waits for it, up to -Timeout seconds, or until -WatchDir holds -WatchCount files (then
# stops it), and exits with its exit code (124 when stopped at the timeout, 0 when stopped
# with its files).
#
#   powershell -File tools/gx/hidden.ps1 -Timeout 120 -WatchDir D -WatchCount 4 -Exe C:\path\Dolphin.exe -CommandArgs 'args...'
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [int]$Timeout = 120,
    [string]$WatchDir = '',
    [int]$WatchCount = 0,
    [string]$CommandArgs = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hidden {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO {
        public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2; public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr devmode, int flags, uint access, IntPtr sa);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string app, string cmd, IntPtr pa, IntPtr ta, bool inherit, int flags,
        IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr h, uint code);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    public static int Run(string exe, string cmdline, string dir, int timeoutMs, string watchDir, int watchCount) {
        // GENERIC_ALL on a desktop of our own in this window station.
        IntPtr desk = CreateDesktop("ssbm-hidden", IntPtr.Zero, IntPtr.Zero, 0, 0x10000000, IntPtr.Zero);
        if (desk == IntPtr.Zero) throw new Exception("CreateDesktop failed: " + Marshal.GetLastWin32Error());
        var si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(si);
        si.lpDesktop = "ssbm-hidden";
        PROCESS_INFORMATION pi;
        if (!CreateProcess(exe, cmdline, IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, dir, ref si, out pi))
            throw new Exception("CreateProcess failed: " + Marshal.GetLastWin32Error());
        uint code = 124;
        bool exited = false;
        for (int waited = 0; waited < timeoutMs; waited += 250) {
            if (WaitForSingleObject(pi.hProcess, 250) == 0) { exited = true; break; }
            if (watchCount > 0 && System.IO.Directory.Exists(watchDir)
                && System.IO.Directory.GetFiles(watchDir).Length >= watchCount) {
                // The last file may still be being written.
                System.Threading.Thread.Sleep(1000);
                code = 0;
                break;
            }
        }
        if (exited) {
            GetExitCodeProcess(pi.hProcess, out code);
        } else {
            TerminateProcess(pi.hProcess, 1);
            WaitForSingleObject(pi.hProcess, 5000);
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        CloseDesktop(desk);
        return (int)code;
    }
}
'@
$cmdline = '"' + $Exe + '" ' + $CommandArgs
exit [Hidden]::Run($Exe, $cmdline, (Split-Path $Exe), $Timeout * 1000, $WatchDir, $WatchCount)
