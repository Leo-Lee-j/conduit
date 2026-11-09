param([string]$MakeNsis)

$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $MakeNsis) {
    $taskCompiler = Get-Command makensis.exe -ErrorAction SilentlyContinue
    $MakeNsis = if ($taskCompiler) { $taskCompiler.Source }
        else { Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe' }
}
if (-not (Test-Path -LiteralPath $MakeNsis)) { throw 'NSIS 3 is required; pass -MakeNsis <path>.' }

# Compile the real installer with isolated shell folders and registry keys.
$taskId = [guid]::NewGuid().ToString('N')
$taskTestRoot = Join-Path $taskRoot "target/installer-icons-$taskId"
$taskInstall = Join-Path $taskTestRoot '安装 with spaces'
$taskDesktop = Join-Path $taskTestRoot 'Desktop'
$taskPrograms = Join-Path $taskTestRoot 'Programs'
$taskRegistry = "Software\ConduitInstallerTest-$taskId"
$taskUninstallRegistry = "Software\Microsoft\Windows\CurrentVersion\Uninstall\ConduitInstallerTest-$taskId"
$null = New-Item -ItemType Directory -Path $taskTestRoot, $taskDesktop, $taskPrograms
$taskSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'installer.nsi') -Raw
$taskSource = $taskSource.Replace('"Software\Conduit"', ('"' + $taskRegistry + '"'))
$taskSource = $taskSource.Replace('"Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit"', ('"' + $taskUninstallRegistry + '"'))
$taskSource = $taskSource.Replace('$SMPROGRAMS', $taskPrograms).Replace('$DESKTOP', $taskDesktop)
$taskSource = $taskSource.Replace('$LOCALAPPDATA\Programs\Conduit', $taskInstall)
$taskScript = Join-Path $taskTestRoot 'installer.nsi'
[IO.File]::WriteAllText($taskScript, $taskSource)
$taskIcon = Join-Path $taskRoot 'assets/app-icons/icon.ico'
$taskShell = New-Object -ComObject WScript.Shell

function Assert-Installer($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Invoke-TestInstaller([string]$Path) {
    $taskProcess = Start-Process -FilePath $Path -ArgumentList "/S /D=$taskInstall" -WindowStyle Hidden -PassThru
    if (-not $taskProcess.WaitForExit(30000)) {
        $taskProcess.Kill()
        throw 'Test installer timed out.'
    }
    Assert-Installer ($taskProcess.ExitCode -eq 0) "Installer failed: $($taskProcess.ExitCode)"
}

try {
    $taskPackages = @{}
    foreach ($taskVersion in @('0.1.0', '0.2.0')) {
        # Tiny inert payloads keep this check independent of a Rust build.
        $taskPayload = Join-Path $taskTestRoot "payload-$taskVersion.exe"
        $taskPayloadScript = Join-Path $taskTestRoot "payload-$taskVersion.nsi"
        $taskIconLine = if ($taskVersion -eq '0.2.0') { 'Icon "' + $taskIcon + '"' } else { '' }
        [IO.File]::WriteAllText($taskPayloadScript, @"
Unicode true
Name "Installer icon test"
OutFile "$taskPayload"
RequestExecutionLevel user
$taskIconLine
Section
SectionEnd
"@)
        & $MakeNsis /V2 /INPUTCHARSET UTF8 $taskPayloadScript
        if ($LASTEXITCODE -ne 0) { throw 'Cannot compile test payload.' }
        $taskPackage = Join-Path $taskTestRoot "setup-$taskVersion.exe"
        & $MakeNsis /V2 /INPUTCHARSET UTF8 "/DAPP_VERSION=$taskVersion" "/DAPP_EXE=$taskPayload" "/DUPDATER_EXE=$taskPayload" "/DAPP_ICON=$taskIcon" "/DOUTPUT_FILE=$taskPackage" $taskScript
        if ($LASTEXITCODE -ne 0) { throw 'Cannot compile test installer.' }
        $taskPackages[$taskVersion] = $taskPackage
    }

    Invoke-TestInstaller $taskPackages['0.1.0']
    $taskLinkPath = Join-Path $taskDesktop 'Conduit.lnk'
    Assert-Installer (-not (Test-Path -LiteralPath $taskLinkPath)) 'Fresh install created an unsolicited desktop shortcut.'

    $taskLink = $taskShell.CreateShortcut($taskLinkPath)
    $taskLink.TargetPath = Join-Path $taskInstall 'conduit.exe'
    $taskLink.IconLocation = (Join-Path $taskTestRoot 'payload-0.1.0.exe') + ',0'
    $taskLink.Save()
    Invoke-TestInstaller $taskPackages['0.2.0']

    $taskExpectedExe = Join-Path $taskInstall 'conduit.exe'
    foreach ($taskShortcut in @($taskLinkPath, (Join-Path $taskPrograms 'Conduit/Conduit.lnk'))) {
        $taskLink = $taskShell.CreateShortcut($taskShortcut)
        Assert-Installer ($taskLink.TargetPath -eq $taskExpectedExe) "Stale shortcut target: $taskShortcut"
        Assert-Installer ($taskLink.IconLocation -eq "$taskExpectedExe,0") "Stale shortcut icon: $taskShortcut"
        Assert-Installer ($taskLink.WorkingDirectory -eq $taskInstall) "Incorrect working directory: $taskShortcut"
    }
    Assert-Installer ((Get-Content -LiteralPath (Join-Path $taskInstall 'conduit.install') -Raw) -eq '0.2.0') 'Version marker was not updated.'
    Assert-Installer ((Get-FileHash -LiteralPath $taskExpectedExe).Hash -eq (Get-FileHash -LiteralPath (Join-Path $taskTestRoot 'payload-0.2.0.exe')).Hash) 'Executable/icon resources were not replaced.'

    Remove-Item -LiteralPath $taskLinkPath
    Invoke-TestInstaller $taskPackages['0.2.0']
    Assert-Installer (-not (Test-Path -LiteralPath $taskLinkPath)) 'Upgrade recreated a deleted desktop shortcut.'
    Write-Output 'PASS: silent install/upgrade, desktop and Start menu icons, deleted shortcut, Unicode/spaced path.'
} finally {
    # Test artifacts remain under target for inspection; only test keys are removed.
    foreach ($taskKey in @($taskRegistry, $taskUninstallRegistry)) {
        $taskRegistryKey = "HKCU:\$taskKey"
        if (Test-Path -LiteralPath $taskRegistryKey) { Remove-Item -LiteralPath $taskRegistryKey }
    }
    Write-Output "Test artifacts: $taskTestRoot"
}
