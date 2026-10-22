param(
    [ValidateSet('chat', 'settings')]
    [string]$View = 'chat'
)

$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskExecutable = Join-Path $taskRoot 'target\debug\conduit.exe'
if (-not (Test-Path -LiteralPath $taskExecutable)) {
    throw 'Run cargo build --locked first.'
}
$taskData = Join-Path $taskRoot ('target\smoke-' + $View + '-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $taskData
$previousData = [Environment]::GetEnvironmentVariable('CONDUIT_DATA_DIR', 'Process')
$taskApp = $null
try {
    $env:CONDUIT_DATA_DIR = $taskData
    $launchOptions = @{
        FilePath = $taskExecutable
        WorkingDirectory = $taskRoot
        # The GUI under test must be visible to expose its main window.
        WindowStyle = 'Normal'
        RedirectStandardOutput = (Join-Path $taskData 'stdout.log')
        RedirectStandardError = (Join-Path $taskData 'stderr.log')
        PassThru = $true
    }
    if ($View -eq 'settings') { $launchOptions.ArgumentList = '--settings' }
    $taskApp = Start-Process @launchOptions
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        $taskApp.Refresh()
        if ($taskApp.HasExited) {
            $diagnostic = Get-Content -LiteralPath (Join-Path $taskData 'stderr.log') -Raw
            throw "Conduit exited during startup ($($taskApp.ExitCode)): $diagnostic"
        }
        if ($taskApp.MainWindowHandle -ne 0 -and $taskApp.MainWindowTitle -eq 'Conduit' -and $taskApp.Responding) {
            $ready = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready) { throw 'Conduit did not expose a responding window within 15 seconds.' }
    # Catch failures just after window creation, including the first render.
    Start-Sleep -Milliseconds 750
    $taskApp.Refresh()
    if ($taskApp.HasExited -or -not $taskApp.Responding) { throw 'Conduit failed after creating its window.' }
    Write-Output "PASS: $View window started and responded. Logs: $taskData"
} finally {
    if ($taskApp -and -not $taskApp.HasExited) {
        # This isolated instance has no workspaces or model tasks. This is a
        # startup smoke test, not a test of normal window-close behavior.
        $taskApp.Kill()
        $null = $taskApp.WaitForExit(5000)
    }
    [Environment]::SetEnvironmentVariable('CONDUIT_DATA_DIR', $previousData, 'Process')
}
