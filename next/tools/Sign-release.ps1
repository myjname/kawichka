<#
Builds update-manifest.json for a release and signs it with minisign.
Run on your own computer (the secret key must never be on GitHub):
  ./next/tools/Sign-release.ps1 -Setup .\Kavychka-Next-Setup.exe -Version 0.1.31 -SecretKey $HOME\kavychka-keys\kavychka.key
Then upload update-manifest.json and update-manifest.json.minisig to the release
next to Kavychka-Next-Setup.exe. minisign asks for the key password.
#>
param(
  [Parameter(Mandatory)][string]$Setup,
  [Parameter(Mandatory)][ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+$')][string]$Version,
  [Parameter(Mandatory)][string]$SecretKey,
  [string]$OutputDirectory = '.'
)
$ErrorActionPreference = 'Stop'
if ((Split-Path $Setup -Leaf) -cne 'Kavychka-Next-Setup.exe') { throw 'The installer must be named Kavychka-Next-Setup.exe' }
if (-not (Get-Command minisign -ErrorAction SilentlyContinue)) { throw 'minisign is not installed (winget install jedisct1.minisign)' }
$file = Get-Item -LiteralPath $Setup
$manifest = [ordered]@{
  schema  = 1
  version = $Version
  asset   = 'Kavychka-Next-Setup.exe'
  sha256  = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
  bytes   = $file.Length
}
$path = Join-Path $OutputDirectory 'update-manifest.json'
# No BOM and no trailing newline: the signature covers the exact bytes.
[System.IO.File]::WriteAllText((Resolve-Path $OutputDirectory).Path + '\update-manifest.json', ($manifest | ConvertTo-Json -Compress), (New-Object System.Text.UTF8Encoding($false)))
minisign -S -s $SecretKey -m $path -t "Kavychka Next $Version"
if ($LASTEXITCODE -ne 0) { throw 'Signing failed' }
Write-Host "Done. Upload $path and $path.minisig to the release."
