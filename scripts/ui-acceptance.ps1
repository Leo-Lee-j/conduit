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
    [string[]]$ClickAt = @(),
    [string[]]$ScrollAt = @(),
    [int]$Width = 0,
    [int]$Height = 0,
    [int]$SettleMs = 800,
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
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $process.Responding) {
            return $process.MainWindowHandle
        }
        Start-Sleep -Milliseconds 150
    }
    throw 'Conduit did not expose a responding window within 20 seconds.'
}

function Get-ElementName($element) {
    try { return [string]$element.Current.Name } catch { return '' }
}

if ($Action -eq 'seed' -or $Seed) { Invoke-Seed }
if ($Action -eq 'seed') { exit 0 }

# -File argument parsing joins comma lists; allow "x,y;x,y" in one value.
$ClickButton = @($ClickButton | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$ClickAt = @($ClickAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })
$ScrollAt = @($ScrollAt | ForEach-Object { $_ -split ';' } | Where-Object { $_ -ne '' })

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
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, int dwData, UIntPtr dwExtraInfo);
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
    }
    if ($AppArgs -ne '') { $launch.ArgumentList = $AppArgs }
    $process = Start-Process @launch
    $handle = Wait-MainWindow $process
    Start-Sleep -Milliseconds 600
    # Foreground activation can be refused by the shell while the previous
    # instance is still closing; retry before sending any synthetic input.
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        [ConduitWin]::SetForegroundWindow($handle) | Out-Null
        if ([ConduitWin]::GetForegroundWindow() -eq $handle) { break }
        Start-Sleep -Milliseconds 150
    }
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

    $rect = New-Object ConduitWin+RECT
    [ConduitWin]::GetWindowRect($handle, [ref]$rect) | Out-Null

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
        [ConduitWin]::SetForegroundWindow($handle) | Out-Null
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
        [ConduitWin]::SetForegroundWindow($handle) | Out-Null
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
