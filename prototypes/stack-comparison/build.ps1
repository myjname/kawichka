param([switch]$Offline)
$ErrorActionPreference='Stop'
$prototypeRoot=$PSScriptRoot
$projectRoot=[IO.Path]::GetFullPath((Join-Path $prototypeRoot '../..'))
$benchRoot=Join-Path $projectRoot 'work/stack-comparison-20261022'
$env:DOTNET_CLI_HOME=Join-Path $benchRoot 'cache/dotnet'
$env:NUGET_PACKAGES=Join-Path $benchRoot 'cache/nuget'
$env:DOTNET_CLI_TELEMETRY_OPTOUT='1'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE='1'
$env:DOTNET_GENERATE_ASPNET_CERTIFICATE='false'
$env:MSBuildEnableWorkloadResolver='false'
$env:CARGO_HOME=Join-Path $benchRoot 'cache/cargo'
$env:CARGO_TARGET_DIR=Join-Path $benchRoot 'rust-target'
$env:SLINT_STYLE='fluent'
New-Item -ItemType Directory -Path $benchRoot -Force | Out-Null
Push-Location $prototypeRoot
try {
    $rustArgs=@('build','--release','--manifest-path','slint/Cargo.toml','--locked','-j','6')
    if($Offline){$rustArgs+='--offline'}
    & cargo @rustArgs
    if($LASTEXITCODE -ne 0){throw 'Slint build failed'}
    $avalonia=Join-Path $prototypeRoot 'artifacts/avalonia'
    $publish=@('publish','avalonia/Quote.Avalonia.csproj','-c','Release','-r','win-x64','--self-contained','true',
        '-p:RuntimeFrameworkVersion=10.0.13','-p:PublishReadyToRun=true','-p:NuGetAudit=false','-p:DebugType=None','-p:DebugSymbols=false','-o',$avalonia)
    if($Offline){$publish+='--no-restore'}
    & dotnet @publish
    if($LASTEXITCODE -ne 0){throw 'Avalonia publish failed'}
    $slint=Join-Path $prototypeRoot 'artifacts/slint'
    New-Item -ItemType Directory -Path $slint -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $benchRoot 'rust-target/release/quote-slint.exe') -Destination (Join-Path $slint 'Quote.Slint.exe') -Force
    foreach($folder in @($slint,$avalonia)){
        Copy-Item -LiteralPath (Join-Path $prototypeRoot 'common') -Destination $folder -Recurse -Force
    }
    & (Join-Path $PSScriptRoot 'prepare-worker.ps1')
    Write-Output 'PASS both Release prototypes built; Avalonia includes .NET runtime and ReadyToRun.'
}finally{Pop-Location}
