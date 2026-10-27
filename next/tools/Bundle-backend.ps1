param([Parameter(Mandatory)][string]$Stage)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$Stage=[IO.Path]::GetFullPath($Stage)
if(-not (Test-Path -LiteralPath $Stage -PathType Container)){throw 'Existing package staging directory required'}
if((Get-Item -LiteralPath $Stage).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Linked package stage is not allowed'}
function Read-Source([string]$Relative){
    return [IO.File]::ReadAllText((Join-Path $projectRoot $Relative),[Text.Encoding]::UTF8).Replace("`r`n","`n")
}
# Format-1 launchers have a frozen file allowlist. Keep source modules separate
# for review, but inline them as readable Python into existing approved entry
# points. No changed loader, hidden executable asset or runtime exec is needed.
$builder=Read-Source 'backend/kavychka_local/qwen_package.py'
$initial=Read-Source 'backend/kavychka_local/__init__.py'
$aliases=@'
import sys as _kavychka_bundle_sys
qwen_package = _kavychka_bundle_sys.modules[__name__]
_kavychka_bundle_sys.modules[__name__ + ".qwen_package"] = qwen_package
'@
$encoding=[Text.UTF8Encoding]::new($false)
$module=$initial+"`n# Inlined from the reviewed qwen_package.py (format-1 compatibility).`n"+$builder+"`n"+$aliases+"`n"
[IO.File]::WriteAllText((Join-Path $Stage 'backend/kavychka_local/__init__.py'),$module,$encoding)
$runtime=Read-Source 'backend/runtime_check.py'
$health=Read-Source 'backend/health_check.py'
$indented=(($runtime -split "`n") | ForEach-Object {'    '+$_}) -join "`n"
$prefix="# Inlined runtime check: preserve the format-1 backend file contract.`nimport sys as _kavychka_bundle_sys`nif __name__ == '__main__' and '--inference-runtime' in _kavychka_bundle_sys.argv:`n"
[IO.File]::WriteAllText((Join-Path $Stage 'backend/health_check.py'),$prefix+$indented+"`n    raise SystemExit(0)`n`n"+$health,$encoding)
