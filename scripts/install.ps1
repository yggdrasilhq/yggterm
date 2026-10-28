param(
  [string]$Repo = $(if ($env:YGGTERM_REPO) { $env:YGGTERM_REPO } else { "yggdrasilhq/yggterm" }),
  [string]$InstallRoot = $(if ($env:YGGTERM_INSTALL_ROOT) { $env:YGGTERM_INSTALL_ROOT } else { Join-Path $env:LOCALAPPDATA "Yggterm" }),
  [switch]$Legacy,
  [switch]$Seed
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# ------------------------------------------------------------- ygg4 seed path
# The GUI-free seed install is the DEFAULT 4.0 entry (-Seed still selects
# it). The 3.x direct/GUI installer stays available EXPLICITLY (-Legacy or
# YGGTERM_LEGACY=1) for version compatibility and is never removed. The
# seed path never touches the direct/GUI install root, never starts a
# daemon, needs no Python and no GUI, and delegates the staging/publication
# transaction to `ynpm seed-activate` extracted from the verified archive
# itself.
if (-not ($Legacy -or $env:YGGTERM_LEGACY -eq "1")) {
  $seedRoot = if ($env:YGGTERM_SEED_ROOT) { $env:YGGTERM_SEED_ROOT } else { Join-Path $env:USERPROFILE ".yggterm\seed" }
  $seedDist = $env:YGGTERM_SEED_DIST
  $scratchRoot = if ($env:YGGTERM_INSTALL_SCRATCH) { $env:YGGTERM_INSTALL_SCRATCH } else { Join-Path $env:USERPROFILE ".yggterm\scratchpad\ygg4-install" }
  New-Item -ItemType Directory -Path $scratchRoot -Force | Out-Null
  $tempDir = Join-Path $scratchRoot ("seed-" + [guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

  $arch = if ($env:PROCESSOR_ARCHITECTURE -match "ARM64") { "aarch64" } else { "x86_64" }
  $targetLabel = if ($arch -eq "aarch64") { "windows-aarch64" } else { "windows-x86_64" }
  $archiveName = "ygg-seed-$targetLabel.zip"

  try {
    $version = $env:YGGTERM_VERSION
    if (-not $version) {
      $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "yggterm-installer" }
      $version = $release.tag_name.TrimStart("v")
    }
    $version = $version.TrimStart("v")
    if (-not $version) { throw "could not resolve the seed version to install" }
    $tag = "v$version"

    $archivePath = Join-Path $tempDir $archiveName
    $checksumPath = "$archivePath.sha256"
    $manifestPath = Join-Path $tempDir "release-manifest.json"
    if ($seedDist) {
      $archiveSource = Join-Path $seedDist $archiveName
      if (-not (Test-Path $archiveSource)) { throw "local seed dist has no $archiveName" }
      Copy-Item $archiveSource $archivePath
      $checksumSource = "$archiveSource.sha256"
      if (-not (Test-Path $checksumSource)) { throw "local seed dist has no $archiveName.sha256" }
      Copy-Item $checksumSource $checksumPath
      $manifestSource = Join-Path $seedDist "release-manifest.json"
      if (-not (Test-Path $manifestSource)) { throw "local seed dist has no release-manifest.json" }
      Copy-Item $manifestSource $manifestPath
    } else {
      $baseUrl = "https://github.com/$Repo/releases/download/$tag"
      Write-Host "downloading ygg4 seed $version for $targetLabel"
      Invoke-WebRequest -Uri "$baseUrl/$archiveName" -OutFile $archivePath
      # Checksum verification is mandatory: an unverifiable archive fails.
      try {
        Invoke-WebRequest -Uri "$baseUrl/$archiveName.sha256" -OutFile $checksumPath
      } catch {
        throw "no checksum published for $archiveName; refusing an unverifiable download"
      }
      Invoke-WebRequest -Uri "$baseUrl/release-manifest.json" -OutFile $manifestPath
    }
    $expected = (Get-Content $checksumPath).Split(" ")[0].Trim().ToLowerInvariant()
    if ($expected -notmatch "^[0-9a-f]{64}$") { throw "published checksum for $archiveName is not a sha256 digest" }
    $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "seed checksum verification failed for $archiveName" }

    # Extract only the transaction owner; seed-activate re-extracts and
    # verifies the archive itself, staged before publication.
    $bootstrapDir = Join-Path $tempDir "bootstrap"
    Expand-Archive -Path $archivePath -DestinationPath $bootstrapDir
    $ynpmBootstrap = Join-Path $bootstrapDir "ynpm.exe"
    if (-not (Test-Path $ynpmBootstrap)) { throw "seed archive did not carry a ynpm.exe bootstrap" }

    Write-Host "activating seed generation (staged by ynpm seed-activate)"
    & $ynpmBootstrap seed-activate `
      --archive $archivePath `
      --manifest $manifestPath `
      --package ygg-seed `
      --expect-version $version `
      --json
    if ($LASTEXITCODE -ne 0) { throw "seed-activate refused the transaction (exit $LASTEXITCODE)" }

    # Command publication: %LOCALAPPDATA%\ygg\bin is seed-owned by
    # construction, so refreshing these copies never races another owner.
    # A file in use fails the copy loudly (Windows running-binary
    # constraint); the activated generation itself is never touched, so
    # re-running the installer after the process exits repairs publication.
    $binDir = Join-Path $env:LOCALAPPDATA "ygg\bin"
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    $generationBin = Join-Path $seedRoot "current\bin"
    if (-not (Test-Path $generationBin)) { throw "activated seed has no $generationBin" }
    foreach ($name in @("ygg.exe", "ynpm.exe", "ynpx.exe")) {
      $source = Join-Path $generationBin $name
      $dest = Join-Path $binDir $name
      $staged = Join-Path $binDir ($name + ".seed-publish")
      Copy-Item $source $staged -Force
      if (Test-Path $dest) {
        try { Remove-Item $dest -Force } catch {
          Remove-Item $staged -Force -ErrorAction SilentlyContinue
          throw "$dest is locked (running process); close it and rerun the installer"
        }
      }
      Move-Item $staged $dest -Force
    }
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = @()
    if ($userPath) { $pathParts = $userPath -split ";" | Where-Object { $_ } }
    if ($pathParts -notcontains $binDir) {
      [Environment]::SetEnvironmentVariable("Path", (($pathParts + $binDir) -join ";"), "User")
    }
    $env:Path = "$binDir;$env:Path"

    Write-Host "installed ygg4 seed $version"
    Write-Host "commands: $binDir\{ygg,ynpm,ynpx}.exe"
    Write-Host "this host can now be operated over ssh as a ygg system; that is a remote-operator capability, not local worker readiness"
    Write-Host "previous generations are preserved under $seedRoot\generations"
    exit 0
  } finally {
    Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
  }
}

$apiUrl = "https://api.github.com/repos/$Repo/releases/latest"
$release = Invoke-RestMethod -Uri $apiUrl -Headers @{ "User-Agent" = "yggterm-installer" }

$arch = if ($env:PROCESSOR_ARCHITECTURE -match "ARM64") { "aarch64" } else { "x86_64" }
$targetLabel = switch ($arch) {
  "aarch64" { "windows-aarch64" }
  default { "windows-x86_64" }
}

$archiveName = "yggterm-$targetLabel.tar.gz"
$checksumName = "$archiveName.sha256"
$archiveAsset = $release.assets | Where-Object { $_.name -eq $archiveName } | Select-Object -First 1
$checksumAsset = $release.assets | Where-Object { $_.name -eq $checksumName } | Select-Object -First 1

if (-not $archiveAsset) {
  throw "failed to locate a compatible release asset for $targetLabel"
}

$version = $release.tag_name.TrimStart("v")
$statePath = Join-Path $InstallRoot "install-state.json"
if (Test-Path $statePath) {
  $existingState = Get-Content $statePath -Raw | ConvertFrom-Json
  $existingVersion = $existingState.active_version
  if ($existingVersion) {
    $existingYnpm = Join-Path $InstallRoot "versions\$existingVersion\ynpm.exe"
    if (Test-Path $existingYnpm) {
      $env:YGGTERM_DIRECT_INSTALL_ROOT = $InstallRoot
      & $existingYnpm self-update
      if ($LASTEXITCODE -ne 0) {
        throw "ynpm self-update exited with status $LASTEXITCODE"
      }
      exit 0
    }
  }
}
$scratchRoot = if ($env:YGGTERM_INSTALL_SCRATCH) {
  $env:YGGTERM_INSTALL_SCRATCH
} else {
  Join-Path $env:USERPROFILE ".yggterm\scratchpad\ynpm"
}
$tempDir = Join-Path $scratchRoot ("install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $scratchRoot -Force | Out-Null
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
  $archivePath = Join-Path $tempDir "yggterm.tar.gz"
  Invoke-WebRequest -Uri $archiveAsset.browser_download_url -OutFile $archivePath
  if ($checksumAsset) {
    $checksumPath = Join-Path $tempDir "yggterm.tar.gz.sha256"
    Invoke-WebRequest -Uri $checksumAsset.browser_download_url -OutFile $checksumPath
    $expected = (Get-Content $checksumPath).Split(" ")[0].Trim()
    $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
    if ($expected.ToLowerInvariant() -ne $actual) {
      throw "checksum verification failed"
    }
  }

  $versionDir = Join-Path $InstallRoot "versions\$version"
  New-Item -ItemType Directory -Path $versionDir -Force | Out-Null
  tar -xzf $archivePath -C $tempDir

  $sourceExe = Join-Path $tempDir "yggterm-$targetLabel.exe"
  $sourceHeadlessExe = Join-Path $tempDir "yggterm-headless-$targetLabel.exe"
  $sourceYnpmExe = Join-Path $tempDir "ynpm-$targetLabel.exe"
  $sourceYnpxExe = Join-Path $tempDir "ynpx-$targetLabel.exe"
  $sourceWebView2Loader = Join-Path $tempDir "WebView2Loader-$targetLabel.dll"
  if (-not (Test-Path $sourceWebView2Loader)) {
    $sourceWebView2Loader = Join-Path $tempDir "WebView2Loader.dll"
  }
  $installedExe = Join-Path $versionDir "yggterm.exe"
  $installedHeadlessExe = Join-Path $versionDir "yggterm-headless.exe"
  $installedYnpmExe = Join-Path $versionDir "ynpm.exe"
  $installedYnpxExe = Join-Path $versionDir "ynpx.exe"
  $installedWebView2Loader = Join-Path $versionDir "WebView2Loader.dll"
  Copy-Item $sourceExe $installedExe -Force
  Copy-Item $sourceHeadlessExe $installedHeadlessExe -Force
  Copy-Item $sourceYnpmExe $installedYnpmExe -Force
  Copy-Item $sourceYnpxExe $installedYnpxExe -Force
  if (Test-Path $sourceWebView2Loader) {
    Copy-Item $sourceWebView2Loader $installedWebView2Loader -Force
  } else {
    throw "release archive is missing WebView2Loader.dll"
  }

  $state = @{
    channel = "direct"
    repo = $Repo
    asset_label = $targetLabel
    active_version = $version
    active_executable = $installedExe
    icon_revision = $version
  } | ConvertTo-Json
  $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText((Join-Path $InstallRoot "install-state.json"), $state, $utf8NoBom)

  $commandDir = Join-Path $InstallRoot "bin"
  New-Item -ItemType Directory -Path $commandDir -Force | Out-Null
  Copy-Item $installedYnpmExe (Join-Path $commandDir "ynpm.exe") -Force
  Copy-Item $installedYnpxExe (Join-Path $commandDir "ynpx.exe") -Force
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $pathParts = @()
  if ($userPath) { $pathParts = $userPath -split ";" | Where-Object { $_ } }
  if ($pathParts -notcontains $commandDir) {
    [Environment]::SetEnvironmentVariable("Path", (($pathParts + $commandDir) -join ";"), "User")
  }
  $env:Path = "$commandDir;$env:Path"

  & $installedExe install integrate | Out-Null

  Write-Host "installed yggterm $version"
  Write-Host "binary: $installedExe"
} finally {
  Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
