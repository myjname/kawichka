param([ValidateSet('test','build','check','clippy','fmt')][string]$Action='test',[switch]$Offline)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $env:CARGO_HOME){$env:CARGO_HOME=Join-Path $projectRoot 'work/rust-next-cache/cargo'}
$env:CARGO_TARGET_DIR=Join-Path $projectRoot 'work/rust-next-target'
if(-not $env:CARGO_INCREMENTAL){$env:CARGO_INCREMENTAL='0'}
$cargoArgs=@($Action,'--manifest-path',(Join-Path $PSScriptRoot 'Cargo.toml'))
if($Action -eq 'fmt'){$cargoArgs+=@('--all','--','--check')}
else {
    $cargoArgs+=@('--workspace','--locked')
    if($Offline){$cargoArgs+='--offline'}
    if($Action -eq 'clippy'){$cargoArgs+=@('--all-targets','--','-D','warnings')}
}
& cargo @cargoArgs
if($LASTEXITCODE -ne 0){throw "cargo $Action failed ($LASTEXITCODE)"}
