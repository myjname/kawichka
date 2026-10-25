param([Parameter(Mandatory=$true)][ValidateSet('slint','avalonia')][string]$Stack)
$ErrorActionPreference='Stop'
$name=if($Stack -eq 'slint'){'Quote.Slint.exe'}else{'Quote.Avalonia.exe'}
$exe=Join-Path $PSScriptRoot "artifacts/$Stack/$name"
if(!(Test-Path -LiteralPath $exe)){throw 'First build the prototypes with build.ps1.'}
$env:SLINT_STYLE='fluent'
$candidates=@((Join-Path $PSScriptRoot 'artifacts/python/python.exe'))
$pythonCommand=Get-Command python -ErrorAction SilentlyContinue
if($pythonCommand -and $pythonCommand.Source -notmatch '[\\/]WindowsApps[\\/]'){$candidates+=$pythonCommand.Source}
$pythonPath=$candidates|Where-Object{Test-Path -LiteralPath $_}|Select-Object -First 1
if(!$pythonPath){throw 'Run prepare-worker.ps1 once to prepare the isolated demonstration worker runtime.'}
$arguments='--common "'+(Join-Path $PSScriptRoot 'common')+'" --python "'+$pythonPath+'"'
Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden | Out-Null
