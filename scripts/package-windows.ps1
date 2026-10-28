param(
    [switch]$SkipBuild,
    [string]$MakeNsis,
    [string]$RuntimeDir
)

$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $taskRoot
try {
    if (-not $SkipBuild) {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $taskExe = Join-Path $taskRoot 'target\release\conduit.exe'
    if (-not (Test-Path -LiteralPath $taskExe)) { throw "Missing release executable: $taskExe" }
    $taskImage = [IO.File]::ReadAllBytes($taskExe)
    $taskPeOffset = [BitConverter]::ToInt32($taskImage, 0x3c)
    if ([BitConverter]::ToUInt16($taskImage, $taskPeOffset + 4) -ne 0x8664) {
        throw 'This installer requires an x64 executable.'
    }
    $taskVersion = (Get-Content -LiteralPath (Join-Path $taskRoot 'Cargo.toml') |
        Select-String '^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"' |
        Select-Object -First 1).Matches.Groups[1].Value
    if (-not $taskVersion) { throw 'Cannot read package version from Cargo.toml.' }
    if (-not $MakeNsis) {
        $taskCompiler = Get-Command makensis.exe -ErrorAction SilentlyContinue
        if ($taskCompiler) { $MakeNsis = $taskCompiler.Source }
        else { $MakeNsis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe' }
    }
    if (-not (Test-Path -LiteralPath $MakeNsis)) {
        throw 'NSIS 3 is required. Install NSIS and pass -MakeNsis <path to makensis.exe>.'
    }
    if (-not $RuntimeDir) {
        $taskVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (Test-Path -LiteralPath $taskVsWhere) {
            $taskVsRoot = & $taskVsWhere -latest -products '*' -property installationPath
            if ($taskVsRoot) {
                $taskRedist = Join-Path $taskVsRoot 'VC\Redist\MSVC'
                $taskCrt = Get-ChildItem -LiteralPath $taskRedist -Directory -ErrorAction SilentlyContinue |
                    Sort-Object Name -Descending | ForEach-Object {
                        Join-Path $_.FullName 'x64\Microsoft.VC143.CRT'
                    } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
                if ($taskCrt) { $RuntimeDir = $taskCrt }
            }
        }
    }
    if (-not $RuntimeDir) {
        throw 'Cannot locate Microsoft x64 CRT redistributable DLLs. Pass -RuntimeDir <Microsoft.VC143.CRT directory>.'
    }
    $taskDist = Join-Path $taskRoot 'target\dist'
    $null = New-Item -ItemType Directory -Force -Path $taskDist
    $taskOutput = Join-Path $taskDist "Conduit-$taskVersion-windows-x64-setup.exe"
    $taskArguments = @(
        '/V3', '/INPUTCHARSET', 'UTF8',
        "/DAPP_VERSION=$taskVersion",
        "/DAPP_EXE=$taskExe",
        "/DAPP_ICON=$(Join-Path $taskRoot 'assets\app-icons\icon.ico')",
        "/DOUTPUT_FILE=$taskOutput"
    )
    if ($RuntimeDir) {
        $RuntimeDir = (Resolve-Path -LiteralPath $RuntimeDir).Path
        $taskDlls = @(Get-ChildItem -LiteralPath $RuntimeDir -Filter '*.dll' -File)
        if (-not $taskDlls.Count) { throw 'RuntimeDir contains no DLL files.' }
        $taskUninstall = Join-Path $taskDist 'runtime-uninstall.nsh'
        $taskDlls | ForEach-Object { 'Delete "$INSTDIR\' + $_.Name + '"' } |
            Set-Content -LiteralPath $taskUninstall -Encoding utf8
        $taskArguments += "/DRUNTIME_DIR=$RuntimeDir"
        $taskArguments += "/DRUNTIME_UNINSTALL=$taskUninstall"
    }
    & $MakeNsis @taskArguments (Join-Path $PSScriptRoot 'windows\installer.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed.' }
    $taskHash = (Get-FileHash -LiteralPath $taskOutput -Algorithm SHA256).Hash.ToLowerInvariant()
    "$taskHash  $([IO.Path]::GetFileName($taskOutput))" |
        Set-Content -LiteralPath "$taskOutput.sha256" -Encoding ascii
    Write-Output "Installer: $taskOutput"
    Write-Output "SHA256: $taskHash"
} finally {
    Pop-Location
}
