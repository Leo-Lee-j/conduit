# Shared local, CI, and release verification for the core test suite.
$ErrorActionPreference = 'Stop'

function Invoke-Check([string]$Program, [string[]]$Arguments) {
    Write-Host "> $Program $($Arguments -join ' ')"
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE."
    }
}

Push-Location (Join-Path $PSScriptRoot '..')
try {
    Invoke-Check python @('scripts/check-i18n.py')
    Invoke-Check cargo @('fmt', '--all', '--check')
    Invoke-Check cargo @('clippy', '--locked', '--all-targets', '--features', 'ui-tests', '--', '-D', 'warnings')
    # ui-tests includes the ordinary tests, so run the combined suite once.
    Invoke-Check cargo @('test', '--locked', '--features', 'ui-tests', '--', '--test-threads=1')
    # Also compile the shipping binaries without test-only features.
    Invoke-Check cargo @('build', '--locked', '--bin', 'conduit', '--bin', 'conduit-updater')
}
finally {
    Pop-Location
}
