# The native Windows seed-install probe. Runs the real install.ps1 against
# a local seed dist (zip + .sha256 + release-manifest.json) on a REAL
# Windows host, then proves the pointer/alias contract the launcher reads:
# active.json (schema 1, bare bin keys, real .exe files), the frozen
# seed-root marker, the immutable alias frontends with their .seed-alias
# metadata, a no-op re-run (idempotent, aliases untouched), and a refusal
# that leaves the previous pointer in charge. The seed ROOT is scoped under
# -Scratch, but the PATH aliases live at the installer's fixed location
# %LOCALAPPDATA%\ygg\bin — this probe runs the REAL installer, so it
# exercises that real directory by design (run it on a scratch host).
# No VM identities. Windows-only by purpose; it is never run on unix CI.
#
# Usage:  ./verify-windows-seed-install.ps1 -Dist <dir-with-seed-artifacts> -Scratch <dir>
param(
  [Parameter(Mandatory=$true)][string]$Dist,
  [Parameter(Mandatory=$true)][string]$Scratch
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repoScript = Join-Path $PSScriptRoot "install.ps1"
if (-not (Test-Path $repoScript)) { throw "install.ps1 not found beside this probe" }
New-Item -ItemType Directory -Path $Scratch -Force | Out-Null
$seedRoot = Join-Path $Scratch "seed"
New-Item -ItemType Directory -Path $seedRoot -Force | Out-Null

function Invoke-Installer([string]$Phase) {
  Write-Host "== $Phase"
  $env:YGGTERM_SEED_DIST = $Dist
  $env:YGGTERM_SEED_ROOT = $seedRoot
  $env:YGGTERM_INSTALL_SCRATCH = (Join-Path $Scratch "install-scratch")
  & $repoScript -Json
  if ($LASTEXITCODE -ne 0) { throw "install.ps1 $Phase exited $LASTEXITCODE" }
}

Invoke-Installer "fresh install (local dist; no latest lookup)"

# 1. Pointer contract: active.json is schema 1, keys are BARE program
#    names, the generation dir exists, and every bin file is a real .exe.
$pointerPath = Join-Path $seedRoot "active.json"
if (-not (Test-Path $pointerPath)) { throw "FAIL: no active.json pointer" }
$pointer = Get-Content $pointerPath -Raw | ConvertFrom-Json
if ($pointer.schema -ne 1) { throw "FAIL: pointer schema $($pointer.schema)" }
$binNames = @($pointer.bins.PSObject.Properties.Name)
foreach ($key in $binNames) {
  if ($key -like "*.exe") { throw "FAIL: pointer bin key '$key' carries a suffix; the contract is bare program names" }
}
$generationBin = Join-Path $seedRoot ("generations\" + $pointer.generation + "\bin")
if (-not (Test-Path $generationBin)) { throw "FAIL: generation bin dir missing: $generationBin" }
foreach ($key in $binNames) {
  if (-not (Test-Path (Join-Path $generationBin "$key.exe"))) { throw "FAIL: generation has no $key.exe" }
}
if ($pointer.generation -notmatch "^[0-9]+\.[0-9]+\.[0-9]+-[0-9a-f]{12}$") {
  throw "FAIL: generation name '$($pointer.generation)' is not version-digest12"
}
Write-Host "PASS: active.json schema 1, bare keys ($($binNames -join ',')), generation $($pointer.generation)"

# 2. Frozen root marker exists.
$markerPath = Join-Path $seedRoot "seed-root.json"
if (-not (Test-Path $markerPath)) { throw "FAIL: no seed-root.json marker" }
$markerDigest = (Get-FileHash -Algorithm SHA256 $markerPath).Hash.ToLowerInvariant()
Write-Host "PASS: seed-root marker $markerDigest"

# 3. Alias frontends: launcher copy + immutable .seed-alias metadata.
$binDir = Join-Path $env:LOCALAPPDATA "ygg\bin"
$aliasDigests = @{}
foreach ($name in @("ygg", "ynpm", "ynpx")) {
  $alias = Join-Path $binDir "$name.exe"
  $metaPath = "$alias.seed-alias"
  if (-not (Test-Path $alias)) { throw "FAIL: no alias $alias" }
  if (-not (Test-Path $metaPath)) { throw "FAIL: no alias metadata $metaPath" }
  $meta = Get-Content $metaPath -Raw | ConvertFrom-Json
  if ($meta.schema -ne 1) { throw "FAIL: $metaPath schema $($meta.schema)" }
  if ($meta.program -ne $name) { throw "FAIL: $metaPath program $($meta.program)" }
  if ($meta.seed_root -ne $seedRoot) { throw "FAIL: $metaPath seed_root $($meta.seed_root)" }
  if ($meta.root_marker_sha256 -ne $markerDigest) { throw "FAIL: $metaPath marker digest mismatch" }
  $aliasDigests[$name] = (Get-FileHash -Algorithm SHA256 $alias).Hash
}
Write-Host "PASS: aliases + immutable metadata installed"

# 4. The alias actually forwards: the version answer names the release
#    (the launcher resolved the pointer, hash gate passed, IR never loaded).
$manifest = Get-Content (Join-Path $Dist "release-manifest.json") -Raw | ConvertFrom-Json
$version = $manifest.release.version
$answer = & (Join-Path $binDir "ygg.exe") --version
if (-not ($answer -match [regex]::Escape($version))) {
  throw "FAIL: alias forwarded answer '$answer' does not name release $version"
}
Write-Host "PASS: alias forwards: $answer"

# 5. Upgrade is a pointer flip: re-run the installer; the alias bytes are
#    untouched (a running alias can never block an upgrade).
Invoke-Installer "re-run (idempotent activation; aliases immutable)"
foreach ($name in @("ygg", "ynpm", "ynpx")) {
  $alias = Join-Path $binDir "$name.exe"
  $now = (Get-FileHash -Algorithm SHA256 $alias).Hash
  if ($now -ne $aliasDigests[$name]) { throw "FAIL: alias $name was rewritten by a re-run" }
}
Write-Host "PASS: aliases immutable across re-run"

# 6. Refusal leaves the pointer in charge: a corrupt archive copy must fail
#    the checksum gate WITHOUT touching active.json or the aliases.
$before = Get-Content $pointerPath -Raw
$badDist = Join-Path $Scratch "bad-dist"
New-Item -ItemType Directory -Path $badDist -Force | Out-Null
Copy-Item (Join-Path $Dist "release-manifest.json") $badDist
Copy-Item (Join-Path $Dist "*") $badDist -Exclude "release-manifest.json"
$badArchive = Get-ChildItem $badDist -Filter "*.zip" | Select-Object -First 1
if (-not $badArchive) { throw "probe needs a .zip seed archive in -Dist" }
$bytes = [System.IO.File]::ReadAllBytes($badArchive.FullName)
$bytes[100] = $bytes[100] -bxor 0xFF
[System.IO.File]::WriteAllBytes($badArchive.FullName, $bytes)
# keep the ORIGINAL checksum sidecar so the gate sees a mismatch
$env:YGGTERM_SEED_DIST = $badDist
$env:YGGTERM_SEED_ROOT = $seedRoot
$env:YGGTERM_INSTALL_SCRATCH = (Join-Path $Scratch "install-scratch-bad")
$refused = $false
try { & $repoScript -Json } catch { $refused = $true }
if (-not $refused -and $LASTEXITCODE -eq 0) { throw "FAIL: corrupt archive activated" }
$after = Get-Content $pointerPath -Raw
if ($before -ne $after) { throw "FAIL: a refused install moved the active pointer" }
Write-Host "PASS: refusal leaves pointer and generations in charge"

# 7. Broken alias metadata refuses instead of falling back into command
#    IR (the launcher copy would otherwise run as a plain frontend). The
#    metadata is restored afterwards.
$metaPath = Join-Path $binDir "ygg.exe.seed-alias"
$metaBackup = Get-Content $metaPath -Raw
Set-Content -Path $metaPath -Value "not json" -NoNewline
& (Join-Path $binDir "ygg.exe") --version *> $null
if ($LASTEXITCODE -eq 0) { throw "FAIL: broken alias metadata fell back to IR" }
Set-Content -Path $metaPath -Value $metaBackup -NoNewline
Write-Host "PASS: broken alias metadata refuses instead of loading IR"

# 8. A re-rooted seed refuses through the alias gate: remove the marker
#    and the alias must fail (run LAST; it corrupts the scratch seed root).
Remove-Item $markerPath
& (Join-Path $binDir "ygg.exe") --version *> $null
if ($LASTEXITCODE -eq 0) { throw "FAIL: alias forwarded with its root marker gone" }
Write-Host "PASS: alias refuses a re-rooted seed"

Write-Host ""
Write-Host "ALL WINDOWS SEED INSTALL PROBES PASSED"
