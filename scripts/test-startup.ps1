param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
if (!$NoBuild) { & (Join-Path $PSScriptRoot 'start.ps1') -BuildOnly; if ($LASTEXITCODE -ne 0) { throw 'Build failed' } }
$output = Join-Path $project 'outputs\Kavychka'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$speech = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\WPF\System.Speech.dll'
$exe = Join-Path $output 'FirstRunTests.exe'
& $compiler /nologo ('/out:' + $exe) ('/reference:' + (Join-Path $output 'Kavychka.exe')) /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll ('/reference:' + $speech) (Join-Path $project 'tests\FirstRunTests.cs')
if ($LASTEXITCODE -ne 0) { throw 'Startup probe compilation failed' }
$fixture = Join-Path $project ('work\first-run-' + [Guid]::NewGuid().ToString('N'))
foreach ($stage in @('--expect-setup','--expect-ready')) {
 & $exe --first-run-probe --data-dir $fixture $stage
 if ($LASTEXITCODE -ne 0) { throw ('First-run probe failed; inspect ' + $fixture) }
}
Write-Output ('Startup evidence: ' + $fixture)
