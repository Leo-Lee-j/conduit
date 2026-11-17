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
$taskOriginalCargoVersion = $env:CARGO_PKG_VERSION
$taskOriginalDataDir = $env:CONDUIT_DATA_DIR

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
    foreach ($taskLegalFile in @('LICENSE', 'NOTICE')) {
        $taskInstalledLegalFile = Join-Path $taskInstall $taskLegalFile
        Assert-Installer (Test-Path -LiteralPath $taskInstalledLegalFile) "Missing installed notice: $taskLegalFile"
        Assert-Installer ((Get-FileHash -LiteralPath $taskInstalledLegalFile).Hash -eq (Get-FileHash -LiteralPath (Join-Path $taskRoot $taskLegalFile)).Hash) "Installed notice differs from source: $taskLegalFile"
    }
}

try {
    $taskPackages = @{}
    foreach ($taskVersion in @('0.1.0', '0.2.0', '0.3.0')) {
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
        $taskLauncher = Join-Path $taskTestRoot "launcher-$taskVersion.exe"
        $env:CARGO_PKG_VERSION = $taskVersion
        & rustc --edition 2024 -C debug-assertions=no (Join-Path $taskRoot 'src/bin/conduit-launcher.rs') -o $taskLauncher
        if ($LASTEXITCODE -ne 0) { throw 'Cannot compile test launcher.' }
        # Exercise migration from the old layout as well as version-to-version upgrades.
        $taskVersionScript = $taskScript
        if ($taskVersion -eq '0.1.0') {
            $taskVersionScript = Join-Path $taskTestRoot 'legacy-installer.nsi'
            $taskLegacySource = $taskSource.Replace('File /oname=${APP_FILENAME} "${APP_EXE}"', 'File /oname=conduit.exe "${APP_EXE}"')
            $taskLegacySource = $taskLegacySource.Replace('File /oname=conduit.exe "${LAUNCHER_EXE}"', '')
            [IO.File]::WriteAllText($taskVersionScript, $taskLegacySource)
        }
        $taskPackage = Join-Path $taskTestRoot "setup-$taskVersion.exe"
        & $MakeNsis /V2 /INPUTCHARSET UTF8 "/DAPP_VERSION=$taskVersion" "/DAPP_EXE=$taskPayload" "/DUPDATER_EXE=$taskPayload" "/DLAUNCHER_EXE=$taskLauncher" "/DAPP_ICON=$taskIcon" "/DAPP_LICENSE=$(Join-Path $taskRoot 'LICENSE')" "/DAPP_NOTICE=$(Join-Path $taskRoot 'NOTICE')" "/DOUTPUT_FILE=$taskPackage" $taskVersionScript
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
    $taskVersionedExe = Join-Path $taskInstall 'conduit-0.2.0.exe'
    Assert-Installer ((Get-FileHash -LiteralPath $taskVersionedExe).Hash -eq (Get-FileHash -LiteralPath (Join-Path $taskTestRoot 'payload-0.2.0.exe')).Hash) 'Versioned executable/icon resources were not installed.'
    Assert-Installer ((Get-FileHash -LiteralPath $taskExpectedExe).Hash -eq (Get-FileHash -LiteralPath (Join-Path $taskTestRoot 'launcher-0.2.0.exe')).Hash) 'Legacy executable was not replaced by the launcher.'

    Remove-Item -LiteralPath $taskLinkPath
    Invoke-TestInstaller $taskPackages['0.2.0']
    Assert-Installer (-not (Test-Path -LiteralPath $taskLinkPath)) 'Upgrade recreated a deleted desktop shortcut.'
    Invoke-TestInstaller $taskPackages['0.3.0']
    Assert-Installer (-not (Test-Path -LiteralPath $taskVersionedExe)) 'Upgrade left the obsolete versioned executable behind.'
    Assert-Installer (Test-Path -LiteralPath (Join-Path $taskInstall 'conduit-updater.exe')) 'Upgrade removed the update helper.'

    # Run the real launcher against a small probe, without loading user data or GUI.
    $taskProbeSource = Join-Path $taskTestRoot 'probe.rs'
    [IO.File]::WriteAllText($taskProbeSource, @'
fn main() {
    let data = std::env::var_os("CONDUIT_DATA_DIR").unwrap();
    let report = format!("{}\n{}\n{}\n{}", std::process::id(),
        std::env::current_exe().unwrap().display(),
        std::env::current_dir().unwrap().display(),
        std::env::args().skip(1).collect::<Vec<_>>().join("\n"));
    std::fs::write(std::path::Path::new(&data).join("launch.txt"), report).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(30));
}
'@)
    $taskVersionedExe = Join-Path $taskInstall 'conduit-0.3.0.exe'
    $taskProbeExe = Join-Path $taskTestRoot 'probe.exe'
    & rustc --edition 2024 $taskProbeSource -o $taskProbeExe
    if ($LASTEXITCODE -ne 0) { throw 'Cannot compile launch probe.' }
    Copy-Item -LiteralPath $taskProbeExe -Destination $taskVersionedExe
    $env:CONDUIT_DATA_DIR = $taskTestRoot
    $taskLauncherProcess = $null
    $taskProbeProcess = $null
    try {
        $taskLauncherProcess = Start-Process -FilePath $taskExpectedExe -ArgumentList '--updated "参数 with spaces"' -WorkingDirectory $taskTestRoot -WindowStyle Hidden -PassThru
        Assert-Installer ($taskLauncherProcess.WaitForExit(10000)) 'Stable launcher stayed running.'
        Assert-Installer ($taskLauncherProcess.ExitCode -eq 0) 'Stable launcher failed.'
        $taskReport = Join-Path $taskTestRoot 'launch.txt'
        $taskDeadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath $taskReport) -and [DateTime]::UtcNow -lt $taskDeadline) { Start-Sleep -Milliseconds 50 }
        $taskLines = @(Get-Content -LiteralPath $taskReport -Encoding utf8)
        $taskProbeProcess = Get-Process -Id ([int]$taskLines[0])
        Assert-Installer ($taskProbeProcess.ProcessName -eq 'conduit-0.3.0') 'Running process does not include the installed version.'
        Assert-Installer ($taskLines[1] -eq $taskVersionedExe) 'Launcher started the wrong executable.'
        Assert-Installer ($taskLines[2] -eq $taskTestRoot) 'Launcher changed the working directory.'
        Assert-Installer (($taskLines[3..4] -join '|') -eq '--updated|参数 with spaces') 'Launcher changed update flags or quoted Unicode arguments.'
    } finally {
        foreach ($taskChild in @($taskLauncherProcess, $taskProbeProcess)) {
            if ($taskChild -and -not $taskChild.HasExited) { $taskChild.Kill(); $taskChild.WaitForExit() }
        }
    }

    # Run a copy so the test can wait for uninstall completion without self-deletion.
    $taskUninstaller = Join-Path $taskTestRoot 'uninstall-test.exe'
    Copy-Item -LiteralPath (Join-Path $taskInstall 'uninstall.exe') -Destination $taskUninstaller
    $taskProcess = Start-Process -FilePath $taskUninstaller -ArgumentList "/S _?=$taskInstall" -WindowStyle Hidden -PassThru
    Assert-Installer ($taskProcess.WaitForExit(30000)) 'Test uninstaller timed out.'
    Assert-Installer ($taskProcess.ExitCode -eq 0) 'Test uninstaller failed.'
    Assert-Installer (-not (Test-Path -LiteralPath $taskInstall)) 'Uninstall left application files behind.'
    Write-Output 'PASS: legacy migration, versioned upgrade/process, launcher exit/arguments/environment, rollback-compatible shortcuts, uninstall, notices, Unicode/spaced path.'
} finally {
    $env:CARGO_PKG_VERSION = $taskOriginalCargoVersion
    $env:CONDUIT_DATA_DIR = $taskOriginalDataDir
    # Test artifacts remain under target for inspection; only test keys are removed.
    foreach ($taskKey in @($taskRegistry, $taskUninstallRegistry)) {
        $taskRegistryKey = "HKCU:\$taskKey"
        if (Test-Path -LiteralPath $taskRegistryKey) { Remove-Item -LiteralPath $taskRegistryKey }
    }
    Write-Output "Test artifacts: $taskTestRoot"
}
