# Run after verify.py --phase build. Captures only the window owned by this launch.
param([switch] $Development, [switch] $Desktop, [switch] $CaptureOnly, [switch] $Overlays)
$ErrorActionPreference = 'Stop'
if ($Overlays -and -not $Desktop) { throw '-Overlays requires -Desktop' }
$taskRepo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$taskExecutable = if ($Desktop) { 'conduit-shell-desktop.exe' } else { 'conduit-shell-prototype.exe' }
$taskBinary = Join-Path $taskRepo "target/shell-prototype/debug/$taskExecutable"
$taskOutput = Join-Path $PSScriptRoot 'artifacts'
$taskPrefix = if ($Overlays) { 'desktop-overlays' } elseif ($Desktop) { 'desktop-native' } elseif ($Development) { 'development-native' } else { 'native' }
$taskTitle = if ($Desktop) { 'Conduit' } elseif ($Development) { 'Development plugin' } else { 'Shell prototype' }
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
    public static IntPtr FindOwnedWindow(uint process, string expectedTitle) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((handle, state) => {
            uint owner;
            GetWindowThreadProcessId(handle, out owner);
            if (owner == process) {
                var title = new System.Text.StringBuilder(512);
                GetWindowText(handle, title, title.Capacity);
                if (title.ToString().Contains(expectedTitle)) { found = handle; return false; }
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
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr handle, IntPtr after, int x, int y, int width, int height, uint flags);
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
function Click-ProbeWindow([IntPtr] $handle, [int] $pointX, [int] $pointY) {
    $packedPoint = [IntPtr](($pointY -shl 16) -bor $pointX)
    foreach ($message in @(0x0200, 0x0201, 0x0202)) {
        $button = if ($message -eq 0x0201) { [IntPtr]1 } else { [IntPtr]::Zero }
        if (-not [ShellProbeWindow]::PostMessage($handle, $message, $button, $packedPoint)) { throw 'Window click rejected' }
    }
}
$report = [ordered]@{ started_utc = [DateTime]::UtcNow.ToString('o'); window_detected = $false; close_exited = $false; exit_code = $null; screenshots = @(); error = $null }
$probeProcess = $null
$taskOriginalData = $env:CONDUIT_DATA_DIR
try {
    if ($Desktop) {
        $taskData = Join-Path $taskRepo ("target/shell-desktop-native-" + [Guid]::NewGuid().ToString('N'))
        $taskWorkspace = Join-Path $taskData 'native-workspace'
        New-Item -ItemType Directory -Force -Path $taskWorkspace | Out-Null
        $taskState = @{
            version = 3
            workspaces = @(@{ path = $taskWorkspace; sessions = @() })
            updates = @{ auto_check = $false; auto_download = $false; auto_install = $false }
        }
        [System.IO.File]::WriteAllText((Join-Path $taskData 'state.json'), ($taskState | ConvertTo-Json -Depth 5), [System.Text.UTF8Encoding]::new($false))
        $env:CONDUIT_DATA_DIR = $taskData
        $report.data_dir = $taskData
    }
    $taskLaunch = @{
        FilePath = $taskBinary; WorkingDirectory = $taskRepo; WindowStyle = 'Hidden'; PassThru = $true
        RedirectStandardOutput = Join-Path $taskOutput "$taskPrefix-stdout.log"
        RedirectStandardError = Join-Path $taskOutput "$taskPrefix-stderr.log"
    }
    if ($Development -or $Desktop) {
        $taskFixture = Join-Path $PSScriptRoot 'fixtures/development-overview'
        $taskLaunch.ArgumentList = @('--dev-plugin', ('"' + $taskFixture + '"'), '--allow-permission', 'workspace.metadata.read', '--allow-permission', 'session.summary.read')
    }
    $probeProcess = Start-Process @taskLaunch
    $null = $probeProcess.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        $probeProcess.Refresh()
        if ($probeProcess.HasExited) { throw "Prototype exited before creating its window: $($probeProcess.ExitCode)" }
        $handle = [ShellProbeWindow]::FindOwnedWindow($probeProcess.Id, $taskTitle)
        if ($handle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($handle -eq [IntPtr]::Zero) { throw 'No native window within 30 seconds' }
    $report.window_detected = $true
    [ShellProbeWindow]::ShowWindow($handle, 4) | Out-Null
    $probeProcess.Refresh()
    $report.title = $probeProcess.MainWindowTitle
    Start-Sleep -Milliseconds 700
    if ($Desktop) { Start-Sleep -Milliseconds 2000 }
    Save-ProbeWindow $handle "$taskPrefix-initial.png"
    $report.screenshots += "$taskPrefix-initial.png"
    $client = New-Object ShellProbeWindow+Rect
    if (-not [ShellProbeWindow]::GetClientRect($handle, [ref] $client)) { throw 'GetClientRect failed' }
    $scale = [ShellProbeWindow]::GetDpiForWindow($handle) / 96.0
    if (-not $CaptureOnly) {
        # Target only this process's window; no global mouse or keyboard input.
        $pointX = if ($Desktop) { [int](20 * $scale) } elseif ($Development) { [int](140 * $scale) } else { [int](370 * $scale) }
        $pointY = if ($Desktop) { [int](96 * $scale) } elseif ($Development) { [int](64 * $scale) } else { [int]($client.Bottom - 17 * $scale) }
        Click-ProbeWindow $handle $pointX $pointY
        Start-Sleep -Milliseconds 1500
        Save-ProbeWindow $handle "$taskPrefix-activated.png"
        $report.screenshots += "$taskPrefix-activated.png"
        # The second footer button targets the summary without selecting its view.
        $pointX = if ($Desktop) { [int](620 * $scale) } elseif ($Development) { [int](370 * $scale) } else { [int](550 * $scale) }
        $pointY = [int]($client.Bottom - 17 * $scale)
        Click-ProbeWindow $handle $pointX $pointY
        Start-Sleep -Milliseconds 700
        Save-ProbeWindow $handle "$taskPrefix-command.png"
        $report.screenshots += "$taskPrefix-command.png"
        if ($Desktop) {
            $bounds = New-Object ShellProbeWindow+Rect
            if (-not [ShellProbeWindow]::GetWindowRect($handle, [ref] $bounds)) { throw 'GetWindowRect failed' }
            $outerWidth = [int](850 * $scale) + $bounds.Right - $bounds.Left - $client.Right
            if (-not [ShellProbeWindow]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, $outerWidth, ($bounds.Bottom - $bounds.Top), 0x0016)) { throw 'Window resize failed' }
            Start-Sleep -Milliseconds 700
            Save-ProbeWindow $handle "$taskPrefix-narrow.png"
            $report.screenshots += "$taskPrefix-narrow.png"
            if (-not [ShellProbeWindow]::GetClientRect($handle, [ref] $client)) { throw 'GetClientRect failed' }
            Click-ProbeWindow $handle ([int]($client.Right - 164 * $scale)) ([int](16 * $scale))
            Start-Sleep -Milliseconds 500
            Save-ProbeWindow $handle "$taskPrefix-settings.png"
            $report.screenshots += "$taskPrefix-settings.png"
            foreach ($message in @(0x0100, 0x0101)) {
                if (-not [ShellProbeWindow]::PostMessage($handle, $message, [IntPtr]27, [IntPtr]::Zero)) { throw 'Escape key rejected' }
            }
            Start-Sleep -Milliseconds 300
        }
        if ($Overlays) {
            # Open actual script overlays, unwind them with Escape, then leave
            # both open for the real WM_CLOSE shutdown path below.
            Click-ProbeWindow $handle ([int]($client.Right - 170 * $scale)) ([int](260 * $scale))
            Start-Sleep -Milliseconds 700
            Save-ProbeWindow $handle "$taskPrefix-dialog.png"
            $report.screenshots += "$taskPrefix-dialog.png"
            foreach ($message in @(0x0100, 0x0101)) {
                if (-not [ShellProbeWindow]::PostMessage($handle, $message, [IntPtr]27, [IntPtr]::Zero)) { throw 'Escape key rejected' }
            }
            Start-Sleep -Milliseconds 400
            # Toasts sit above modal content. Dismiss this fixture's toast so
            # the sheet's title can be inspected without occlusion.
            Click-ProbeWindow $handle ([int]($client.Right - 170 * $scale)) ([int](40 * $scale))
            Start-Sleep -Milliseconds 400
            Save-ProbeWindow $handle "$taskPrefix-sheet.png"
            $report.screenshots += "$taskPrefix-sheet.png"
            foreach ($message in @(0x0100, 0x0101)) {
                if (-not [ShellProbeWindow]::PostMessage($handle, $message, [IntPtr]27, [IntPtr]::Zero)) { throw 'Escape key rejected' }
            }
            Start-Sleep -Milliseconds 400
            Save-ProbeWindow $handle "$taskPrefix-dismissed.png"
            $report.screenshots += "$taskPrefix-dismissed.png"
            Click-ProbeWindow $handle ([int]($client.Right - 170 * $scale)) ([int](260 * $scale))
            Start-Sleep -Milliseconds 500
            Save-ProbeWindow $handle "$taskPrefix-before-close.png"
            $report.screenshots += "$taskPrefix-before-close.png"
        } elseif ($Development -or $Desktop) {
            $stopX = if ($Desktop) { [int](100 * $scale) } else { [int](260 * $scale) }
            $stopY = if ($Desktop) { [int](54 * $scale) } else { [int](24 * $scale) }
            Click-ProbeWindow $handle $stopX $stopY
            Start-Sleep -Milliseconds 500
            Save-ProbeWindow $handle "$taskPrefix-stopped.png"
            $report.screenshots += "$taskPrefix-stopped.png"
        }
    }
    # Screenshot contents need visual review; file creation alone does not assert rendering.
    if (-not [ShellProbeWindow]::PostMessage($handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'WM_CLOSE was rejected' }
    $report.close_exited = $probeProcess.WaitForExit(10000)
    if (-not $report.close_exited) { throw 'Window closed but process did not exit within 10 seconds' }
    $report.exit_code = $probeProcess.ExitCode
    if ($report.exit_code -ne 0) { throw "Prototype exit code: $($report.exit_code)" }
} catch {
    $report.error = $_.Exception.Message
} finally {
    $env:CONDUIT_DATA_DIR = $taskOriginalData
    if ($null -ne $probeProcess) {
        $probeProcess.Refresh()
        if (-not $probeProcess.HasExited) { $probeProcess | Stop-Process -Force }
        $probeProcess.Dispose()
    }
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $taskOutput "$taskPrefix-smoke.json") -Encoding utf8
}
$report | ConvertTo-Json -Depth 4
if ($null -ne $report.error) { exit 1 }
