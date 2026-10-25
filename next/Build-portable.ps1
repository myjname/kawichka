param([Parameter(Mandatory)][string]$Setup,[Parameter(Mandatory)][string]$Output,[string]$Launcher)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$Setup=(Resolve-Path -LiteralPath $Setup).Path
$Output=[IO.Path]::GetFullPath($Output)
if(Test-Path -LiteralPath $Output){throw 'Choose a new ZIP output path'}
if(-not $Launcher){$Launcher=Join-Path $projectRoot 'work/rust-next-release-target/release/Kavychka.exe'}
$Launcher=(Resolve-Path -LiteralPath $Launcher).Path
$stage=Join-Path $projectRoot ('work/portable-'+[guid]::NewGuid().ToString('N'))
$process=Start-Process -FilePath $Launcher -ArgumentList @('--make-portable',('"'+$Setup+'"'),('"'+$stage+'"')) -WindowStyle Hidden -Wait -PassThru
if($process.ExitCode -ne 0){throw 'Portable layout verification failed'}
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Output))|Out-Null
Compress-Archive -LiteralPath (Join-Path $stage 'Portable.exe'),(Join-Path $stage 'setup.exe'),(Join-Path $stage 'portable.owner'),(Join-Path $stage 'README.txt') -DestinationPath $Output -CompressionLevel Optimal
Get-FileHash -LiteralPath $Output -Algorithm SHA256
Write-Output $Output
