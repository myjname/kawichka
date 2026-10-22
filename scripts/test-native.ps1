param([string]$Ffmpeg = '', [switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$output = Join-Path $project 'outputs\Kavychka'
$workRoot = [IO.Path]::GetFullPath((Join-Path $project 'work'))
if (!$NoBuild) { & (Join-Path $PSScriptRoot 'start.ps1') -BuildOnly; if ($LASTEXITCODE -ne 0) { throw 'Build failed' } }
$references = @('/reference:System.Windows.Forms.dll','/reference:System.Drawing.dll','/reference:System.Net.Http.dll','/reference:System.Web.Extensions.dll','/reference:System.IO.Compression.dll','/reference:System.IO.Compression.FileSystem.dll')
foreach ($name in @('RitualCoreTests','StorageTests')) {
 $core = if ($name -eq 'RitualCoreTests') { 'RitualCore.cs' } else { 'SharedStorage.cs' }
 $exe = Join-Path $workRoot ($name + '.exe')
 & $compiler /nologo ('/out:' + $exe) $references (Join-Path $project 'src\AppText.cs') (Join-Path $project ('src\' + $core)) (Join-Path $project ('tests\' + $name + '.cs'))
 if ($LASTEXITCODE -ne 0) { throw ($name + ' compilation failed') }
 if ($name -eq 'StorageTests') {
  $fixture = Join-Path $workRoot ('storage-tests-' + [Guid]::NewGuid().ToString('N'))
  & $exe $fixture
  $storageExit = $LASTEXITCODE
  if ($storageExit -eq 0 -and [IO.Path]::GetFullPath($fixture).StartsWith($workRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $fixture) -like 'storage-tests-*') { Remove-Item -LiteralPath $fixture -Recurse -Force }
  if ($storageExit -ne 0) { throw ('Storage fixtures retained: ' + $fixture) }
 } else { & $exe }
 if ($LASTEXITCODE -ne 0) { throw ($name + ' failed') }
}
foreach ($name in @('InstallTransactionTests','V2UiTests','LocalizationUiTests','ReminderUiTests','ReminderStateTests','ReminderStateUiTests','NativePresetTests','StartupTests','SettingsFileTests','BuiltinDaysTests','DayLibraryUiTests','ModelPreparationUiTests','PetRenderingTests')) {
 if ($name -eq 'NativePresetTests' -and !$Ffmpeg) { Write-Output 'SKIP NativePresetTests: pass -Ffmpeg with the imageio-ffmpeg 0.6.0 Windows executable'; continue }
 $exe = Join-Path $output ($name + '.exe')
 & $compiler /nologo ('/out:' + $exe) ('/reference:' + (Join-Path $output 'Kavychka.exe')) $references (Join-Path $project ('tests\' + $name + '.cs'))
 if ($LASTEXITCODE -ne 0) { throw ($name + ' compilation failed') }
 if ($name -eq 'NativePresetTests') { & $exe $Ffmpeg }
 elseif ($name -eq 'PetRenderingTests') {
  $fixture = Join-Path $workRoot ('pet-rendering-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $fixture | Out-Null
  $gif = Join-Path $fixture 'source.gif'
  [IO.File]::WriteAllBytes($gif,[Convert]::FromBase64String('R0lGODlhEAAQAIEAAP8AAAAAAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAFAAAACwAAAAAEAAQAAAIHQABCBxIsKDBgwgTKlzIsKHDhxAjSpxIsaLFgQEBACH5BAEUAAEALAAAAAAQABAAgQAA/wAAAAAAAAAAAAgdAAEIHEiwoMGDCBMqXMiwocOHECNKnEixosWBAQEAOw=='))
  & $exe $fixture $gif
  $testExit = $LASTEXITCODE
  if ($testExit -eq 0 -and [IO.Path]::GetFullPath($fixture).StartsWith($workRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $fixture) -like 'pet-rendering-*') { Remove-Item -LiteralPath $fixture -Recurse -Force }
  if ($testExit -ne 0) { throw ('Pet rendering fixtures retained: ' + $fixture) }
 }
 elseif ($name -in @('LocalizationUiTests','ReminderUiTests','ReminderStateUiTests','DayLibraryUiTests','ModelPreparationUiTests')) {
  $fixture = Join-Path $workRoot ('reminder-ui-' + [Guid]::NewGuid().ToString('N'))
  & $exe --self-test --data-dir $fixture
  $testExit = $LASTEXITCODE
  if ($testExit -eq 0 -and [IO.Path]::GetFullPath($fixture).StartsWith($workRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $fixture) -like 'reminder-ui-*') { Remove-Item -LiteralPath $fixture -Recurse -Force }
  if ($testExit -ne 0) { throw ('Reminder UI fixtures retained: ' + $fixture) }
 } else { & $exe }
 if ($LASTEXITCODE -ne 0) { throw ($name + ' failed') }
}
