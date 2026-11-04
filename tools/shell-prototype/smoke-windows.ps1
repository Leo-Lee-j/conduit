# Run after verify.py --phase build. Captures only the window owned by this launch.
$ErrorActionPreference = 'Stop'
$taskRepo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$taskBinary = Join-Path $taskRepo 'target/shell-prototype/debug/conduit-shell-prototype.exe'
$taskOutput = Join-Path $PSScriptRoot 'artifacts'
if (-not (Test-Path -LiteralPath $taskBinary)) { throw 'Build the prototype first.' }
New-Item -ItemType Directory -Force -Path $taskOutput | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class ShellProbeWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    public delegate bool EnumCallback(IntPtr handle, IntPtr state);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumCallback callback, IntPtr state);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr handle, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr handle, System.Text.StringBuilder text, int max);
    public static IntPtr FindOwnedWindow(uint process) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((handle, state) => {
            uint owner;
            GetWindowThreadProcessId(handle, out owner);
            if (owner == process) {
                var title = new System.Text.StringBuilder(512);
                GetWindowText(handle, title, title.Capacity);
                if (title.ToString().Contains("Shell prototype")) { found = handle; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr handle, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr handle, out Rect rect);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr handle, int command);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr handle);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr handle, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr handle, IntPtr dc, uint flags);
}
"@
function Save-ProbeWindow([IntPtr] $handle, [string] $filename) {
    $bounds = New-Object ShellProbeWindow+Rect
    if (-not [ShellProbeWindow]::GetWindowRect($handle, [ref] $bounds)) { throw 'GetWindowRect failed' }
    $bitmap = New-Object System.Drawing.Bitmap(($bounds.Right - $bounds.Left), ($bounds.Bottom - $bounds.Top))
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $dc = $graphics.GetHdc()
        try { $captured = [ShellProbeWindow]::PrintWindow($handle, $dc, 2) }
        finally { $graphics.ReleaseHdc($dc) }
        if (-not $captured) { throw 'PrintWindow failed; screenshot is unavailable.' }
        $bitmap.Save((Join-Path $taskOutput $filename), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
$report = [ordered]@{ started_utc = [DateTime]::UtcNow.ToString('o'); window_detected = $false; close_exited = $false; exit_code = $null; screenshots = @(); error = $null }
$probeProcess = $null
try {
    $probeProcess = Start-Process -FilePath $taskBinary -WorkingDirectory $taskRepo -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskOutput 'native-stdout.log') -RedirectStandardError (Join-Path $taskOutput 'native-stderr.log')
    $null = $probeProcess.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        $probeProcess.Refresh()
        if ($probeProcess.HasExited) { throw "Prototype exited before creating its window: $($probeProcess.ExitCode)" }
        $handle = [ShellProbeWindow]::FindOwnedWindow($probeProcess.Id)
        if ($handle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No native window within 30 seconds' }
    $report.window_detected = $true
    [ShellProbeWindow]::ShowWindow($handle, 4) | Out-Null
    $probeProcess.Refresh()
    $report.title = $probeProcess.MainWindowTitle
    Start-Sleep -Milliseconds 700
    Save-ProbeWindow $handle 'native-initial.png'
    $report.screenshots += 'native-initial.png'
    $client = New-Object ShellProbeWindow+Rect
    if (-not [ShellProbeWindow]::GetClientRect($handle, [ref] $client)) { throw 'GetClientRect failed' }
    $scale = [ShellProbeWindow]::GetDpiForWindow($handle) / 96.0
    # Target the known footer in this fixture; no global mouse or keyboard input.
    $pointX = [int](370 * $scale)
    $pointY = [int]($client.Bottom - 17 * $scale)
    $packedPoint = [IntPtr](($pointY -shl 16) -bor $pointX)
    [ShellProbeWindow]::PostMessage($handle, 0x0200, [IntPtr]::Zero, $packedPoint) | Out-Null
    [ShellProbeWindow]::PostMessage($handle, 0x0201, [IntPtr]1, $packedPoint) | Out-Null
    [ShellProbeWindow]::PostMessage($handle, 0x0202, [IntPtr]::Zero, $packedPoint) | Out-Null
    Start-Sleep -Milliseconds 1500
    Save-ProbeWindow $handle 'native-activated.png'
    $report.screenshots += 'native-activated.png'
    # Screenshot contents need visual review; file creation alone does not assert rendering.
    if (-not [ShellProbeWindow]::PostMessage($handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'WM_CLOSE was rejected' }
    $report.close_exited = $probeProcess.WaitForExit(10000)
    if (-not $report.close_exited) { throw 'Window closed but process did not exit within 10 seconds' }
    $report.exit_code = $probeProcess.ExitCode
    if ($report.exit_code -ne 0) { throw "Prototype exit code: $($report.exit_code)" }
} catch {
    $report.error = $_.Exception.Message
} finally {
    if ($null -ne $probeProcess) {
        $probeProcess.Refresh()
        if (-not $probeProcess.HasExited) { $probeProcess | Stop-Process -Force }
        $probeProcess.Dispose()
    }
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $taskOutput 'native-smoke.json') -Encoding utf8
}
$report | ConvertTo-Json -Depth 4
if ($null -ne $report.error) { exit 1 }
