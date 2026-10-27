param([ValidateSet('debug','release')][string]$Profile='release',[switch]$Offline,[string]$Output,[string]$Version)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $env:CARGO_HOME){$env:CARGO_HOME=Join-Path $projectRoot 'work/rust-next-cache/cargo'}
$env:CARGO_TARGET_DIR=Join-Path $projectRoot $(if($Profile -eq 'release'){'work/rust-next-release-target'}else{'work/rust-next-target'})
$env:CARGO_INCREMENTAL='0'
$manifest=Get-Content (Join-Path $PSScriptRoot 'Cargo.toml') -Raw
if(-not $Version){$Version=[regex]::Match($manifest,'(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value}
if($Version -notmatch '^(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})\.(0|[1-9][0-9]{0,7})$'){throw 'Use major.minor.patch'}
$env:KAVYCHKA_RELEASE_VERSION=$Version
function Get-PackageSourceHash {
    $packageInputs=@(Get-ChildItem -LiteralPath $PSScriptRoot -Recurse -File; Get-ChildItem -LiteralPath (Join-Path $projectRoot 'backend') -Recurse -File; Get-ChildItem -LiteralPath (Join-Path $projectRoot 'assets') -Recurse -File) | Where-Object { $_.Extension -in '.rs','.toml','.slint','.svg','.sql','.py','.json','.txt','.md','.lock','.ps1','.wav' -and $_.FullName -notmatch '[\\/]__pycache__[\\/]' } | Sort-Object FullName
    $lines=@($packageInputs | ForEach-Object { $_.FullName+':'+(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash })
    $hash=[Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes(($lines -join "`n")))).Replace('-','') } finally { $hash.Dispose() }
}
$packageSourceHash=Get-PackageSourceHash
if($Profile -eq 'release'){
    $env:RUSTFLAGS=($env:RUSTFLAGS+' -C target-feature=+crt-static').Trim()
    if($env:CARGO_ENCODED_RUSTFLAGS){$env:CARGO_ENCODED_RUSTFLAGS+=[char]31+'-C'+[char]31+'target-feature=+crt-static'}
}
$cargoArgs=@('build','--manifest-path',(Join-Path $PSScriptRoot 'Cargo.toml'),'-p','kavychka-app','-p','kavychka-maintenance','--locked')
if($Profile -eq 'release'){$cargoArgs+='--release'}
if($Offline){$cargoArgs+='--offline'}
& cargo @cargoArgs
if($LASTEXITCODE -ne 0){throw 'Package build failed'}
if((Get-PackageSourceHash) -ne $packageSourceHash){throw 'Sources changed during compilation; rebuild from a stable source snapshot'}
if(-not $Output){$Output=Join-Path $projectRoot ('outputs/next/'+$version+'/Kavychka-Next-Setup.exe')}
$Output=[IO.Path]::GetFullPath($Output)
if(Test-Path -LiteralPath $Output){throw 'Choose a new output file; existing packages are not overwritten'}
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Output))|Out-Null
$stage=Join-Path $projectRoot ('work/next-package-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage)|Out-Null
$bin=Join-Path $env:CARGO_TARGET_DIR $Profile
Copy-Item -LiteralPath (Join-Path $bin 'kavychka-app.exe') -Destination (Join-Path $stage 'app.exe')
Copy-Item -LiteralPath (Join-Path $bin 'Kavychka.exe') -Destination (Join-Path $stage 'Kavychka.exe')
[IO.Directory]::CreateDirectory((Join-Path $stage 'next'))|Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'worker.py') -Destination (Join-Path $stage 'next/worker.py')
$backend=@('server.py','health_check.py','cosy_worker.py','relocate_storage.py','migrate_storage.py','requirements.txt','requirements-cosy.txt','kavychka_local/__init__.py','kavychka_local/character.py','kavychka_local/cosy.py','kavychka_local/delivery.py','kavychka_local/diversity.py','kavychka_local/downloads.py','kavychka_local/engine.py','kavychka_local/hardware.py','kavychka_local/jobs.py','kavychka_local/llama.py','kavychka_local/locale.py','kavychka_local/messages.json','kavychka_local/profiles.py','kavychka_local/store.py')
foreach($name in $backend){
    $relative='backend/'+$name
    $target=Join-Path $stage $relative
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))|Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot $relative) -Destination $target
}
& (Join-Path $PSScriptRoot 'tools/Bundle-backend.ps1') -Stage $stage
foreach($relative in @('assets/components.json','assets/character.json','assets/voice/profile.json','assets/voice/default-en.wav')){
    $target=Join-Path $stage $relative
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))|Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot $relative) -Destination $target
}
foreach($relative in @('assets/character.json','assets/voice/profile.json')){if(-not (Get-Content (Join-Path $stage $relative) -Raw | ConvertFrom-Json).public_default){throw 'Personal default cannot be packaged'}}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $stage 'README.md')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'THIRD_PARTY_NOTICES.md') -Destination (Join-Path $stage 'THIRD_PARTY_NOTICES.md')
foreach($name in @('Slint-Royalty-free-2.0.md','claxon-Apache-2.0.txt','zip-MIT.txt','byteorder-MIT.txt')){
    [IO.Directory]::CreateDirectory((Join-Path $stage 'licenses'))|Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot ('licenses/'+$name)) -Destination (Join-Path $stage ('licenses/'+$name))
}
& (Join-Path $PSScriptRoot 'Collect-notices.ps1') -Output (Join-Path $stage 'licenses/dependencies') -Offline:$Offline
$packArgs=@('--pack',('"'+(Join-Path $bin 'kavychka-app.exe')+'"'),('"'+$stage+'"'),('"'+$Output+'"'),$version)
$pack=Start-Process -FilePath (Join-Path $bin 'Kavychka.exe') -ArgumentList $packArgs -WorkingDirectory $projectRoot -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput ($stage+'.pack.stdout.log') -RedirectStandardError ($stage+'.pack.stderr.log')
if($pack.ExitCode -ne 0){throw ('Package integrity/allowlist verification failed; see '+$stage+'.pack.stderr.log')}
Get-FileHash -LiteralPath $Output -Algorithm SHA256
Write-Output $Output
