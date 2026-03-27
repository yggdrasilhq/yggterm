use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_RELEASE_REPO: &str = "yggdrasilhq/yggterm";
const INSTALL_STATE_FILENAME: &str = "install-state.json";
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallChannel {
    Direct,
    Deb,
    Homebrew,
    Winget,
    Scoop,
    Flatpak,
    Snap,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    Auto,
    NotifyOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallContext {
    pub channel: InstallChannel,
    pub update_policy: UpdatePolicy,
    pub repo: String,
    pub asset_label: String,
    pub current_version: String,
    pub executable_path: PathBuf,
    pub managed_root: Option<PathBuf>,
    pub manager_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectInstallState {
    channel: InstallChannel,
    repo: String,
    asset_label: String,
    active_version: String,
    active_executable: PathBuf,
    icon_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseUpdate {
    pub version: String,
    pub tag_name: String,
    pub archive_url: String,
    pub checksum_url: Option<String>,
}

pub fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn current_asset_label() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let label = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        _ => anyhow::bail!("unsupported release target {os}-{arch}"),
    };
    Ok(label.to_string())
}

pub fn detect_install_context(executable_path: &Path) -> Result<InstallContext> {
    let executable_path = executable_path
        .canonicalize()
        .unwrap_or_else(|_| executable_path.to_path_buf());
    if let Some((root, state)) = find_direct_install_state(&executable_path)? {
        return Ok(InstallContext {
            channel: InstallChannel::Direct,
            update_policy: UpdatePolicy::Auto,
            repo: state.repo,
            asset_label: state.asset_label,
            current_version: state.active_version,
            executable_path,
            managed_root: Some(root),
            manager_hint: Some("Direct install".to_string()),
        });
    }

    let asset_label = current_asset_label().unwrap_or_else(|_| "unknown".to_string());
    let repo = DEFAULT_RELEASE_REPO.to_string();
    let current_version = current_version();
    let executable_text = executable_path.to_string_lossy().replace('\\', "/");

    if std::env::var_os("FLATPAK_ID").is_some() || executable_text.starts_with("/app/") {
        return Ok(notify_only_context(
            InstallChannel::Flatpak,
            "Installed via Flatpak. Use your Flatpak tooling to update Yggterm.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }
    if std::env::var_os("SNAP").is_some() || executable_text.starts_with("/snap/") {
        return Ok(notify_only_context(
            InstallChannel::Snap,
            "Installed via Snap. Use your Snap tooling to update Yggterm.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }
    if executable_text.contains("/.linuxbrew/")
        || executable_text.contains("/homebrew/")
        || executable_text.contains("/Cellar/")
        || executable_text.contains("/Homebrew/")
    {
        return Ok(notify_only_context(
            InstallChannel::Homebrew,
            "Installed via Homebrew. Run `brew upgrade yggterm` when a newer release is available.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }
    if executable_text.contains("/scoop/apps/") || executable_text.contains("\\scoop\\apps\\") {
        return Ok(notify_only_context(
            InstallChannel::Scoop,
            "Installed via Scoop. Run `scoop update yggterm` when a newer release is available.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }
    if executable_text.contains("WindowsApps") {
        return Ok(notify_only_context(
            InstallChannel::Winget,
            "Installed via Winget or the Windows package manager. Update Yggterm through Winget.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }
    if cfg!(target_os = "linux")
        && (executable_text.starts_with("/usr/")
            || executable_text.starts_with("/opt/")
            || executable_text.starts_with("/bin/"))
    {
        return Ok(notify_only_context(
            InstallChannel::Deb,
            "Installed via a system package. Use your package manager to update Yggterm.",
            repo,
            asset_label,
            current_version,
            executable_path,
        ));
    }

    Ok(InstallContext {
        channel: InstallChannel::Unknown,
        update_policy: UpdatePolicy::NotifyOnly,
        repo,
        asset_label,
        current_version,
        executable_path,
        managed_root: None,
        manager_hint: Some("Development build or unmanaged install".to_string()),
    })
}

fn notify_only_context(
    channel: InstallChannel,
    hint: &str,
    repo: String,
    asset_label: String,
    current_version: String,
    executable_path: PathBuf,
) -> InstallContext {
    InstallContext {
        channel,
        update_policy: UpdatePolicy::NotifyOnly,
        repo,
        asset_label,
        current_version,
        executable_path,
        managed_root: None,
        manager_hint: Some(hint.to_string()),
    }
}

fn find_direct_install_state(
    executable_path: &Path,
) -> Result<Option<(PathBuf, DirectInstallState)>> {
    for ancestor in executable_path.ancestors() {
        let path = ancestor.join(INSTALL_STATE_FILENAME);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read install state {}", path.display()))?;
        let state: DirectInstallState = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse install state {}", path.display()))?;
        if state.channel == InstallChannel::Direct {
            return Ok(Some((ancestor.to_path_buf(), state)));
        }
    }
    Ok(None)
}

pub fn direct_install_root() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return Ok(dirs::data_local_dir()
            .context("unable to resolve local data directory")?
            .join("Yggterm"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(dirs::data_local_dir()
            .context("unable to resolve local data directory")?
            .join("yggterm")
            .join("direct"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Ok(dirs::data_local_dir()
            .context("unable to resolve local data directory")?
            .join("yggterm")
            .join("direct"))
    }
}

pub fn write_direct_install_state(
    root: &Path,
    repo: &str,
    asset_label: &str,
    version: &str,
    executable: &Path,
) -> Result<()> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create install root {}", root.display()))?;
    let state = DirectInstallState {
        channel: InstallChannel::Direct,
        repo: repo.to_string(),
        asset_label: asset_label.to_string(),
        active_version: version.to_string(),
        active_executable: executable.to_path_buf(),
        icon_revision: version.to_string(),
    };
    let encoded = serde_json::to_vec_pretty(&state).context("failed to serialize install state")?;
    fs::write(root.join(INSTALL_STATE_FILENAME), encoded)
        .with_context(|| format!("failed to write install state under {}", root.display()))?;
    Ok(())
}

pub fn refresh_desktop_integration(context: &InstallContext) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    if context.channel != InstallChannel::Direct {
        return Ok(notes);
    }

    #[cfg(target_os = "linux")]
    {
        notes.extend(refresh_linux_integration(context)?);
    }
    #[cfg(target_os = "macos")]
    {
        notes.extend(refresh_macos_integration(context)?);
    }
    #[cfg(target_os = "windows")]
    {
        notes.extend(refresh_windows_integration(context)?);
    }

    Ok(notes)
}

#[cfg(target_os = "linux")]
fn refresh_linux_integration(context: &InstallContext) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let data_dir = dirs::data_local_dir().context("unable to resolve local data dir")?;
    let applications_dir = data_dir.join("applications");
    let pixmaps_dir = data_dir.join("pixmaps");
    let icons_dir = data_dir
        .join("icons")
        .join("hicolor")
        .join("512x512")
        .join("apps");
    let scalable_icons_dir = data_dir
        .join("icons")
        .join("hicolor")
        .join("scalable")
        .join("apps");
    fs::create_dir_all(&applications_dir)?;
    fs::create_dir_all(&pixmaps_dir)?;
    fs::create_dir_all(&icons_dir)?;
    fs::create_dir_all(&scalable_icons_dir)?;

    let icon_path = icons_dir.join("yggterm.png");
    write_if_changed(
        &icon_path,
        include_bytes!("../../../assets/brand/yggterm-icon-512.png"),
    )?;
    let scalable_icon_path = scalable_icons_dir.join("yggterm.svg");
    write_if_changed(
        &scalable_icon_path,
        include_bytes!("../../../assets/brand/yggterm-icon.svg"),
    )?;
    let pixmaps_icon_path = pixmaps_dir.join("yggterm.png");
    write_if_changed(
        &pixmaps_icon_path,
        include_bytes!("../../../assets/brand/yggterm-icon-512.png"),
    )?;
    let desktop_path = applications_dir.join("dev.yggterm.Yggterm.desktop");
    let desktop_contents = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Yggterm\nComment=Remote-first terminal workspace\nExec={}\nTryExec={}\nIcon={}\nTerminal=false\nCategories=System;TerminalEmulator;Development;\nStartupNotify=true\nStartupWMClass=yggterm\nX-Desktop-File-Install-Version=0.27\n",
        desktop_exec_escape(&context.executable_path),
        desktop_exec_escape(&context.executable_path),
        "yggterm",
    );
    write_if_changed(&desktop_path, desktop_contents.as_bytes())?;
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&applications_dir)
        .status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("-f")
        .arg("-t")
        .arg(data_dir.join("icons").join("hicolor"))
        .status();
    let _ = std::process::Command::new("xdg-icon-resource")
        .arg("forceupdate")
        .status();
    let _ = std::process::Command::new("xdg-desktop-menu")
        .arg("forceupdate")
        .status();
    let _ = std::process::Command::new("kbuildsycoca6").status();
    let _ = std::process::Command::new("kbuildsycoca5").status();

    if let Some(bin_dir) = linux_user_bin_dir() {
        fs::create_dir_all(&bin_dir)?;
        let shim = bin_dir.join("yggterm");
        create_or_replace_symlink(&context.executable_path, &shim)?;
        notes.push(format!(
            "desktop entry refreshed at {}",
            desktop_path.display()
        ));
    }

    Ok(notes)
}

#[cfg(target_os = "macos")]
fn refresh_macos_integration(context: &InstallContext) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let home_dir = dirs::home_dir().context("unable to resolve home directory")?;
    let app_dir = home_dir.join("Applications").join("Yggterm.app");
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");
    fs::create_dir_all(&macos_dir)?;
    fs::create_dir_all(&resources_dir)?;

    let launcher = macos_dir.join("yggterm");
    let script = format!(
        "#!/bin/sh\nexec \"{}\" \"$@\"\n",
        context.executable_path.display()
    );
    write_if_changed(&launcher, script.as_bytes())?;
    set_unix_executable(&launcher)?;
    write_if_changed(
        &contents_dir.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleName</key><string>Yggterm</string><key>CFBundleDisplayName</key><string>Yggterm</string><key>CFBundleIdentifier</key><string>dev.yggterm.Yggterm</string><key>CFBundleExecutable</key><string>yggterm</string><key>CFBundlePackageType</key><string>APPL</string><key>LSBackgroundOnly</key><false/></dict></plist>\n"
        )
        .as_bytes(),
    )?;
    write_if_changed(
        &resources_dir.join("yggterm.png"),
        include_bytes!("../../../assets/brand/yggterm-icon-512.png"),
    )?;
    notes.push(format!("app bundle refreshed at {}", app_dir.display()));
    Ok(notes)
}

#[cfg(target_os = "windows")]
fn refresh_windows_integration(context: &InstallContext) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let roaming = dirs::data_dir().context("unable to resolve roaming data dir")?;
    let shortcut_dir = roaming
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Yggterm");
    fs::create_dir_all(&shortcut_dir)?;
    let shortcut_path = shortcut_dir.join("Yggterm.lnk");
    let ps = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $sc = $ws.CreateShortcut('{}'); \
         $sc.TargetPath = '{}'; \
         $sc.WorkingDirectory = '{}'; \
         $sc.IconLocation = '{}'; \
         $sc.Save();",
        powershell_escape(shortcut_path.as_os_str().to_string_lossy().as_ref()),
        powershell_escape(
            context
                .executable_path
                .as_os_str()
                .to_string_lossy()
                .as_ref()
        ),
        powershell_escape(
            context
                .executable_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .as_os_str()
                .to_string_lossy()
                .as_ref()
        ),
        powershell_escape(
            context
                .executable_path
                .as_os_str()
                .to_string_lossy()
                .as_ref()
        ),
    );
    let status = std::process::Command::new("powershell")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(ps)
        .status()
        .context("failed to launch powershell for shortcut creation")?;
    if !status.success() {
        anyhow::bail!("failed to create Windows shortcut");
    }
    notes.push(format!(
        "Start Menu shortcut refreshed at {}",
        shortcut_path.display()
    ));
    Ok(notes)
}

pub fn check_for_update(context: &InstallContext) -> Result<Option<ReleaseUpdate>> {
    let client = release_client()?;
    let latest_url = format!("https://github.com/{}/releases/latest", context.repo);
    let response = client
        .get(&latest_url)
        .send()
        .context("failed to query latest GitHub release")?
        .error_for_status()
        .context("failed to read latest GitHub release redirect")?;
    let final_url = response.url().clone();
    let tag_name = final_url
        .path_segments()
        .and_then(|segments| segments.last())
        .filter(|segment| segment.starts_with('v'))
        .context("failed to resolve latest release tag from redirect")?
        .to_string();
    let latest_version = tag_name.trim_start_matches('v').to_string();
    if !is_newer_version(&latest_version, &context.current_version)? {
        return Ok(None);
    }

    let archive_name = format!("yggterm-{}.tar.gz", context.asset_label);
    let checksum_name = format!("{archive_name}.sha256");
    let archive_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        context.repo, tag_name, archive_name
    );
    let checksum_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        context.repo, tag_name, checksum_name
    );

    Ok(Some(ReleaseUpdate {
        version: latest_version,
        tag_name,
        archive_url,
        checksum_url: Some(checksum_url),
    }))
}

pub fn install_release_update(context: &InstallContext, update: &ReleaseUpdate) -> Result<PathBuf> {
    if context.channel != InstallChannel::Direct {
        anyhow::bail!("self-update is only available for direct installs");
    }
    let root = context
        .managed_root
        .as_ref()
        .context("missing direct install root")?;
    let versions_dir = root.join("versions");
    let version_dir = versions_dir.join(&update.version);
    fs::create_dir_all(&version_dir)
        .with_context(|| format!("failed to create version dir {}", version_dir.display()))?;

    let archive = release_client()?
        .get(&update.archive_url)
        .send()
        .context("failed to download release archive")?
        .error_for_status()
        .context("failed to fetch release archive")?
        .bytes()
        .context("failed to read release archive bytes")?;

    if let Some(checksum_url) = &update.checksum_url {
        verify_archive_checksum(&archive, checksum_url)?;
    }

    let binary_name = if cfg!(target_os = "windows") {
        format!("yggterm-{}.exe", context.asset_label)
    } else {
        format!("yggterm-{}", context.asset_label)
    };
    let headless_name = if cfg!(target_os = "windows") {
        format!("yggterm-headless-{}.exe", context.asset_label)
    } else {
        format!("yggterm-headless-{}", context.asset_label)
    };
    let binary_path = version_dir.join(if cfg!(target_os = "windows") {
        "yggterm.exe"
    } else {
        "yggterm"
    });
    let headless_path = version_dir.join(if cfg!(target_os = "windows") {
        "yggterm-headless.exe"
    } else {
        "yggterm-headless"
    });
    extract_binary_from_archive(&archive, &binary_name, &binary_path)?;
    extract_binary_from_archive(&archive, &headless_name, &headless_path)?;
    write_direct_install_state(
        root,
        &context.repo,
        &context.asset_label,
        &update.version,
        &binary_path,
    )?;

    let updated_context = InstallContext {
        channel: InstallChannel::Direct,
        update_policy: UpdatePolicy::Auto,
        repo: context.repo.clone(),
        asset_label: context.asset_label.clone(),
        current_version: update.version.clone(),
        executable_path: binary_path.clone(),
        managed_root: Some(root.clone()),
        manager_hint: context.manager_hint.clone(),
    };
    let _ = refresh_desktop_integration(&updated_context);
    Ok(binary_path)
}

pub fn install_mode_summary(context: &InstallContext) -> String {
    match context.update_policy {
        UpdatePolicy::Auto => format!("Direct install · updates automatically on launch"),
        UpdatePolicy::NotifyOnly => context
            .manager_hint
            .clone()
            .unwrap_or_else(|| "Notify only".to_string()),
    }
}

pub fn update_command_hint(channel: InstallChannel) -> &'static str {
    match channel {
        InstallChannel::Homebrew => "brew upgrade yggterm",
        InstallChannel::Winget => "winget upgrade yggterm",
        InstallChannel::Scoop => "scoop update yggterm",
        InstallChannel::Flatpak => "flatpak update",
        InstallChannel::Snap => "snap refresh yggterm",
        InstallChannel::Deb => "sudo apt upgrade yggterm",
        InstallChannel::Direct | InstallChannel::Unknown => "",
    }
}

fn is_newer_version(latest: &str, current: &str) -> Result<bool> {
    let latest = semver::Version::parse(latest)
        .with_context(|| format!("invalid latest version {latest}"))?;
    let current = semver::Version::parse(current)
        .with_context(|| format!("invalid current version {current}"))?;
    Ok(latest > current)
}

fn release_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent(format!("yggterm/{}", current_version()))
        .build()
        .context("failed to construct release client")
}

fn verify_archive_checksum(archive: &[u8], checksum_url: &str) -> Result<()> {
    let checksum_text = release_client()?
        .get(checksum_url)
        .send()
        .context("failed to download archive checksum")?
        .error_for_status()
        .context("failed to fetch archive checksum")?
        .text()
        .context("failed to read archive checksum")?;
    let expected = checksum_text
        .split_whitespace()
        .next()
        .context("missing checksum value")?;
    let actual = format!("{:x}", Sha256::digest(archive));
    if expected != actual {
        anyhow::bail!("release checksum mismatch");
    }
    Ok(())
}

fn extract_binary_from_archive(
    archive_bytes: &[u8],
    entry_name: &str,
    out_path: &Path,
) -> Result<()> {
    let cursor = Cursor::new(archive_bytes);
    let decoder = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);
    let mut found = false;
    for entry in archive
        .entries()
        .context("failed to iterate release archive")?
    {
        let mut entry = entry.context("failed to read release archive entry")?;
        let path = entry.path().context("failed to read archive entry path")?;
        if path.as_ref() == Path::new(entry_name) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .context("failed to extract archive entry")?;
            fs::write(out_path, bytes)
                .with_context(|| format!("failed to write {}", out_path.display()))?;
            #[cfg(unix)]
            set_unix_executable(out_path)?;
            found = true;
            break;
        }
    }
    if !found {
        anyhow::bail!("failed to locate {entry_name} in release archive");
    }
    Ok(())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path)
        && existing == bytes
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_unix_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_unix_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn desktop_exec_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
}

#[cfg(target_os = "linux")]
fn linux_user_bin_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("bin"))
}

#[cfg(target_os = "linux")]
fn create_or_replace_symlink(target: &Path, link: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;
    if let Ok(existing_target) = fs::read_link(link)
        && existing_target == target
    {
        return Ok(());
    }
    let _ = fs::remove_file(link);
    symlink(target, link).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            link.display(),
            target.display()
        )
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn powershell_escape(input: &str) -> String {
    input.replace('\'', "''")
}
