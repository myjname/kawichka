param([string]$Python = 'python')
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$workRoot = [IO.Path]::GetFullPath((Join-Path $project 'work'))
$fixture = Join-Path $workRoot ('update-e2e-' + [Guid]::NewGuid().ToString('N'))
$app = Join-Path $fixture 'app'
$data = Join-Path $fixture 'data'
$setup = Join-Path $project 'outputs\Kavychka-Setup.exe'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$passed = $false
function Assert-Update([bool]$Condition,[string]$Message) { if (!$Condition) { throw $Message }; Write-Output ('PASS ' + $Message) }
function Invoke-TestSetup([string]$Executable,[string]$Extra = '') {
 $info = [Diagnostics.ProcessStartInfo]::new($Executable,('--silent --target "' + $app + '" --data-dir "' + $data + '" ' + $Extra))
 $info.UseShellExecute = $false; $info.CreateNoWindow = $true
 $info.EnvironmentVariables['KAVYCHKA_PYTHON'] = $Python
 $process = [Diagnostics.Process]::Start($info)
 try { if (!$process.WaitForExit(90000)) { throw 'Setup test timed out; inspect the running installer before retrying' }; return $process.ExitCode } finally { $process.Dispose() }
}
try {
 if (!(Test-Path -LiteralPath $setup)) { throw 'Build the installer first: scripts/build.ps1' }
 # START builds the portable app only. Refuse a stale embedded payload instead
 # of accidentally testing an older installer after a source change.
 $embedded = [Reflection.Assembly]::LoadFile($setup).GetManifestResourceStream('payload.zip')
 if ($null -eq $embedded) { throw 'Installer payload is missing' }
 $digest = [Security.Cryptography.SHA256]::Create()
 try { $embeddedHash = [BitConverter]::ToString($digest.ComputeHash($embedded)).Replace('-','') } finally { $embedded.Dispose(); $digest.Dispose() }
 if ($embeddedHash -ne (Get-FileHash -LiteralPath (Join-Path $project 'outputs\Kavychka-portable.zip') -Algorithm SHA256).Hash) { throw 'Installer is stale; run scripts/build.ps1 before testing updates' }
 $Python = (Get-Command $Python -ErrorAction Stop).Source
 New-Item -ItemType Directory -Force -Path $app,(Join-Path $data 'models'),(Join-Path $data 'audio') | Out-Null
 '{"VoiceProvider":"Qwen local","History":["retained phrase"],"ExerciseEnabled":true,"Swear":false,"ThemeMode":"dark"}' | Set-Content -LiteralPath (Join-Path $data 'settings.json') -Encoding UTF8
 'model sentinel' | Set-Content -LiteralPath (Join-Path $data 'models\keep.bin')
 'audio sentinel' | Set-Content -LiteralPath (Join-Path $data 'audio\keep.wav')
 $nativeFolder = Join-Path $data 'builtin-days'
 New-Item -ItemType Directory -Path $nativeFolder | Out-Null
 $nativeFile = Join-Path $nativeFolder '2026-10-19.en.json'
 '{"Version":1,"Day":"2026-10-19","Language":"en","Rows":[{"Id":"stock-11111111111111111111111111111111","Slot":"old-break","Category":"break","Text":"A retained basic reminder.","CreatedAt":"2026-10-19T09:00:00Z","UsedAt":"2026-10-19T09:05:00Z","Kind":"planned","Followup":false,"Approved":true,"Active":true,"Rejected":false,"Deleted":false}]}' | Set-Content -LiteralPath $nativeFile -Encoding UTF8
 $nativeHash = (Get-FileHash -LiteralPath $nativeFile -Algorithm SHA256).Hash
 $reminderFile = Join-Path $data 'reminder-state.json'
 $reminderStamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
 @{Version=1;Category='exercise';Text='Retained pending reminder.';MessageUtc=('/Date(' + ($reminderStamp-30000) + ')/');CheckpointUtc=('/Date(' + $reminderStamp + ')/');Pending=$true;Manual=$false;Followup=$true;Cycle=@{Category='exercise';StartedUtc=('/Date(' + ($reminderStamp-60000) + ')/');NextRepeatUtc=('/Date(' + ($reminderStamp+300000) + ')/');RepeatsSent=1;Finished=$false;HiddenByConfirmation=$false}} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $reminderFile -Encoding UTF8
 $reminderHash = (Get-FileHash -LiteralPath $reminderFile -Algorithm SHA256).Hash
 @'
import json,sqlite3,sys
from pathlib import Path
data=Path(sys.argv[1]); db=sqlite3.connect(data/'library.sqlite3')
if sys.argv[2]=='create':
    db.executescript('CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE phrases(id TEXT PRIMARY KEY,text TEXT NOT NULL,normalized TEXT UNIQUE NOT NULL,category TEXT NOT NULL,model TEXT NOT NULL,profile TEXT NOT NULL,cache_key TEXT,audio_path TEXT,approved INTEGER NOT NULL DEFAULT 0,day TEXT,used_at TEXT,created_at TEXT NOT NULL,deleted INTEGER NOT NULL DEFAULT 0);')
    db.execute('INSERT INTO settings VALUES(?,?)',('character_description',json.dumps('Retained custom personality')))
    db.commit()
else:
    assert {'followup','language','rejected'} <= {row[1] for row in db.execute('PRAGMA table_info(phrases)')}
    assert json.loads(db.execute("SELECT value FROM settings WHERE key='character_description'").fetchone()[0])=='Retained custom personality'
    assert db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
db.close()
'@ | Set-Content -LiteralPath (Join-Path $fixture 'database.py') -Encoding UTF8
 & $Python -B (Join-Path $fixture 'database.py') $data create
 if ($LASTEXITCODE -ne 0) { throw 'Could not create legacy database fixture' }
 Assert-Update ((Invoke-TestSetup $setup) -eq 0) 'actual installer passes startup and legacy migration checks'
 $settings = Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw | ConvertFrom-Json
 Assert-Update ($settings.SchemaVersion -eq 2 -and $settings.ThemeMode -eq 'dark' -and $settings.History[0] -eq 'retained phrase' -and $settings.SpeechLanguage -eq 'ru') 'legacy settings, theme, history and Russian reminder language survive installation'
 Assert-Update ((Get-FileHash -LiteralPath $nativeFile -Algorithm SHA256).Hash -eq $nativeHash) 'native daily text and playback history survive installation'
 Assert-Update ((Get-FileHash -LiteralPath $reminderFile -Algorithm SHA256).Hash -eq $reminderHash) 'pending reminder and repeat deadline survive installation unchanged'
 & $Python -B (Join-Path $fixture 'database.py') $data verify
 Assert-Update ($LASTEXITCODE -eq 0) 'actual Python migration preserves custom personality and valid SQLite'
 $report = Join-Path $fixture 'personalization-check.json'
 $info = [Diagnostics.ProcessStartInfo]::new((Join-Path $app 'Kavychka.exe'),('--setup-smoke "' + $report + '" --settings-check --data-dir "' + $data + '"'))
 $info.UseShellExecute = $false; $info.CreateNoWindow = $true; $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden; $info.EnvironmentVariables['KAVYCHKA_PYTHON'] = $Python
 $personalizationCheck = [Diagnostics.Process]::Start($info)
 try { if (!$personalizationCheck.WaitForExit(60000)) { throw 'Personalization check timed out; inspect its process before retrying' } } finally { $personalizationCheck.Dispose() }
 $personalization = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
 Assert-Update ($personalization.personalization_migrated -and $personalization.character_description -eq 'Retained custom personality') 'actual first backend launch imports the existing personality as a user preset'
 $savedPreferences = [IO.File]::ReadAllBytes((Join-Path $data 'settings.json'))
 try {
  $invalidSettings = '{"WorkDays":"invalid array"}'
  [IO.File]::WriteAllText((Join-Path $data 'settings.json'),$invalidSettings,[Text.UTF8Encoding]::new($false))
  $health = Start-Process -FilePath (Join-Path $app 'Kavychka.exe') -ArgumentList ('--health-check "' + (Join-Path $fixture 'invalid-health.json') + '" --data-dir "' + $data + '"') -WindowStyle Hidden -Wait -PassThru
  Assert-Update ($health.ExitCode -ne 0 -and [IO.File]::ReadAllText((Join-Path $data 'settings.json')) -eq $invalidSettings) 'startup verification refuses incompatible settings instead of overwriting them with defaults'
 } finally { [IO.File]::WriteAllBytes((Join-Path $data 'settings.json'),$savedPreferences) }
 $lockedSettings = Join-Path $data 'settings.json'
 $lockedHash = (Get-FileHash -LiteralPath $lockedSettings -Algorithm SHA256).Hash
 $locked = [IO.FileStream]::new($lockedSettings,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
 try {
  Assert-Update ((Invoke-TestSetup $setup) -eq 1) 'installer refuses to commit when migrated preferences cannot be saved'
  Assert-Update (!(Test-Path -LiteralPath (Join-Path $app '.kavychka-install.json'))) 'rollback completes without rewriting unchanged locked preferences'
 } finally { $locked.Dispose() }
 Assert-Update ((Get-FileHash -LiteralPath $lockedSettings -Algorithm SHA256).Hash -eq $lockedHash) 'locked preferences remain byte-for-byte unchanged after rollback'
 # A short-lived fixture process exercises --wait-pid without touching a user's app.
 @'
using System; using System.Threading;
class WaitFixture { static void Main(){bool own; using(var m=new Mutex(true,"Local\\KavychkaBreaks-v1",out own)){if(own){Thread.Sleep(1500);m.ReleaseMutex();}}} }
'@ | Set-Content -LiteralPath (Join-Path $fixture 'wait.cs') -Encoding UTF8
 & $compiler /nologo /target:winexe ('/out:' + (Join-Path $app 'Kavychka.exe')) (Join-Path $fixture 'wait.cs')
 if ($LASTEXITCODE -ne 0) { throw 'Wait fixture build failed' }
 $waiting = Start-Process -FilePath (Join-Path $app 'Kavychka.exe') -WindowStyle Hidden -PassThru
 try { Assert-Update ((Invoke-TestSetup $setup ('--update --wait-pid ' + $waiting.Id)) -eq 0) 'installer waits for the previous process before replacing files' } finally { $waiting.Dispose() }
 $beforeExe = (Get-FileHash -LiteralPath (Join-Path $app 'Kavychka.exe') -Algorithm SHA256).Hash
 $beforeSettings = (Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash
 $beforeDatabase = (Get-FileHash -LiteralPath (Join-Path $data 'library.sqlite3') -Algorithm SHA256).Hash
 @'
using System; using System.IO;
class BadStartup { static int Main(string[] a){int i=Array.IndexOf(a,"--data-dir");if(i>=0){File.WriteAllText(Path.Combine(a[i+1],"settings.json"),"broken preferences");File.WriteAllText(Path.Combine(a[i+1],"library.sqlite3"),"broken database");}return 7;} }
'@ | Set-Content -LiteralPath (Join-Path $fixture 'bad.cs') -Encoding UTF8
 $badExe = Join-Path $fixture 'bad.exe'
 & $compiler /nologo /target:winexe ('/out:' + $badExe) (Join-Path $fixture 'bad.cs')
 if ($LASTEXITCODE -ne 0) { throw 'Bad startup fixture build failed' }
 Add-Type -AssemblyName System.IO.Compression.FileSystem
 $brokenZip = Join-Path $fixture 'broken.zip'
 Copy-Item -LiteralPath (Join-Path $project 'outputs\Kavychka-portable.zip') -Destination $brokenZip
 $zip = [IO.Compression.ZipFile]::Open($brokenZip,[IO.Compression.ZipArchiveMode]::Update)
 try {
  $entry = $zip.GetEntry('package.json'); $reader = [IO.StreamReader]::new($entry.Open())
  try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
  $entry.Delete(); $zip.GetEntry('Kavychka.exe').Delete()
  [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$badExe,'Kavychka.exe') | Out-Null
  $file = $manifest.files | Where-Object {$_.path -eq 'Kavychka.exe'}
  $file.bytes = (Get-Item -LiteralPath $badExe).Length; $file.sha256 = (Get-FileHash -LiteralPath $badExe -Algorithm SHA256).Hash.ToLowerInvariant()
  $writer = [IO.StreamWriter]::new($zip.CreateEntry('package.json').Open(),[Text.UTF8Encoding]::new($false))
  try { $writer.Write(($manifest | ConvertTo-Json -Depth 8)) } finally { $writer.Dispose() }
 } finally { $zip.Dispose() }
 $badSetup = Join-Path $fixture 'bad-setup.exe'
 $sources = @('Installer.cs','InstallTransaction.cs','LegacyPersonalization.cs','SharedStorage.cs','AppText.cs','AssemblyInfo.cs') | ForEach-Object {Join-Path $project ('src\'+$_)}
 & $compiler /nologo /target:winexe ('/out:' + $badSetup) /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll /reference:System.IO.Compression.dll /reference:System.IO.Compression.FileSystem.dll ('/resource:' + $brokenZip + ',payload.zip') $sources
 if ($LASTEXITCODE -ne 0) { throw 'Bad installer fixture build failed' }
 Assert-Update ((Invoke-TestSetup $badSetup) -eq 1) 'actual startup failure makes the installer return failure'
 Assert-Update ((Get-FileHash -LiteralPath (Join-Path $app 'Kavychka.exe') -Algorithm SHA256).Hash -eq $beforeExe) 'actual failed update restores the previous executable'
 Assert-Update ((Get-FileHash -LiteralPath (Join-Path $data 'settings.json') -Algorithm SHA256).Hash -eq $beforeSettings) 'actual failed update restores preferences byte for byte'
 Assert-Update ((Get-FileHash -LiteralPath (Join-Path $data 'library.sqlite3') -Algorithm SHA256).Hash -eq $beforeDatabase) 'actual failed update restores the library byte for byte'
 Assert-Update ((Get-Content -LiteralPath (Join-Path $data 'models\keep.bin') -Raw).Trim() -eq 'model sentinel') 'models remain untouched through installation and rollback'
 Assert-Update ((Get-Content -LiteralPath (Join-Path $data 'audio\keep.wav') -Raw).Trim() -eq 'audio sentinel') 'recordings remain untouched through installation and rollback'
 Assert-Update ((Get-FileHash -LiteralPath $nativeFile -Algorithm SHA256).Hash -eq $nativeHash) 'native daily history remains unchanged through failed update rollback'
 Assert-Update ((Get-FileHash -LiteralPath $reminderFile -Algorithm SHA256).Hash -eq $reminderHash) 'pending reminder state remains unchanged through failed update rollback'
 $passed = $true
} finally {
 if ($passed -and [IO.Path]::GetFullPath($fixture).StartsWith($workRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $fixture) -like 'update-e2e-*') { Remove-Item -LiteralPath $fixture -Recurse -Force }
 elseif (!$passed) { Write-Output ('Test fixtures retained: ' + $fixture) }
}
