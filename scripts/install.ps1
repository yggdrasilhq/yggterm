param(
  [string]$Repo = $(if ($env:YGGTERM_REPO) { $env:YGGTERM_REPO } else { "yggdrasilhq/yggterm" }),
  [string]$InstallRoot = $(if ($env:YGGTERM_INSTALL_ROOT) { $env:YGGTERM_INSTALL_ROOT } else { Join-Path $env:LOCALAPPDATA "Yggterm" })
)

$ErrorActionPreference = "Stop"

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
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("yggterm-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDir | Out-Null

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
  $sourceMockCliExe = Join-Path $tempDir "yggterm-mock-cli-$targetLabel.exe"
  $sourceWebView2Loader = Join-Path $tempDir "WebView2Loader-$targetLabel.dll"
  if (-not (Test-Path $sourceWebView2Loader)) {
    $sourceWebView2Loader = Join-Path $tempDir "WebView2Loader.dll"
  }
  $installedExe = Join-Path $versionDir "yggterm.exe"
  $installedHeadlessExe = Join-Path $versionDir "yggterm-headless.exe"
  $installedMockCliExe = Join-Path $versionDir "yggterm-mock-cli.exe"
  $installedWebView2Loader = Join-Path $versionDir "WebView2Loader.dll"
  Copy-Item $sourceExe $installedExe -Force
  Copy-Item $sourceHeadlessExe $installedHeadlessExe -Force
  if (Test-Path $sourceMockCliExe) {
    Copy-Item $sourceMockCliExe $installedMockCliExe -Force
  } else {
    throw "release archive is missing yggterm-mock-cli.exe"
  }
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

  & $installedExe install integrate | Out-Null

  Write-Host "installed yggterm $version"
  Write-Host "binary: $installedExe"
} finally {
  Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
