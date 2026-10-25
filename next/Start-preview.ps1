param([switch]$Offline, [switch]$Verify, [switch]$VerifyAudio, [switch]$BuildOnly, [string]$LegacySource)
$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Utility -ErrorAction Stop
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'work/rust-next-target'
$exePath = Join-Path $env:CARGO_TARGET_DIR 'debug/kavychka-app.exe'
$stampPath = Join-Path $env:CARGO_TARGET_DIR 'kavychka-preview-build.json'
$inputs = Get-ChildItem -LiteralPath $PSScriptRoot -Recurse -File | Where-Object { $_.Extension -in '.rs','.toml','.slint','.svg','.sql','.py','.json' -or $_.Name -eq 'Cargo.lock' } | Sort-Object FullName
$fingerprints = @($inputs | ForEach-Object { $_.FullName.Substring($PSScriptRoot.Length) + ':' + (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash })
[Array]::Sort($fingerprints, [StringComparer]::Ordinal)
$digest = [Security.Cryptography.SHA256]::Create()
try { $sourceHash = [BitConverter]::ToString($digest.ComputeHash([Text.Encoding]::UTF8.GetBytes(($fingerprints -join "`n")))).Replace('-','') }
finally { $digest.Dispose() }
$current = $false
if ((Test-Path -LiteralPath $exePath) -and (Test-Path -LiteralPath $stampPath)) {
    try { $stamp = Get-Content -LiteralPath $stampPath -Raw | ConvertFrom-Json; $current = $stamp.source -eq $sourceHash -and $stamp.binary -eq (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash } catch { $current = $false }
}
if (-not $current) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Install Rust with the MSVC toolchain and Visual Studio C++ Build Tools, then restart the terminal.' }
    if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $projectRoot 'work/rust-next-cache/cargo' }
    if (-not $env:CARGO_INCREMENTAL) { $env:CARGO_INCREMENTAL = '0' }
    $cargoArgs = @('build', '--manifest-path', (Join-Path $PSScriptRoot 'Cargo.toml'), '-p', 'kavychka-app', '--locked')
    if ($Offline) { $cargoArgs += '--offline' }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Preview build failed ($LASTEXITCODE)" }
    @{source=$sourceHash; binary=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash} | ConvertTo-Json | Set-Content -LiteralPath $stampPath -Encoding UTF8
} else { Write-Output 'Using the current local build (source and executable hashes match).' }
if ($BuildOnly) { Write-Output $exePath; return }
if ($VerifyAudio) { $Verify = $true }
$runPath = Join-Path $projectRoot ('work/slint-shell-launch-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runPath | Out-Null
$dataPath = Join-Path $projectRoot 'work/slint-library-user-preview'
if ($Verify) { $dataPath = Join-Path $runPath 'test-data' }
$logPath = Join-Path $runPath 'stderr.log'
$outputPath = Join-Path $runPath 'stdout.log'
# Only the sandbox is writable; the legacy source is resolved below for import.
$appArgs = @('--data-dir', ('"' + $dataPath + '"'))
if (-not $Verify) {
    $explicitLegacy = [bool]$LegacySource
    $selectedPointer = $false
    if (-not $LegacySource) {
        $localPath = [Environment]::GetFolderPath('LocalApplicationData')
        $pointerPath = Join-Path $localPath 'Kavychka-storage.json'
        if (Test-Path -LiteralPath $pointerPath) { $selectedPointer = $true; $LegacySource = (Get-Content -LiteralPath $pointerPath -Raw | ConvertFrom-Json).directory }
        else { $LegacySource = Join-Path $localPath 'Kavychka' }
    }
    if ((Test-Path -LiteralPath $LegacySource) -or $explicitLegacy -or $selectedPointer) { $appArgs += @('--legacy-source', ('"' + $LegacySource + '"')) }
}
$options = @{
    FilePath = $exePath
    ArgumentList = $appArgs
    WorkingDirectory = $projectRoot
    WindowStyle = 'Hidden'
    RedirectStandardError = $logPath
    RedirectStandardOutput = $outputPath
}
if ($Verify) {
    $checks = @(@{Name='runtime'; Mode='--runtime-smoke-test'}, @{Name='live'; Mode='--live-reminder-test'})
    if ($VerifyAudio) { $checks += @{Name='platform'; Mode='--platform-smoke-test'} }
    foreach ($check in $checks) {
        $checkPath = Join-Path $runPath $check.Name
        New-Item -ItemType Directory -Path $checkPath | Out-Null
        $checkOptions = $options.Clone()
        $checkOptions.ArgumentList = @('--data-dir', ('"' + (Join-Path $checkPath 'data') + '"'), $check.Mode, ('"' + (Join-Path $checkPath 'evidence') + '"'))
        $checkOptions.RedirectStandardError = Join-Path $checkPath 'stderr.log'
        $checkOptions.RedirectStandardOutput = Join-Path $checkPath 'stdout.log'
        $process = Start-Process @checkOptions -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Preview verification failed. See $($checkOptions.RedirectStandardError)" }
    }
    Write-Output "Preview verification passed: $runPath"
} else { Start-Process @options | Out-Null }
