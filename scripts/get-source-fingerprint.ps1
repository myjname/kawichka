param([Parameter(Mandatory = $true)][string]$Project)
$ErrorActionPreference = 'Stop'
$inputs = @()
foreach ($directory in @('src', 'backend', 'assets')) {
    $inputs += Get-ChildItem -LiteralPath (Join-Path $Project $directory) -File -Recurse |
        Where-Object { $_.Extension -ne '.pyc' -and $_.FullName -notmatch '[\\/]__pycache__[\\/]' }
}
foreach ($relative in @('scripts\build.ps1','scripts\start.ps1','scripts\get-source-fingerprint.ps1','README.md')) {
    $inputs += Get-Item -LiteralPath (Join-Path $Project $relative)
}
# Never hash installed runtimes, weights or the user's library.
(@($inputs | Sort-Object FullName | ForEach-Object {
    $stream = [IO.File]::OpenRead($_.FullName)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $_.FullName.Substring($Project.Length) + ':' + [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}) -join "`n")
