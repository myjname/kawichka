param([string]$Output)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'tools/Notice-functions.ps1')
if(-not $Output){$Output=Join-Path (Split-Path -Parent $PSScriptRoot) ('work/notices-tests-'+[guid]::NewGuid().ToString('N'))}
$Output=[IO.Path]::GetFullPath($Output)
if(Test-Path -LiteralPath $Output){throw 'Use a new test evidence directory'}
[IO.Directory]::CreateDirectory($Output)|Out-Null
$checks=[Collections.Generic.List[string]]::new()
function Assert-NoticeTest([bool]$Condition,[string]$Name){if(-not $Condition){throw ('FAILED '+$Name)};$checks.Add($Name)}
function Reject-NoticeTest([string]$Name,[scriptblock]$Action){$rejected=$false;try{& $Action|Out-Null}catch{$rejected=$true};Assert-NoticeTest $rejected $Name}
function Write-NoticeFixture([string]$Path,[string]$Text){[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))|Out-Null;[IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false))}
function New-NoticePackage([string]$Name,[string]$Folder){[IO.Directory]::CreateDirectory($Folder)|Out-Null;Write-NoticeFixture (Join-Path $Folder 'Cargo.toml') '[package]';[PSCustomObject]@{name=$Name;version='1.2.3';source='registry+https://github.com/rust-lang/crates.io-index';manifest_path=(Join-Path $Folder 'Cargo.toml');license='MIT';license_file=$null;repository='https://github.com/example/fixture'}}
$crate=New-NoticePackage 'fixture' (Join-Path $Output 'crate')
$optional=New-NoticePackage 'optional' (Join-Path $Output 'optional')
$roots=@([PSCustomObject]@{name='kavychka-app';version='0.1.0';source=$null},[PSCustomObject]@{name='kavychka-maintenance';version='0.1.0';source=$null})
$metadata=[PSCustomObject]@{packages=@($roots)+@($crate,$optional)}
$tree=@('kavychka-app v0.1.0 (fixture)','fixture v1.2.3','fixture v1.2.3 (*)','','kavychka-maintenance v0.1.0 (fixture)')
$active=@(Get-ActiveNoticePackages $metadata $tree)
Assert-NoticeTest ($active.Count -eq 1 -and $active[0].name -eq 'fixture') 'optional metadata excluded and tree deduplicated'
Reject-NoticeTest 'missing root rejected' {Get-ActiveNoticePackages $metadata @('fixture v1.2.3')}
Reject-NoticeTest 'unknown Cargo output rejected' {Get-ActiveNoticePackages $metadata ($tree+@('unrecognized graph output'))}
$ambiguous=[PSCustomObject]@{packages=@($metadata.packages)+@($crate)}
Reject-NoticeTest 'ambiguous package rejected' {Get-ActiveNoticePackages $ambiguous $tree}
$checksum='a'*64
$lock="[[package]]`nname = `"fixture`"`nversion = `"1.2.3`"`nsource = `"registry+https://github.com/rust-lang/crates.io-index`"`nchecksum = `"$checksum`"`n"
$pins=Get-NoticeLockChecksums $lock
Assert-NoticeTest ($pins['fixture@1.2.3'] -eq $checksum) 'Cargo checksum extracted'
Reject-NoticeTest 'duplicate lock pin rejected' {Get-NoticeLockChecksums ($lock+$lock)}
Reject-NoticeTest 'missing lock checksum rejected' {Get-NoticeLockChecksums ($lock.Replace($checksum,'invalid'))}
$base=Split-Path -Parent $crate.manifest_path
Write-NoticeFixture (Join-Path $base 'NOTICE.txt') 'Attribution fixture only'
$none=[PSCustomObject]@{packages=@()}
$inputs=@(Get-NoticeInputs $crate $Output $none @{})
Assert-NoticeTest ($inputs.Count -eq 1 -and -not $inputs[0].is_license) 'NOTICE alone not counted as license text'
Write-NoticeFixture (Join-Path $base 'LICENSES/MIT.txt') 'License fixture'
$inputs=@(Get-NoticeInputs $crate $Output $none @{})
Assert-NoticeTest (@($inputs|Where-Object is_license).Count -eq 1) 'REUSE LICENSES text recognized'
Write-NoticeFixture (Join-Path $base 'legal.txt') 'Declared license fixture'
$crate.license_file='legal.txt'
$inputs=@(Get-NoticeInputs $crate $Output $none @{})
Assert-NoticeTest (@($inputs|Where-Object {$_.relative -eq 'legal.txt' -and $_.is_license}).Count -eq 1) 'explicit license file recognized'
$crate.license_file='../outside.txt'
Reject-NoticeTest 'declared license traversal rejected' {Get-NoticeInputs $crate $Output $none @{}}
$crate.license_file=$null
Reject-NoticeTest 'absolute source rejected' {Get-VerifiedNoticeFile $base 'C:/outside.txt'}
Reject-NoticeTest 'wrong file hash rejected' {Get-VerifiedNoticeFile $base 'NOTICE.txt' ('0'*64)}
Write-NoticeFixture (Join-Path $base 'EMPTY.txt') ''
Reject-NoticeTest 'empty file rejected' {Get-VerifiedNoticeFile $base 'EMPTY.txt'}
[IO.File]::WriteAllBytes((Join-Path $base 'LARGE.txt'),[byte[]]::new(1000001))
Reject-NoticeTest 'oversize file rejected' {Get-VerifiedNoticeFile $base 'LARGE.txt'}
$suppPackage=New-NoticePackage 'supp-fixture' (Join-Path $Output 'supp-crate')
$suppBase=Split-Path -Parent $suppPackage.manifest_path
$revision='b'*40
Write-NoticeFixture (Join-Path $suppBase '.cargo_vcs_info.json') (@{git=@{sha1=$revision;dirty=$false}}|ConvertTo-Json)
$relative='supp-fixture-1.2.3/LICENSE-MIT.txt'
Write-NoticeFixture (Join-Path $Output $relative) 'Pinned upstream test fixture'
$hash=(Get-FileHash -LiteralPath (Join-Path $Output $relative) -Algorithm SHA256).Hash.ToLowerInvariant()
$extra=[PSCustomObject]@{name='supp-fixture';version='1.2.3';license='MIT';repository=$suppPackage.repository;crate_checksum=$checksum;vcs_revision=$revision;files=@([PSCustomObject]@{path=$relative;sha256=$hash;git_blob_sha1=('c'*40);upstream_path='LICENSE-MIT';url=('https://raw.githubusercontent.com/example/fixture/'+$revision+'/LICENSE-MIT')})}
$supp=[PSCustomObject]@{packages=@($extra)}
$suppPins=@{'supp-fixture@1.2.3'=$checksum}
$inputs=@(Get-NoticeInputs $suppPackage $Output $supp $suppPins)
Assert-NoticeTest ($inputs.Count -eq 1 -and $inputs[0].is_license -and $inputs[0].origin.kind -eq 'pinned_upstream') 'pinned upstream supplement accepted'
Reject-NoticeTest 'different locked crate rejected' {Get-NoticeInputs $suppPackage $Output $supp @{'supp-fixture@1.2.3'=('d'*64)}}
$extra.files[0].sha256='0'*64
Reject-NoticeTest 'corrupt upstream supplement rejected' {Get-NoticeInputs $suppPackage $Output $supp $suppPins}
$extra.files[0].sha256=$hash
$extra.vcs_revision='d'*40
Reject-NoticeTest 'different upstream revision rejected' {Get-NoticeInputs $suppPackage $Output $supp $suppPins}
$extra.vcs_revision=$revision
$extra.files[0].path='../outside.txt'
Reject-NoticeTest 'supplement traversal rejected' {Get-NoticeInputs $suppPackage $Output $supp $suppPins}
$extra.files[0].path=$relative
$linked=Join-Path $Output 'linked'
New-Item -ItemType Junction -Path $linked -Target $suppBase|Out-Null
Reject-NoticeTest 'linked source rejected' {Get-VerifiedNoticeFile $linked '.cargo_vcs_info.json'}
$result=@{passed=$true;checks=$checks;network_used=$false;live_storage_opened=$false}
[IO.File]::WriteAllText((Join-Path $Output 'result.json'),($result|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output ('PASS notice collector: '+$checks.Count+' checks; '+$Output)
