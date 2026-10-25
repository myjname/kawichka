[CmdletBinding()]
param([switch]$PlanOnly)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskEvidence = Join-Path $taskRoot 'work/p5-acceptance-20261023'
$taskDescriptor = Join-Path $taskEvidence 'full-volume-test-binary.json'
if (!(Test-Path -LiteralPath $taskDescriptor)) { throw 'Build and fingerprint the P5 storage test binary first; descriptor is missing.' }
$taskBinaryRecord = Get-Content -LiteralPath $taskDescriptor -Raw | ConvertFrom-Json
$taskBinary = [IO.Path]::GetFullPath((Join-Path $taskRoot $taskBinaryRecord.path))
if ($taskBinaryRecord.format -ne 1 -or !$taskBinary.StartsWith((Join-Path $taskRoot 'work/'), [StringComparison]::OrdinalIgnoreCase) -or
    [IO.Path]::GetFileName($taskBinary) -notlike 'kavychka_storage-*.exe' -or
    (Get-FileHash -LiteralPath $taskBinary -Algorithm SHA256).Hash -ne $taskBinaryRecord.sha256) { throw 'Unexpected storage test binary; do not run an unverified executable as administrator.' }
$taskPlan = [ordered]@{
    virtual_disk_mib = 128
    location = $taskEvidence
    formats_only = 'A newly created, verified FileBackedVirtual disk attached from the new VHDX in this folder'
    ordinary_disks = 'Never selected or formatted; test also checks capacity, owner and volume GUID'
    source_data = 'New synthetic fixtures only; current application and system storage pointers untouched'
    cleanup = 'Detach only this VHDX; retain the image, test output and result for inspection'
    test_binary_sha256 = $taskBinaryRecord.sha256
}
if ($PlanOnly) { $taskPlan | ConvertTo-Json; return }
$taskPrincipal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (!$taskPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator rights are required to attach a disposable VHDX. No disk changes were made. Use -PlanOnly to review.' }
$taskRun = Join-Path $taskEvidence ('full-volume-' + [Guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $taskRun) { throw 'Expected a new evidence folder.' }
New-Item -ItemType Directory -Path $taskRun | Out-Null
$taskImage = [IO.Path]::GetFullPath((Join-Path $taskRun 'disk-full.vhdx'))
if (!$taskImage.StartsWith(([IO.Path]::GetFullPath($taskEvidence) + '\'), [StringComparison]::OrdinalIgnoreCase) -or $taskImage -match '["\r\n]' -or $taskImage -match '[^\x00-\x7F]') { throw 'Virtual image path escaped the private ASCII evidence directory.' }
$taskExistingDisks = @(Get-Disk | Select-Object -ExpandProperty Number)
$taskScript = Join-Path $taskRun 'create-vdisk.txt'
@("create vdisk file=`"$taskImage`" maximum=128 type=expandable", "select vdisk file=`"$taskImage`"", 'attach vdisk', 'exit') | Set-Content -LiteralPath $taskScript -Encoding ASCII
$taskPlan | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRun 'plan.json') -Encoding UTF8

function Get-TaskVirtualDisk {
    $taskImageObject = Get-DiskImage -ImagePath $taskImage
    if (!$taskImageObject.Attached -or [IO.Path]::GetFullPath($taskImageObject.ImagePath) -ne $taskImage) { throw 'Exact VHDX identity/attachment check failed.' }
    $taskDiskObject = $taskImageObject | Get-Disk
    if ($null -eq $taskDiskObject -or $taskDiskObject.Size -ne 128MB -or [int]$taskDiskObject.BusType -notin @(14,15) -or $taskDiskObject.Number -in $taskExistingDisks) { throw 'Refusing an existing, physical, differently sized or unrelated disk.' }
    return $taskDiskObject
}

$taskOldVolume = $env:KAVYCHKA_P5_FULL_VOLUME
try {
    & "$env:SystemRoot\System32\diskpart.exe" /s $taskScript *> (Join-Path $taskRun 'diskpart.log')
    $taskDisk = Get-TaskVirtualDisk
    if ($taskDisk.PartitionStyle -ne 'RAW') { throw 'Only a fresh uninitialized virtual disk may be formatted.' }
    $taskDisk = Initialize-Disk -InputObject (Get-TaskVirtualDisk) -PartitionStyle GPT -PassThru
    $taskPartition = New-Partition -InputObject (Get-TaskVirtualDisk) -UseMaximumSize -AssignDriveLetter
    $taskCheckDisk = Get-TaskVirtualDisk
    if ($taskPartition.DiskNumber -ne $taskCheckDisk.Number -or !$taskPartition.DriveLetter -or
        ([string]$taskPartition.DriveLetter + ':') -eq $env:SystemDrive -or
        ([string]$taskPartition.DriveLetter + ':') -eq [IO.Path]::GetPathRoot($taskRoot).TrimEnd('\')) { throw 'Partition does not belong to the verified private virtual disk.' }
    $taskVolume = Format-Volume -Partition $taskPartition -FileSystem NTFS -NewFileSystemLabel 'Kavychka-P5-FULL' -Force -Confirm:$false
    if ($taskVolume.FileSystemLabel -ne 'Kavychka-P5-FULL' -or $taskVolume.Size -gt 160000000 -or $taskVolume.Size -lt 64000000) { throw 'Unexpected test volume.' }
    $taskMount = [string]$taskPartition.DriveLetter + ':\'
    $taskOwner = @{format=1;kind='Kavychka.P5.ENOSPC.v1';image=$taskImage;volume_guid=$taskVolume.UniqueId;result=(Join-Path $taskRun 'result.json')}
    $taskOwner | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskMount 'kavychka-p5-volume.owner.json') -Encoding UTF8
    # Windows PowerShell 5.1 writes a UTF-8 BOM; JSON reader expects plain UTF-8.
    [IO.File]::WriteAllText((Join-Path $taskMount 'kavychka-p5-volume.owner.json'),($taskOwner | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    $env:KAVYCHKA_P5_FULL_VOLUME = $taskMount
    & $taskBinary --exact profile::full_volume_tests::actual_virtual_volume_full_during_zip_and_restore_is_recoverable --ignored --nocapture *> (Join-Path $taskRun 'test.log')
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath (Join-Path $taskRun 'result.json'))) { throw "P5 full-volume test failed; inspect $taskRun. Data/image retained." }
    Write-Output "PASS: $taskRun"
}
finally {
    $env:KAVYCHKA_P5_FULL_VOLUME = $taskOldVolume
    if (Test-Path -LiteralPath $taskImage) {
        $taskAttached = Get-DiskImage -ImagePath $taskImage -ErrorAction SilentlyContinue
        if ($taskAttached -and $taskAttached.Attached -and [IO.Path]::GetFullPath($taskAttached.ImagePath) -eq $taskImage) { Dismount-DiskImage -ImagePath $taskImage | Out-Null }
    }
}
