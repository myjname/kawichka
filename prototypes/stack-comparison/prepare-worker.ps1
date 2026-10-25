$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$archive=Join-Path $projectRoot 'work/downloads/python.zip'
$expected='4acbed6dd1c744b0376e3b1cf57ce906f9dc9e95e68824584c8099a63025a3c3'
if(!(Test-Path -LiteralPath $archive)){
    New-Item -ItemType Directory -Path (Split-Path -Parent $archive) -Force | Out-Null
    Invoke-WebRequest -Uri 'https://www.python.org/ftp/python/3.12.10/python-3.12.10-embed-amd64.zip' -OutFile $archive
}
if((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant() -ne $expected){throw 'Embedded Python archive hash mismatch'}
$destination=Join-Path $PSScriptRoot 'artifacts/python'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
[IO.File]::WriteAllLines((Join-Path $destination 'python312._pth'),@('python312.zip','.'),[Text.UTF8Encoding]::new($false))
& (Join-Path $destination 'python.exe') -B -c 'import hashlib,json,threading; print("PASS isolated demonstration worker runtime")'
if($LASTEXITCODE -ne 0){throw 'Embedded Python smoke check failed'}
$slintFolder=Join-Path $PSScriptRoot 'artifacts/slint'
if(Test-Path -LiteralPath $slintFolder){Copy-Item -LiteralPath (Join-Path $destination 'vcruntime140.dll') -Destination $slintFolder -Force}
