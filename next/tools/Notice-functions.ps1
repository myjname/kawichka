# Offline notice helpers. Dot-sourcing does not start processes or mutate files.
function Get-ActiveNoticePackages {
    param([object]$Metadata,[string[]]$TreeLines)
    $byKey=@{}
    foreach($p in $Metadata.packages){$key=$p.name+'@'+$p.version;$byKey[$key]=@($byKey[$key])+@($p)}
    $selected=@{}
    foreach($line in $TreeLines){
        if([string]::IsNullOrWhiteSpace($line)){continue}
        if($line -notmatch '^([A-Za-z0-9_-]+) v([^\s]+)(?:\s.*)?$'){throw 'Unexpected cargo tree row'}
        $key=$Matches[1]+'@'+$Matches[2]
        $candidates=@($byKey[$key]|Where-Object {$_})
        if($candidates.Count -ne 1){throw ('Missing or ambiguous Cargo package: '+$key)}
        $selected[$key]=$candidates[0]
    }
    foreach($name in @('kavychka-app','kavychka-maintenance')){
        if(@($selected.Values|Where-Object {$_.name -eq $name -and -not $_.source}).Count -ne 1){throw ('Missing build root: '+$name)}
    }
    @($selected.Values|Where-Object source|Sort-Object name,version)
}

function Get-NoticeLockChecksums {
    param([string]$LockText)
    $pins=@{}
    foreach($block in ($LockText -split '(?m)^\[\[package\]\]\s*$')){
        if($block -notmatch '(?m)^source = "registry\+[^"\r\n]+"\s*$'){continue}
        if($block -notmatch '(?m)^name = "([A-Za-z0-9_-]+)"\s*$'){throw 'Registry package without name'}
        $name=$Matches[1]
        if($block -notmatch '(?m)^version = "([A-Za-z0-9.+-]+)"\s*$'){throw 'Registry package without version'}
        $key=$name+'@'+$Matches[1]
        if($block -notmatch '(?m)^checksum = "([a-f0-9]{64})"\s*$'){throw ('Registry checksum missing: '+$key)}
        if($pins.ContainsKey($key)){throw ('Ambiguous registry checksum: '+$key)}
        $pins[$key]=$Matches[1]
    }
    $pins
}

function Get-VerifiedNoticeFile {
    param([string]$Base,[string]$Relative,[string]$ExpectedHash='')
    if($Relative -notmatch '^[A-Za-z0-9_./+-]+$' -or @($Relative.Split('/')|Where-Object {$_ -in @('','.', '..')}).Count){throw 'Unexpected notice filename'}
    $basePath=[IO.Path]::GetFullPath($Base).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $full=[IO.Path]::GetFullPath((Join-Path $basePath $Relative))
    if(-not $full.StartsWith($basePath+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Notice path escaped source'}
    $current=$basePath
    foreach($part in @('')+$Relative.Split('/')){
        if($part){$current=Join-Path $current $part}
        $item=Get-Item -LiteralPath $current -Force
        if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Linked notice source rejected'}
    }
    if($item.PSIsContainer -or $item.Length -eq 0 -or $item.Length -gt 1000000){throw 'Invalid notice file size/type'}
    if($ExpectedHash -and ((Get-FileHash -LiteralPath $full -Algorithm SHA256).Hash -ne $ExpectedHash)){throw 'Supplemental notice checksum differs'}
    $item
}

function Get-NoticeInputs {
    param([object]$Package,[string]$SupplementRoot,[object]$Supplement,[hashtable]$Checksums)
    $base=[IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Package.manifest_path))
    $sources=@(Get-ChildItem -LiteralPath $base -File|Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE)([._-].*)?$')
    foreach($sub in @('LICENSES','licenses')){
        $folder=Join-Path $base $sub
        if(Test-Path -LiteralPath $folder){
            if((Get-Item -LiteralPath $folder -Force).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Linked notice directory rejected'}
            $sources+=@(Get-ChildItem -LiteralPath $folder -File)
        }
    }
    if($Package.license_file){$sources+=@(Get-VerifiedNoticeFile -Base $base -Relative ($Package.license_file.Replace('\','/')))}
    foreach($source in ($sources|Sort-Object FullName -Unique)){
        $relative=$source.FullName.Substring($base.TrimEnd('\').Length+1).Replace('\','/')
        $file=Get-VerifiedNoticeFile -Base $base -Relative $relative
        $isLicense=$file.Name -match '^(LICENSE|LICENCE|COPYING)' -or ($relative -match '^(LICENSES|licenses)/' -and $file.Name -notmatch '^(README|NOTICE|AUTHORS|COPYRIGHT)') -or ($Package.license_file -and $relative -eq $Package.license_file.Replace('\','/'))
        [PSCustomObject]@{file=$file;relative=$relative;is_license=[bool]$isLicense;origin=@{kind='crate';path=$relative}}
    }
    $extra=@($Supplement.packages|Where-Object {$_.name -eq $Package.name -and $_.version -eq $Package.version})
    if($extra.Count -gt 1){throw 'Duplicate supplemental package'}
    if($extra.Count -eq 0){return}
    $extra=$extra[0];$key=$Package.name+'@'+$Package.version
    if($extra.license -ne $Package.license -or $extra.repository -ne $Package.repository -or -not $Checksums.ContainsKey($key) -or $Checksums[$key] -ne $extra.crate_checksum){throw ('Supplement does not match locked package: '+$key)}
    $vcs=Get-Content -LiteralPath (Join-Path $base '.cargo_vcs_info.json') -Raw|ConvertFrom-Json
    if($vcs.git.sha1 -ne $extra.vcs_revision -or $vcs.git.dirty){throw ('Supplement revision mismatch: '+$key)}
    foreach($entry in $extra.files){
        $repo=$extra.repository.TrimEnd('/').Replace('https://github.com/','')
        $expectedUrl='https://raw.githubusercontent.com/'+$repo+'/'+$extra.vcs_revision+'/'+$entry.upstream_path
        if($entry.url -ne $expectedUrl -or $entry.sha256 -notmatch '^[a-f0-9]{64}$' -or $entry.git_blob_sha1 -notmatch '^[a-f0-9]{40}$'){throw 'Invalid supplemental provenance'}
        $file=Get-VerifiedNoticeFile -Base $SupplementRoot -Relative $entry.path -ExpectedHash $entry.sha256
        [PSCustomObject]@{file=$file;relative=('upstream/'+$file.Name);is_license=($entry.upstream_path -match '^(LICENSE|LICENCE|COPYING)');origin=@{kind='pinned_upstream';url=$entry.url;revision=$extra.vcs_revision;git_blob_sha1=$entry.git_blob_sha1}}
    }
}
