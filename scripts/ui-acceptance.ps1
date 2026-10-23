# NOTE: Save this file as UTF-8 with BOM. Windows PowerShell 5.1 otherwise reads the
# Chinese seed strings in the system ANSI code page and fails to parse the script.
<#
.SYNOPSIS
  Repeatable UI acceptance harness for the Conduit Chat M0 prototype.

.DESCRIPTION
  Actions:
    seed     Write a deterministic state.json (one workspace, one Claude and one
             Codex session with synthetic Markdown/tool history) into an isolated
             data directory. Never touches %LOCALAPPDATA%\Conduit or .ccgui data.
    capture  Launch target\debug\conduit.exe with CONDUIT_DATA_DIR pointing at the
             isolated data directory, optionally resize the window, optionally
             invoke UI Automation buttons by name, then save a PNG screenshot and
             a UIA snapshot for review.

  This harness only produces evidence for manual acceptance; it does not replace
  the human acceptance sign-off in docs/m0-validation.md.

.EXAMPLE
  # Seed an isolated data directory, then capture the Chat view.
  ./scripts/ui-acceptance.ps1 -Action seed
  ./scripts/ui-acceptance.ps1 -Action capture -Name chat-empty -Seed

.EXAMPLE
  # Capture the settings page and open the model sync page first.
  ./scripts/ui-acceptance.ps1 -Action capture -Name settings-models -AppArgs '--settings' -ClickButton '模型同步'

.EXAMPLE
  # Capture the narrow layout at the documented minimum window size.
  ./scripts/ui-acceptance.ps1 -Action capture -Name chat-narrow -Width 850 -Height 500
#>
param(
    [ValidateSet('seed', 'capture')]
    [string]$Action = 'capture',
    [string]$Name = 'capture',
    [string]$DataDir = '',
    [string]$OutDir = '',
    [string]$WorkspacePath = '',
    [string]$AppArgs = '',
    [string[]]$ClickButton = @(),
    [int[]]$ClickButtonIndex = @(),
    [string[]]$PreClickAt = @(),
    [string[]]$ClickAt = @(),
    [string[]]$PostClickAt = @(),
    [string[]]$PostClickButton = @(),
    [string[]]$SendClickAt = @(),
    [string[]]$ScrollAt = @(),
    [string[]]$ClickAtAfterScroll = @(),
    [string[]]$DragAt = @(),
    [string]$TypeText = '',
    [string]$UiaSetText = '',
    [string]$SendKeys = '',
    [int]$FocusEditIndex = -1,
    [switch]$PasteClipboard,
    [int]$Width = 0,
    [int]$Height = 0,
    [int]$SettleMs = 800,
    [int]$WaitSeconds = 0,
    [ValidateSet('all', 'claude', 'codex')]
    [string]$SeedProfile = 'all',
    [switch]$Seed,
    [switch]$KeepRunning
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($DataDir -eq '') { $DataDir = Join-Path $root 'target\ui-acceptance\data' }
if ($OutDir -eq '') { $OutDir = Join-Path $root 'target\ui-acceptance\out' }
if ($WorkspacePath -eq '') { $WorkspacePath = Join-Path $root 'target\ui-acceptance\workspace' }
$DataDir = [IO.Path]::GetFullPath($DataDir)
$OutDir = [IO.Path]::GetFullPath($OutDir)
$WorkspacePath = [IO.Path]::GetFullPath($WorkspacePath)

function Invoke-Seed {
    $null = New-Item -ItemType Directory -Force -Path $WorkspacePath, $DataDir
    $statePath = Join-Path $DataDir 'state.json'

    $markdown = @'
## 验收：Markdown 渲染

这是一段用于验收的**合成演示**回复，不是真实 CLI 输出。

- 行内代码：`cargo test --locked`
- 中文、标点与 emoji：🙂 ✅
- 无序列表第二项

```rust
fn main() {
    println!("conduit acceptance");
}
```

> 引用块：用于检查文本换行与缩进。
'@

    $longReply = @'
### 长回复与滚动验收

以下内容是合成的演示文本，用于检查长回复的滚动跟随与阅读体验。

1. 第一段：项目按工作区组织会话，同一工作区同一时间只允许一个活动任务。
2. 第二段：切换会话会保留草稿，后台会话的完成事件不应拉动当前会话的滚动条。
3. 第三段：工具执行详情在右侧面板展示，窄窗口时覆盖显示并可独立关闭。
4. 第四段：审批请求在输入区上方排队，逐项处理并显示待审批数量。
5. 第五段：取消任务会结束整个进程树，界面状态不得先于本地进程停止。

'@ + ((1..24 | ForEach-Object { "- 追加行 $_：用于制造可滚动的长内容，滚动跟随只应在停留底部时生效。`n" }) -join '') + @'

结束段：如果你滚动到这里，说明长内容完整渲染。
'@

    $state = [ordered]@{
        version    = 2
        workspaces = @(
            [ordered]@{
                path     = $WorkspacePath
                sessions = @(
                    [ordered]@{
                        id          = 1
                        provider    = 'Claude'
                        created_at  = 1792651200
                        exit_code   = $null
                        title       = '验收：Markdown 与工具卡片'
                        native_id   = $null
                        model       = 'gpt-6.1-sol[1M]'
                        interrupted = $false
                        messages    = @(
                            [ordered]@{
                                key    = 'user-1'
                                kind   = 'User'
                                text   = '请检查这段代码，并用 Markdown 汇总结果。'
                                input  = ''
                                detail = ''
                                status = ''
                            },
                            [ordered]@{
                                key    = 'assistant-1'
                                kind   = 'Assistant'
                                text   = $markdown
                                input  = ''
                                detail = ''
                                status = ''
                            },
                            [ordered]@{
                                key    = 'tool-bash-1'
                                kind   = 'Tool'
                                text   = 'Bash · cargo test'
                                input  = "{\n  `"command`": `"cargo test --locked`"\n}"
                                detail = "running 33 tests`n... ok`ntest result: ok. 33 passed; 0 failed; 8 ignored"
                                status = '完成'
                            },
                            [ordered]@{
                                key    = 'assistant-2'
                                kind   = 'Assistant'
                                text   = '工具执行结果正常，回复用于验收工具卡片与最终快照的拼接。'
                                input  = ''
                                detail = ''
                                status = ''
                            }
                        )
                    },
                    [ordered]@{
                        id          = 2
                        provider    = 'Codex'
                        created_at  = 1792651260
                        exit_code   = $null
                        title       = '验收：长回复与滚动'
                        native_id   = $null
                        model       = 'gpt-5.6-sol'
                        interrupted = $false
                        messages    = @(
                            [ordered]@{
                                key    = 'user-2'
                                kind   = 'User'
                                text   = '生成一段用于检查滚动跟随的长回复。'
                                input  = ''
                                detail = ''
                                status = ''
                            },
                            [ordered]@{
                                key    = 'assistant-3'
                                kind   = 'Assistant'
                                text   = $longReply
                                input  = ''
                                detail = ''
                                status = ''
                            }
                        )
                    }
                )
            }
        )
        cli        = [ordered]@{
            claude_path = $null
            codex_path  = $null
        }
    }

    if ($SeedProfile -ne 'all') {
        $provider = if ($SeedProfile -eq 'claude') { 'Claude' } else { 'Codex' }
        $state.workspaces[0].sessions = @($state.workspaces[0].sessions | Where-Object { $_.provider -eq $provider })
    }
    $json = $state | ConvertTo-Json -Depth 12
    [IO.File]::WriteAllText($statePath, $json, [Text.UTF8Encoding]::new($false))
    Write-Output "SEEDED state=$statePath profile=$SeedProfile"
    Write-Output "SEEDED workspace=$WorkspacePath"
}

function Wait-MainWindow($process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $process.Refresh()
        if ($process.HasExited) { throw "Conduit exited during startup ($($process.ExitCode))" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $process.MainWindowTitle -eq 'Conduit' -and $process.Responding) {
            return $process.MainWindowHandle
        }
        Start-Sleep -Milliseconds 150
    }
    throw 'Conduit did not expose a responding window within 20 seconds.'
}

function Get-ElementName($element) {
    try { return [string]$element.Current.Name } catch { return '' }
}

function Set-TargetForeground($handle) {
    [ConduitWin]::ShowWindow($handle, 9) | Out-Null
    # HWND_TOP, SWP_NOMOVE | SWP_NOSIZE
    [ConduitWin]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, 0, 0, 0x0003) | Out-Null
    if ([ConduitWin]::GetForegroundWindow() -ne $handle) {
        $foregroundPid = [uint32]0
        $foregroundThread = [ConduitWin]::GetWindowThreadProcessId([ConduitWin]::GetForegroundWindow(), [ref]$foregroundPid)
        $currentThread = [ConduitWin]::GetCurrentThreadId()
        [ConduitWin]::AttachThreadInput($currentThread, $foregroundThread, $true) | Out-Null
        [ConduitWin]::SetForegroundWindow($handle) | Out-Null
        [ConduitWin]::SetFocus($handle) | Out-Null
        [ConduitWin]::AttachThreadInput($currentThread, $foregroundThread, $false) | Out-Null
    }
    Start-Sleep -Milliseconds 200
}

if ($Action -eq 'seed' -or $Seed) { Invoke-Seed }
if ($Action -eq 'seed') { exit 0 }

# -File argument parsing joins comma lists; allow "x,y;x,y" in one value.
$ClickButton = @($ClickButton | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$ClickAt = @($ClickAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$PreClickAt = @($PreClickAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$PostClickAt = @($PostClickAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$PostClickButton = @($PostClickButton | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$ScrollAt = @($ScrollAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$ClickAtAfterScroll = @($ClickAtAfterScroll | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$DragAt = @($DragAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })

$exe = Join-Path $root 'target\debug\conduit.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Run cargo build --locked first.' }
if (-not (Test-Path -LiteralPath (Join-Path $DataDir 'state.json'))) {
    throw "No state.json in $DataDir; run -Action seed first."
}
$null = New-Item -ItemType Directory -Force -Path $OutDir, $DataDir
$png = Join-Path $OutDir ($Name + '.png')
$uiaPath = Join-Path $OutDir ($Name + '-uia.txt')
$metaPath = Join-Path $OutDir ($Name + '.json')

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ConduitWin {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, int dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(uint bVk, uint bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool attach);
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$previousData = [Environment]::GetEnvironmentVariable('CONDUIT_DATA_DIR', 'Process')
$process = $null
try {
    $env:CONDUIT_DATA_DIR = $DataDir
    $launch = @{
        FilePath         = $exe
        WorkingDirectory = $root
        WindowStyle      = 'Normal'
        PassThru         = $true
        # Keep child stdout/stderr for diagnosing crashes during automation.
        RedirectStandardOutput = (Join-Path $OutDir ($Name + '-stdout.log'))
        RedirectStandardError  = (Join-Path $OutDir ($Name + '-stderr.log'))
    }
    if ($AppArgs -ne '') { $launch.ArgumentList = $AppArgs }
    $process = Start-Process @launch
    $handle = Wait-MainWindow $process
    Start-Sleep -Milliseconds 600
    # Foreground activation can be refused by the shell while another window
    # owns the foreground lock; attach to its thread to force activation so
    # synthetic clicks and keyboard events reach the app under test.
    Set-TargetForeground $handle
    # A settings modal or popover needs the first frame to settle before
    # synthetic clicks can hit its widgets reliably.
    Start-Sleep -Milliseconds 900

    if ($Width -gt 0 -and $Height -gt 0) {
        # SWP_NOMOVE (0x2) | SWP_NOZORDER (0x4) | SWP_NOACTIVATE (0x10)
        [ConduitWin]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, $Width, $Height, 0x16) | Out-Null
        Start-Sleep -Milliseconds 500
    }

    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $auto = [System.Windows.Automation.AutomationElement]
    $rootElement = $auto::FromHandle($handle)
    $buttonCondition = New-Object System.Windows.Automation.PropertyCondition($auto::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $editCondition = New-Object System.Windows.Automation.PropertyCondition($auto::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)
    $textCondition = New-Object System.Windows.Automation.PropertyCondition($auto::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    $edits = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
    $texts = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCondition)

    $rect = New-Object ConduitWin+RECT
    [ConduitWin]::GetWindowRect($handle, [ref]$rect) | Out-Null

    if ($FocusEditIndex -ge 0) {
        if ($FocusEditIndex -ge $edits.Count) { throw "FocusEditIndex out of range: $FocusEditIndex" }
        try {
            $edits.Item($FocusEditIndex).SetFocus()
        } catch {
            Write-Output "FOCUS_EDIT_ERROR=$($_.Exception.Message)"
        }
        Start-Sleep -Milliseconds 300
    }

    if ($UiaSetText -ne '') {
        if ($edits.Count -eq 0) { throw 'No Edit element found for -UiaSetText' }
        $edit = $edits.Item(0)
        try {
            $valuePattern = $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
            $valuePattern.SetValue($UiaSetText)
        } catch {
            throw "The edit element does not expose ValuePattern: $($_.Exception.Message)"
        }
        Start-Sleep -Milliseconds 400
    }

    # The accessibility tree is occasionally empty right after the first frame;
    # poll briefly so index-based and name-based clicks stay reliable.
    for ($attempt = 0; $attempt -lt 20 -and $buttons.Count -eq 0; $attempt++) {
        Start-Sleep -Milliseconds 100
        $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    }

    foreach ($position in $PreClickAt) {
        $parts = $position.Split(',')
        if ($parts.Count -ne 2) { throw "PreClickAt must be 'x,y': $position" }
        $x = $rect.Left + [int]$parts[0]
        $y = $rect.Top + [int]$parts[1]
        Set-TargetForeground $handle
        [ConduitWin]::SetCursorPos($x, $y) | Out-Null
        Start-Sleep -Milliseconds 150
        [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
        [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 500
    }

    foreach ($index in $ClickButtonIndex) {
        if ($index -lt 0 -or $index -ge $buttons.Count) { throw "UIA button index out of range: $index" }
        $pattern = $buttons.Item($index).GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $pattern.Invoke()
        Start-Sleep -Milliseconds 700
        $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    }

    foreach ($wanted in $ClickButton) {
        $target = $null
        for ($i = 0; $i -lt $buttons.Count; $i++) {
            if ((Get-ElementName $buttons.Item($i)) -like "*$wanted*") { $target = $buttons.Item($i); break }
        }
        if ($null -eq $target) { throw "UIA button not found: $wanted" }
        $pattern = $target.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $pattern.Invoke()
        Start-Sleep -Milliseconds 700
        $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    }

    foreach ($position in $ClickAt) {
        $parts = $position.Split(',')
        if ($parts.Count -ne 2) { throw "ClickAt must be 'x,y': $position" }
        $x = $rect.Left + [int]$parts[0]
        $y = $rect.Top + [int]$parts[1]
        Set-TargetForeground $handle
        [ConduitWin]::SetCursorPos($x, $y) | Out-Null
        Start-Sleep -Milliseconds 150
        # MOUSEEVENTF_LEFTDOWN / LEFTUP
        [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
        [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 700
    }

    foreach ($scroll in $ScrollAt) {
        $parts = $scroll.Split(',')
        if ($parts.Count -ne 3) { throw "ScrollAt must be 'x,y,notches': $scroll" }
        $x = $rect.Left + [int]$parts[0]
        $y = $rect.Top + [int]$parts[1]
        $notches = [int]$parts[2]
        Set-TargetForeground $handle
        [ConduitWin]::SetCursorPos($x, $y) | Out-Null
        Start-Sleep -Milliseconds 150
        # A positive notch count scrolls the content down; MOUSEEVENTF_WHEEL
        # takes a signed wheel delta (120 per notch).
        for ($i = 0; $i -lt [Math]::Abs($notches); $i++) {
            $delta = if ($notches -gt 0) { -120 } else { 120 }
            [ConduitWin]::mouse_event(0x0800, 0, 0, $delta, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 120
        }
        Start-Sleep -Milliseconds 600
    }

    # Clicks that only make sense after the view has been scrolled (e.g. a
    # stepper below the settings fold). Same coordinate space as ClickAt.
    foreach ($position in $ClickAtAfterScroll) {
        $parts = $position.Split(',')
        if ($parts.Count -ne 2) { throw "ClickAtAfterScroll must be 'x,y': $position" }
        $x = $rect.Left + [int]$parts[0]
        $y = $rect.Top + [int]$parts[1]
        Set-TargetForeground $handle
        [ConduitWin]::SetCursorPos($x, $y) | Out-Null
        Start-Sleep -Milliseconds 200
        [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
        [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 500
    }

    foreach ($drag in $DragAt) {
        $parts = $drag.Split(',')
        if ($parts.Count -ne 4) { throw "DragAt must be 'x1,y1,x2,y2': $drag" }
        $x1 = $rect.Left + [int]$parts[0]
        $y1 = $rect.Top + [int]$parts[1]
        $x2 = $rect.Left + [int]$parts[2]
        $y2 = $rect.Top + [int]$parts[3]
        Set-TargetForeground $handle
        # Drag detection needs real pointer-move messages; SetCursorPos alone
        # does not reliably produce them, so move through mouse_event with
        # MOUSEEVENTF_MOVE|MOUSEEVENTF_ABSOLUTE (normalized screen coords).
        $screenW = [ConduitWin]::GetSystemMetrics(0)
        $screenH = [ConduitWin]::GetSystemMetrics(1)
        if ($screenW -le 0) { $screenW = 1920 }
        if ($screenH -le 0) { $screenH = 1080 }
        $moveTo = {
            param([int]$px, [int]$py)
            $ax = [uint32][int](($px + 0.5) * 65535 / $screenW)
            $ay = [uint32][int](($py + 0.5) * 65535 / $screenH)
            [ConduitWin]::mouse_event(0x8001, $ax, $ay, 0, [UIntPtr]::Zero)
        }
        & $moveTo $x1 $y1
        Start-Sleep -Milliseconds 200
        [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
        # Move in small steps so the widget sees pointer movement, not a jump.
        for ($step = 1; $step -le 12; $step++) {
            $x = [int]($x1 + ($x2 - $x1) * $step / 12)
            $y = [int]($y1 + ($y2 - $y1) * $step / 12)
            & $moveTo $x $y
            Start-Sleep -Milliseconds 30
        }
        [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 500
    }

    if ($PasteClipboard) {
        Set-TargetForeground $handle
        Start-Sleep -Milliseconds 250
        # Ctrl+V through keybd_event: .NET SendKeys is unreliable under pwsh and
        # can throw "operation completed successfully" without sending anything.
        [ConduitWin]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
        [ConduitWin]::keybd_event(0x56, 0, 0, [UIntPtr]::Zero)
        [ConduitWin]::keybd_event(0x56, 0, 2, [UIntPtr]::Zero)
        [ConduitWin]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 500
    }

    if ($TypeText -ne '' -or $SendKeys -ne '') {
        Add-Type -AssemblyName System.Windows.Forms
        Set-TargetForeground $handle
        Start-Sleep -Milliseconds 250
        if ($TypeText -ne '') {
            try { [System.Windows.Forms.SendKeys]::SendWait($TypeText) } catch { }
            Start-Sleep -Milliseconds 300
        }
        if ($SendKeys -ne '') {
            # SendWait occasionally throws a spurious WOULDBLOCK-ish error when
            # the target did not own the foreground yet; retry a few times and
            # report the last error so a failed paste is visible in the log.
            $keyError = ''
            for ($attempt = 0; $attempt -lt 4; $attempt++) {
                try {
                    [System.Windows.Forms.SendKeys]::SendWait($SendKeys)
                    $keyError = ''
                    break
                } catch {
                    $keyError = $_.Exception.Message
                    Set-TargetForeground $handle
                    Start-Sleep -Milliseconds 300
                }
            }
            if ($keyError -ne '') { Write-Output "SENDKEYS_ERROR=$keyError" }
            Start-Sleep -Milliseconds 500
        }
    }
    if ($SendClickAt.Count -gt 0) {
        [ConduitWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
        foreach ($position in $SendClickAt) {
            $parts = $position.Split(',')
            if ($parts.Count -ne 2) { throw "SendClickAt must be 'x,y': $position" }
            $x = $rect.Left + [int]$parts[0]
            $y = $rect.Top + [int]$parts[1]
            [ConduitWin]::SetForegroundWindow($handle) | Out-Null
            [ConduitWin]::SetCursorPos($x, $y) | Out-Null
            Start-Sleep -Milliseconds 150
            [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
            [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 700
        }
        Start-Sleep -Milliseconds 800
    }
    if ($WaitSeconds -gt 0) { Start-Sleep -Seconds $WaitSeconds }

    if ($PostClickButton.Count -gt 0) {
        Set-TargetForeground $handle
        foreach ($wanted in $PostClickButton) {
            $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
            $target = $null
            for ($i = 0; $i -lt $buttons.Count; $i++) {
                if ((Get-ElementName $buttons.Item($i)) -like "*$wanted*") { $target = $buttons.Item($i); break }
            }
            if ($null -eq $target) { Write-Output "POST_BUTTON_NOT_FOUND=$wanted"; continue }
            $pattern = $target.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
            $pattern.Invoke()
            Start-Sleep -Milliseconds 700
        }
        Start-Sleep -Milliseconds 800
    }

    if ($PostClickAt.Count -gt 0) {
        [ConduitWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
        foreach ($position in $PostClickAt) {
            $parts = $position.Split(',')
            if ($parts.Count -ne 2) { throw "PostClickAt must be 'x,y': $position" }
            $x = $rect.Left + [int]$parts[0]
            $y = $rect.Top + [int]$parts[1]
            [ConduitWin]::SetForegroundWindow($handle) | Out-Null
            [ConduitWin]::SetCursorPos($x, $y) | Out-Null
            Start-Sleep -Milliseconds 150
            [ConduitWin]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
            [ConduitWin]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 700
        }
        Start-Sleep -Milliseconds 800
    }

    Start-Sleep -Milliseconds $SettleMs
    [ConduitWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $h = $rect.Bottom - $rect.Top

    Add-Type -AssemblyName System.Drawing
    $bitmap = New-Object System.Drawing.Bitmap($w, $h)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    [ConduitWin]::PrintWindow($handle, $hdc, 2) | Out-Null
    $graphics.ReleaseHdc($hdc)
    $graphics.Dispose()
    $bitmap.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()

    $buttons = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    $edits = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
    $texts = $rootElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCondition)

    $uia = New-Object System.Collections.Generic.List[string]
    for ($i = 0; $i -lt $buttons.Count; $i++) {
        $element = $buttons.Item($i)
        $uia.Add(("BUTTON {0} | {1} | enabled={2}" -f $i, (Get-ElementName $element), $element.Current.IsEnabled))
    }
    for ($i = 0; $i -lt $edits.Count; $i++) {
        $uia.Add(("EDIT {0} | {1}" -f $i, (Get-ElementName $edits.Item($i))))
    }
    $uia.Add("TEXT_COUNT=$($texts.Count)")
    $limit = [Math]::Min($texts.Count, 120)
    for ($i = 0; $i -lt $limit; $i++) {
        $uia.Add(("TEXT {0} | {1}" -f $i, (Get-ElementName $texts.Item($i))))
    }

    $uia | Out-File -FilePath $uiaPath -Encoding utf8
    [ordered]@{
        name      = $Name
        app_args  = $AppArgs
        data_dir  = $DataDir
        window    = "$w x $h"
        buttons   = $buttons.Count
        edits     = $edits.Count
        texts     = $texts.Count
        png       = $png
        uia       = $uiaPath
        timestamp = (Get-Date).ToString('o')
    } | ConvertTo-Json -Depth 4 | Out-File -FilePath $metaPath -Encoding utf8

    Write-Output "CAPTURED name=$Name png=$png size=${w}x${h} buttons=$($buttons.Count) edits=$($edits.Count) texts=$($texts.Count)"
}
finally {
    [Environment]::SetEnvironmentVariable('CONDUIT_DATA_DIR', $previousData, 'Process')
    if ($null -ne $process -and -not $KeepRunning -and -not $process.HasExited) {
        $process.Kill()
        $null = $process.WaitForExit(5000)
    }
}
