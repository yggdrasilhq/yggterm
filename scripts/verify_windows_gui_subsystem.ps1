param(
  [Parameter(Mandatory = $true)]
  [string]$Path
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path)) {
  throw "file not found: $Path"
}

$resolved = (Resolve-Path -LiteralPath $Path).Path
$fs = [System.IO.File]::Open($resolved, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
try {
  $br = New-Object System.IO.BinaryReader($fs)
  $fs.Seek(0x3C, [System.IO.SeekOrigin]::Begin) | Out-Null
  $peOffset = $br.ReadInt32()
  $fs.Seek($peOffset + 0x5C, [System.IO.SeekOrigin]::Begin) | Out-Null
  $subsystem = [int]$br.ReadUInt16()
} finally {
  $fs.Dispose()
}

if ($subsystem -ne 2) {
  throw "expected GUI subsystem (2) but found $subsystem for $resolved"
}

Write-Host "GUI subsystem verified for $resolved"
