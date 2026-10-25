param([string]$Source, [switch]$Offline)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Source) {
    $localPath = [Environment]::GetFolderPath('LocalApplicationData')
    $pointerPath = Join-Path $localPath 'Kavychka-storage.json'
    $Source = if (Test-Path -LiteralPath $pointerPath) { (Get-Content -LiteralPath $pointerPath -Raw | ConvertFrom-Json).directory } else { Join-Path $localPath 'Kavychka' }
}
if (-not [IO.Path]::IsPathRooted($Source) -or -not (Test-Path -LiteralPath $Source)) { throw 'Existing absolute legacy storage is required. Close the old application first.' }
$buildArgs = @{BuildOnly=$true}
if ($Offline) { $buildArgs.Offline=$true }
$buildOutput = @(& (Join-Path $PSScriptRoot 'Start-preview.ps1') @buildArgs)
$executable = [string]$buildOutput[-1]
if (-not (Test-Path -LiteralPath $executable)) { throw 'Preview build did not return an executable.' }
$runPath = Join-Path $projectRoot ('work/slint-models-verify-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runPath | Out-Null
$dataPath = Join-Path $runPath 'data'
$checks = @(@{Name='inference';Mode='--models-library-test'},@{Name='ui';Mode='--library-ui-test'})
foreach ($check in $checks) {
    $arguments = @('--data-dir',('"'+$dataPath+'"'),$check.Mode,('"'+(Join-Path $runPath $check.Name)+'"'))
    if ($check.Name -eq 'inference') { $arguments += @('--legacy-source',('"'+$Source+'"')) }
    $process = Start-Process -FilePath $executable -ArgumentList $arguments -WorkingDirectory $projectRoot -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $runPath ($check.Name+'.stdout.log')) -RedirectStandardError (Join-Path $runPath ($check.Name+'.stderr.log'))
    if ($process.ExitCode -ne 0) { throw "Verification failed ($($check.Name)); see $runPath" }
}
Write-Output "Installed-model and native library verification passed: $runPath"
