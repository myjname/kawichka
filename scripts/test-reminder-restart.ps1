param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
if (!$NoBuild) { & (Join-Path $PSScriptRoot 'start.ps1') -BuildOnly; if ($LASTEXITCODE -ne 0) { throw 'Build failed' } }
$output = Join-Path $project 'outputs\Kavychka'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$speech = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\WPF\System.Speech.dll'
$exe = Join-Path $output 'ReminderRestartTests.exe'
& $compiler /nologo ('/out:' + $exe) ('/reference:' + (Join-Path $output 'Kavychka.exe')) /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll ('/reference:' + $speech) (Join-Path $project 'tests\ReminderRestartTests.cs')
if ($LASTEXITCODE -ne 0) { throw 'Restart probe compilation failed' }
$fixture = Join-Path $project ('work\reminder-restart-' + [Guid]::NewGuid().ToString('N'))
foreach ($stage in @('create','restore','hidden','cleared')) {
 & $exe --restart-probe --data-dir $fixture --stage $stage
 if ($LASTEXITCODE -ne 0) { throw ('Restart probe failed; inspect ' + $fixture) }
}
Write-Output ('Restart evidence: ' + $fixture)
