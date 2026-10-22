param([switch]$NoInstaller)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$fingerprint = & (Join-Path $PSScriptRoot 'get-source-fingerprint.ps1') -Project $project
$destination = Join-Path $project 'outputs\Kavychka'
New-Item -ItemType Directory -Force -Path $destination | Out-Null
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$speech = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\WPF\System.Speech.dll'
$workRoot = [IO.Path]::GetFullPath((Join-Path $project 'work'))
$staging = Join-Path $workRoot ('build-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $staging | Out-Null
try {
 $sourceFiles = @(Get-ChildItem -LiteralPath (Join-Path $project 'src') -Filter '*.cs' -File | Where-Object {$_.Name -ne 'Installer.cs'} | ForEach-Object {$_.FullName})
 $stageExe = Join-Path $staging 'Kavychka.exe'
 & $compiler /nologo /target:winexe ('/out:' + $stageExe) /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Net.Http.dll /reference:System.Web.Extensions.dll /reference:System.Security.dll /reference:Microsoft.CSharp.dll /reference:System.IO.Compression.dll /reference:System.IO.Compression.FileSystem.dll ('/reference:' + $speech) $sourceFiles
 if ($LASTEXITCODE -ne 0) { throw 'Application build failed' }
 # Preserve a previous source build's personal assets before replacing defaults.
 # This private folder is excluded from the distributable package.
 if (Test-Path -LiteralPath (Join-Path $destination 'assets')) {
  $capture = Start-Process -FilePath $stageExe -ArgumentList ('--capture-legacy "' + $destination + '" --data-dir "' + (Join-Path $destination 'user-legacy') + '"') -WindowStyle Hidden -Wait -PassThru
  if ($capture.ExitCode -ne 0) { throw 'Could not preserve existing personal assets. Build output was not replaced.' }
 }
 foreach ($file in Get-ChildItem -LiteralPath (Join-Path $project 'backend') -File -Recurse | Where-Object {$_.Extension -in @('.py','.txt','.json') -and $_.FullName -notmatch '[\\/]__pycache__[\\/]'}) {
  $relative = $file.FullName.Substring($project.Length + 1)
  $copy = Join-Path $staging $relative
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $copy) | Out-Null
  Copy-Item -LiteralPath $file.FullName -Destination $copy
 }
 foreach ($relative in @('assets\character.json','assets\components.json','assets\voice\profile.json','assets\voice\default-en.wav','README.md')) {
  $copy = Join-Path $staging $relative
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $copy) | Out-Null
  Copy-Item -LiteralPath (Join-Path $project $relative) -Destination $copy
 }
 $manifestFiles = @(Get-ChildItem -LiteralPath $staging -File -Recurse | Sort-Object FullName | ForEach-Object {
  @{path=$_.FullName.Substring($staging.Length+1).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
 })
 @{format=1;version=[Reflection.AssemblyName]::GetAssemblyName($stageExe).Version.ToString();files=$manifestFiles} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $staging 'package.json') -Encoding UTF8
 foreach ($file in Get-ChildItem -LiteralPath $staging -File -Recurse) {
  $copy = Join-Path $destination $file.FullName.Substring($staging.Length+1)
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $copy) | Out-Null
  Copy-Item -LiteralPath $file.FullName -Destination $copy -Force
 }
$zipPath = Join-Path $project 'outputs\Kavychka-portable.zip'
if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath }
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zipPath -CompressionLevel Optimal
if (!$NoInstaller) {
 $setupSources = @('Installer.cs','InstallTransaction.cs','LegacyPersonalization.cs','SharedStorage.cs','AppText.cs','AssemblyInfo.cs') | ForEach-Object {Join-Path $project ('src\'+$_)}
 & $compiler /nologo /target:winexe ('/out:' + (Join-Path $project 'outputs\Kavychka-Setup.exe')) /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll /reference:System.IO.Compression.dll /reference:System.IO.Compression.FileSystem.dll ('/resource:' + $zipPath + ',payload.zip') $setupSources
 if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
}
@{fingerprint=$fingerprint} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $project 'outputs\launcher-build.json') -Encoding UTF8
Write-Output $destination
} finally {
 $resolvedStage = [IO.Path]::GetFullPath($staging)
 if ($resolvedStage.StartsWith($workRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $resolvedStage) -like 'build-*' -and (Test-Path -LiteralPath $resolvedStage)) {
  Remove-Item -LiteralPath $resolvedStage -Recurse -Force
 }
}
