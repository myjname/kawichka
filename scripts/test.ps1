param([string]$Python = 'python', [string]$Ffmpeg = '')
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$pythonCommand = Get-Command $Python -ErrorAction Stop
$pythonPath = $pythonCommand.Source
Push-Location $project
try {
 if (!$Ffmpeg) {
  $Ffmpeg = & $pythonPath -B -c 'import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())'
  if ($LASTEXITCODE -ne 0) { throw 'Install test helpers first: python -m pip install -r tests/requirements.txt' }
  $Ffmpeg = [string]$Ffmpeg
 }
 if (!(Test-Path -LiteralPath $Ffmpeg) -or (Get-FileHash -LiteralPath $Ffmpeg -Algorithm SHA256).Hash.ToLowerInvariant() -ne '2ce797a0f88d7f067180338fb227f7b1928ea727bd9a4d7a1d022f7c52af71a3') { throw 'Use the pinned imageio-ffmpeg 0.6.0 Windows decoder for the native audio tests' }
 & (Join-Path $PSScriptRoot 'build.ps1')
 if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
 & (Join-Path $PSScriptRoot 'test-native.ps1') -NoBuild -Ffmpeg $Ffmpeg
 if ($LASTEXITCODE -ne 0) { throw 'Native checks failed' }
 & (Join-Path $PSScriptRoot 'test-startup.ps1') -NoBuild
 if ($LASTEXITCODE -ne 0) { throw 'Ordinary startup checks failed' }
 & (Join-Path $PSScriptRoot 'test-reminder-restart.ps1') -NoBuild
 if ($LASTEXITCODE -ne 0) { throw 'Ordinary restart checks failed' }
 & (Join-Path $PSScriptRoot 'test-update.ps1') -Python $pythonPath
 if ($LASTEXITCODE -ne 0) { throw 'Installer and rollback checks failed' }
 & $pythonPath -B -m unittest discover -s tests -v
 if ($LASTEXITCODE -ne 0) { throw 'Python checks failed' }
 Write-Output 'PASS complete Windows regression suite (no model downloads)'
} finally { Pop-Location }
