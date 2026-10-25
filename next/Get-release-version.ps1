param([Parameter(Mandatory)][string]$Tag)
$ErrorActionPreference='Stop'
if($Tag -cmatch '\Anext-v((?:0|[1-9][0-9]{0,7})\.(?:0|[1-9][0-9]{0,7})\.(?:0|[1-9][0-9]{0,7}))\z'){
    return $Matches[1]
}
throw 'Use an exact stable Rust tag: next-vX.Y.Z (no leading zeroes or suffix).'
