# Validate the versioned, committed notes; optionally copy them for signing/publication.
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Tag,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
if ($Tag -cnotmatch '^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
    throw 'A stable vX.Y.Z tag is required for release notes.'
}

$taskRepoRoot = Split-Path -Parent $PSScriptRoot
$taskNotesPath = Join-Path $taskRepoRoot "release-notes/$Tag.md"
if (-not (Test-Path -LiteralPath $taskNotesPath -PathType Leaf)) {
    throw "Missing release-notes/$Tag.md. Follow docs/release-notes-prompt.md and include the file in the release commit before tagging."
}

$taskUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
$taskNotes = [System.IO.File]::ReadAllText($taskNotesPath, $taskUtf8)
if ([string]::IsNullOrWhiteSpace($taskNotes)) {
    throw "Release notes are empty: $taskNotesPath"
}
if ($taskUtf8.GetByteCount($taskNotes) -gt 256 * 1024) {
    throw 'Release notes exceed the update manifest limit of 256 KiB.'
}
if (($taskNotes -split '\r?\n', 2)[0] -cne "# Conduit $Tag") {
    throw "Release notes must start with '# Conduit $Tag'."
}
if ($taskNotes -match '\{\{[^}]*\}\}|<!--') {
    throw 'Replace template placeholders and remove comments before releasing.'
}

$taskHighlights = [regex]::Match($taskNotes, '(?m)^## 更新亮点\r?$\n(?<body>[\s\S]*?)(?=^## |\z)')
$taskHighlightCount = [regex]::Matches($taskHighlights.Groups['body'].Value, '(?m)^- \S').Count
if (-not $taskHighlights.Success -or $taskHighlightCount -lt 1 -or $taskHighlightCount -gt 5) {
    throw 'Release notes must contain a 更新亮点 section with 1 to 5 bullet points.'
}

if ($OutputPath) {
    $taskOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
    $null = [System.IO.Directory]::CreateDirectory((Split-Path -Parent $taskOutputPath))
    [System.IO.File]::WriteAllText($taskOutputPath, $taskNotes, $taskUtf8)
    Write-Host "Prepared release-notes/$Tag.md at $taskOutputPath"
} else {
    Write-Host "Validated release-notes/$Tag.md"
}
