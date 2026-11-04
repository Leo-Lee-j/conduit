# Save as UTF-8 with BOM for Windows PowerShell 5.1.
<#
.SYNOPSIS
  Exercise notification history using synthetic, isolated data and UI Automation.
.DESCRIPTION
  Builds are separate: run cargo build --offline --locked --bin conduit first.
  Each run creates its own directory beneath target/notification-acceptance.
  Captures wide/narrow and light/dark layouts, then checks navigation, read state,
  normal shutdown/restart, clearing, and preservation of a corrupt history file.
  PNGs still require visual review; this does not test a screen reader or real CLI.
.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/notification-acceptance.ps1
#>
param(
    [ValidateSet('all', 'wide', 'narrow', 'light', 'long', 'empty', 'navigation', 'stale', 'restart', 'corrupt')]
    [string]$Case = 'all'
)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskRun = Join-Path $taskRoot ('target\notification-acceptance\' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$taskHarness = Join-Path $PSScriptRoot 'ui-acceptance.ps1'
$taskShell = Join-Path $PSHOME 'powershell.exe'
$taskOut = Join-Path $taskRun 'out'
$null = New-Item -ItemType Directory -Force -Path $taskOut

function Write-Json($Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
}
function Assert-Condition($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}
function New-Fixture([string]$Fixture) {
    $data = Join-Path $taskRun $Fixture
    $workspace = Join-Path $data 'workspace'
    $null = New-Item -ItemType Directory -Force -Path $workspace
    # Store canonicalizes Windows workspace paths during load; notification
    # targets must use the same identity as notifications published by the app.
    $targetWorkspace = if ($workspace.StartsWith('\\')) { '\\?\UNC\' + $workspace.Substring(2) } else { '\\?\' + $workspace }
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $session = [ordered]@{
        id = 1; provider = 'Codex'; created_at = $now - 100; exit_code = 0
        title = '通知验收会话'; native_id = $null; messages = @()
    }
    $state = [ordered]@{
        version = 2
        workspaces = @([ordered]@{ path = $workspace; sessions = @($session) })
        ui = [ordered]@{ appearance = 'dark'; auto_rename = $false; performance_monitor = $false }
        updates = [ordered]@{ auto_check = $false; auto_download = $false; install_when_idle = $false }
    }
    if ($Fixture -eq 'light') { $state.ui.appearance = 'light' }
    if ($Fixture -eq 'empty') { $state.workspaces = @() }
    if ($Fixture -eq 'navigation') {
        $state.workspaces[0].sessions += [ordered]@{
            id = 2; provider = 'Codex'; created_at = $now - 200; exit_code = 0
            title = '通知目标会话'; native_id = $null; messages = @()
        }
    }
    Write-Json (Join-Path $data 'state.json') $state
    $target = @{ Session = @{ workspace = $targetWorkspace; session = 1; created_at = $session.created_at; native_id = $null } }
    if ($Fixture -eq 'navigation') { $target.Session.session = 2; $target.Session.created_at = $now - 200 }
    if ($Fixture -eq 'stale') { $target.Session.session = 999 }
    $records = @(
        [ordered]@{
            id = 3; source = 'Session'; level = 'Success'; title = '后台会话已完成'
            message = '代码检查已经结束，可以返回会话查看结果。中文、emoji 🙂 和多行文本应完整显示。'
            target = $target; created_at = $now - 60; updated_at = $now - 60; read_at = $null
            lifecycle = 'Recorded'; occurrences = 1; key = $null; seen_events = @(); revision = 1
        },
        [ordered]@{
            id = 2; source = 'Git'; level = 'Error'; title = '推送未完成'
            message = '连接远端失败；请检查网络后在工作区中重试。'
            target = @{ Workspace = $targetWorkspace }; created_at = $now - 120; updated_at = $now - 120; read_at = $null
            lifecycle = 'Recorded'; occurrences = 2; key = $null; seen_events = @(); revision = 1
        },
        [ordered]@{
            id = 1; source = 'Session'; level = 'Warning'; title = '等待确认'
            message = '上次运行中的确认请求。重启后只保留历史，不重放请求。'
            target = $target; created_at = $now - 180; updated_at = $now - 180; read_at = $now - 90
            lifecycle = 'Active'; occurrences = 1; key = 'fixture-approval'; seen_events = @('requested'); revision = 1
        }
    )
    if ($Fixture -eq 'long') {
        $records[0].title = 'LongUnbrokenNotificationTitle' * 8
        $records[0].message = ('abcdef0123456789' * 40) + "`n" + ('长消息换行验收。' * 28)
    }
    if ($Fixture -eq 'empty') { $records = @() }
    Write-Json (Join-Path $data 'notifications.json') @{ schema = 1; next_id = 4; records = $records }
    return $data
}
function Capture([string]$Name, [string]$Data, [string]$Buttons, [int]$Width = 1440, [int]$Height = 900) {
    & $taskShell -NoProfile -ExecutionPolicy Bypass -File $taskHarness -Action capture -Name $Name -DataDir $Data -OutDir $taskOut -Width $Width -Height $Height -ClickButton $Buttons -SettleMs 1200 -CloseNormally
    if ($LASTEXITCODE -ne 0) { throw "Capture failed: $Name (exit $LASTEXITCODE)" }
}
function Read-History([string]$Data) {
    return Get-Content -LiteralPath (Join-Path $Data 'notifications.json') -Encoding UTF8 -Raw | ConvertFrom-Json
}
function Assert-Text([string]$Name, [string]$Text) {
    $snapshot = Get-Content -LiteralPath (Join-Path $taskOut ($Name + '-uia.txt')) -Encoding UTF8 -Raw
    Assert-Condition ($snapshot.Contains($Text)) "Missing UI text '$Text' in $Name"
}
$cases = if ($Case -eq 'all') { @('wide', 'narrow', 'light', 'long', 'empty', 'navigation', 'stale', 'restart', 'corrupt') } else { @($Case) }
foreach ($scenario in $cases) {
    $data = New-Fixture $scenario
    switch ($scenario) {
        { $_ -in 'wide', 'narrow', 'light', 'long' } {
            $width = if ($scenario -in 'narrow', 'long') { 850 } else { 1440 }
            $height = if ($scenario -in 'narrow', 'long') { 500 } else { 900 }
            Capture $scenario $data '通知，2 条未读' $width $height
            Assert-Text $scenario '关闭通知'
            Assert-Text $scenario '全部标为已读'
            $history = Read-History $data
            Assert-Condition (@($history.records | Where-Object { $null -eq $_.read_at }).Count -eq 2) 'Opening history changed unread state.'
            Assert-Condition (@($history.records | Where-Object { $_.lifecycle -eq 'Active' }).Count -eq 0) 'A restored active request was replayed.'
        }
        'empty' {
            Capture $scenario $data '通知，0 条未读'
            Assert-Text $scenario '暂无通知'
        }
        'navigation' {
            Capture $scenario $data '通知，2 条未读;查看'
            Assert-Text $scenario '通知，1 条未读'
            $state = Get-Content -LiteralPath (Join-Path $data 'state.json') -Encoding UTF8 -Raw | ConvertFrom-Json
            Assert-Condition ($state.ui.recent_sessions[0] -eq 2) 'The notification did not navigate to its bound session.'
            $history = Read-History $data
            Assert-Condition ($null -ne ($history.records | Where-Object { $_.id -eq 3 }).read_at) 'Viewing the source did not mark its record read.'
        }
        'stale' {
            Capture $scenario $data '通知，2 条未读;查看'
            Assert-Text $scenario '目标已不存在'
            $state = Get-Content -LiteralPath (Join-Path $data 'state.json') -Encoding UTF8 -Raw | ConvertFrom-Json
            Assert-Condition ($state.workspaces[0].sessions.Count -eq 1) 'Opening a stale target created a session.'
        }
        'restart' {
            Capture 'mark-all' $data '通知，2 条未读;全部标为已读'
            $history = Read-History $data
            Assert-Condition (@($history.records | Where-Object { $null -eq $_.read_at }).Count -eq 0) 'Read state was not saved on normal shutdown.'
            Capture 'restart-unread' $data '通知，0 条未读;未读'
            Assert-Text 'restart-unread' '没有未读通知'
            Capture 'clear-read' $data '通知，0 条未读;清除已读'
            Assert-Text 'clear-read' '暂无通知'
            Assert-Condition (@((Read-History $data).records).Count -eq 0) 'Cleared history was not persisted.'
        }
        'corrupt' {
            $path = Join-Path $data 'notifications.json'
            $invalid = '{invalid-history'
            [IO.File]::WriteAllText($path, $invalid, [Text.UTF8Encoding]::new($false))
            $before = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            Capture $scenario $data '通知，0 条未读'
            Assert-Text $scenario '通知历史损坏'
            Assert-Condition ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $before) 'Corrupt history was overwritten.'
        }
    }
    Write-Output "PASS: $scenario"
}
Write-Output "Artifacts: $taskOut"
