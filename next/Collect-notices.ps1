param([Parameter(Mandatory)][string]$Output,[switch]$Offline,[switch]$AllowMissing)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'tools/Notice-functions.ps1')
$Output=[IO.Path]::GetFullPath($Output)
if(Test-Path -LiteralPath $Output){throw 'Choose a new notice directory'}
[IO.Directory]::CreateDirectory($Output)|Out-Null
$metadataArgs=@('metadata','--manifest-path',(Join-Path $PSScriptRoot 'Cargo.toml'),'--locked','--format-version','1','--filter-platform','x86_64-pc-windows-msvc')
$treeArgs=@('tree','--manifest-path',(Join-Path $PSScriptRoot 'Cargo.toml'),'-p','kavychka-app','-p','kavychka-maintenance','--target','x86_64-pc-windows-msvc','--edges','normal,build','--locked','--prefix','none','--format','{p}')
if($Offline){$metadataArgs+='--offline';$treeArgs+='--offline'}
$metadataText=(& cargo @metadataArgs) -join "`n"
if($LASTEXITCODE -ne 0){throw 'Cargo metadata failed'}
$metadata=$metadataText|ConvertFrom-Json
$tree=@(& cargo @treeArgs)
if($LASTEXITCODE -ne 0){throw 'Cargo tree failed'}
$packages=@(Get-ActiveNoticePackages -Metadata $metadata -TreeLines $tree)
$supplementRoot=Join-Path $PSScriptRoot 'licenses/supplemental'
$supplement=Get-Content -LiteralPath (Join-Path $supplementRoot 'sources.json') -Raw|ConvertFrom-Json
if($supplement.format -ne 1){throw 'Unsupported supplemental notices'}
$checksums=Get-NoticeLockChecksums -LockText (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Cargo.lock') -Raw)
$inventory=@()
foreach($package in $packages){
    $id=$package.name+'-'+$package.version
    if($id -notmatch '^[a-zA-Z0-9_.+-]+$'){throw 'Unexpected package identifier'}
    $inputs=@(Get-NoticeInputs -Package $package -SupplementRoot $supplementRoot -Supplement $supplement -Checksums $checksums)
    $files=@();$destinations=@{}
    foreach($noticeInput in $inputs){
        if($destinations.ContainsKey($noticeInput.relative)){throw 'Duplicate notice destination'}
        $destinations[$noticeInput.relative]=$true
        $target=Join-Path $Output ($id+'/'+$noticeInput.relative)
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))|Out-Null
        Copy-Item -LiteralPath $noticeInput.file.FullName -Destination $target
        $files+=@{path=($id+'/'+$noticeInput.relative);sha256=(Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant();origin=$noticeInput.origin}
    }
    $inventory+=@{name=$package.name;version=$package.version;license=$package.license;repository=$package.repository;files=$files;license_text_present=(@($inputs|Where-Object is_license).Count -gt 0)}
}
$missing=@($inventory|Where-Object {-not $_.license_text_present})
$record=@{format=2;target='x86_64-pc-windows-msvc';roots=@('kavychka-app','kavychka-maintenance');scope='Active cargo tree normal/build dependencies for the selected Windows build; includes build-time tools, not a binary-link or legal certification.';metadata_packages=$metadata.packages.Count;missing_license_text_count=$missing.Count;packages=$inventory}
[IO.File]::WriteAllText((Join-Path $Output 'inventory.json'),($record|ConvertTo-Json -Depth 14),[Text.UTF8Encoding]::new($false))
Write-Output ('Notices collected: '+$inventory.Count+' active packages; '+$missing.Count+' without bundled license text')
if($missing.Count -gt 0 -and -not $AllowMissing){throw ('Missing license text: '+(($missing|ForEach-Object {$_.name+' '+$_.version}) -join ', '))}
