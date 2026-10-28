//! The versioned GitHub-release package contract (schema 1).
//!
//! One release manifest, published as the `release-manifest.json` asset of a
//! GitHub release, describes every installable package that release ships:
//! a package identity, its porcelain role, and per-platform artifacts with
//! MANDATORY sha256 digests. The contract is deliberately generic so the
//! CLI seed and optional porcelains (desktop/web/mobile) share one schema
//! while keeping separate identities and ownership.
//!
//! Nothing in this module touches the network or the disk beyond parsing:
//! validation and archive-safety guards are pure so the malicious-input
//! tests can run offline. The transaction that consumes a validated
//! manifest lives in `github_release.rs`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

/// The manifest schema this ynpm understands. A release carrying a newer
/// schema is refused loudly rather than guessed at.
pub const RELEASE_MANIFEST_SCHEMA: u32 = 1;

/// The asset name a release must publish its manifest under.
pub const RELEASE_MANIFEST_ASSET: &str = "release-manifest.json";

/// Extraction guards are caps, not suggestions: an archive that would
/// outgrow them is refused BEFORE extraction, not mopped up afterwards.
pub const MAX_ARCHIVE_ENTRIES: usize = 100_000;
pub const MAX_DECOMPRESSED_TOTAL_BYTES: u64 = 1 << 30; // 1 GiB extracted
pub const MAX_DECOMPRESSED_FILE_BYTES: u64 = 1 << 29; // 512 MiB single entry
/// A compressed porcelain archive larger than this is implausible and is
/// refused even when its listing cannot be size-accounted.
pub const MAX_COMPRESSED_ARCHIVE_BYTES: u64 = 1 << 29; // 512 MiB compressed

/// The platform identifiers a manifest may name. A platform outside this
/// set is a manifest typo, not a supported target, and invalidates the
/// whole manifest: installing a mistyped platform silently would be worse
/// than refusing the release.
pub const KNOWN_RELEASE_PLATFORMS: [&str; 8] = [
    "linux-x64",
    "linux-arm64",
    "macos-x64",
    "macos-arm64",
    "windows-x64",
    "windows-arm64",
    "android-arm64",
    "ios-arm64",
];

/// The platform identifier THIS host maps to, or the honest refusal naming
/// what exists. Distinct from the npm `platform_target` (which covers only
/// the linux @ygghq binary matrix): release packages may ship any platform
/// in `KNOWN_RELEASE_PLATFORMS`, and the host mapping must not pretend to
/// be npm's.
pub fn release_platform(os: &str, arch: &str) -> Result<String> {
    let mapped = match (os, arch) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        _ => "",
    };
    if mapped.is_empty() {
        bail!(
            "no release-package platform maps this host ({os}/{arch}); \
             release manifests ship {KNOWN_RELEASE_PLATFORMS:?}"
        );
    }
    Ok(mapped.to_string())
}

pub fn release_platform_for_host() -> Result<String> {
    release_platform(std::env::consts::OS, std::env::consts::ARCH)
}

// ===== schema types =====

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub release: ReleaseIdentity,
    #[serde(default)]
    pub packages: Vec<ReleasePackage>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseIdentity {
    pub version: String,
    pub tag: String,
}

/// What a package IS for, not what it happens to contain. Roles drive the
/// install policy: `seed` and `desktop` install through the generation
/// transaction, `web` is validated but owned by the yggterm web plane, and
/// `mobile` is NEVER installed locally - it becomes an official-store
/// handoff. Nothing auto-launches from a manifest.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleasePackageRole {
    Seed,
    Desktop,
    Web,
    Mobile,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleasePackage {
    pub id: String,
    pub role: ReleasePackageRole,
    #[serde(default)]
    pub summary: Option<String>,
    pub artifacts: Vec<ReleaseArtifact>,
    /// Bundle-only desktop artifacts have no bin to interrogate; the probe
    /// names a file inside the extracted tree whose bytes must contain the
    /// release version. Required exactly when an artifact carries no bins.
    #[serde(default)]
    pub version_probe: Option<ReleaseVersionProbe>,
    /// How the desktop surface is entered after install: a relative path
    /// (from the extraction root) to the executable inside the bundle.
    /// Existence is verified; launching is the daemon/GUI plane's job.
    #[serde(default)]
    pub launch: Option<ReleaseLaunch>,
    /// Optional `yggterm.app` block (the `package.json.yggterm.app` shape)
    /// so a desktop package can register with the yggterm launcher. Only
    /// valid on packages that also declare bins: the app block names a bin.
    #[serde(default)]
    pub app: Option<serde_json::Value>,
    /// Official-package handoff for mobile artifacts. ynpm prints this and
    /// installs nothing; adb is never part of the path.
    #[serde(default)]
    pub handoff: Option<ReleaseHandoff>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum ReleaseArtifactFormat {
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "apk")]
    Apk,
    #[serde(rename = "ipa")]
    Ipa,
}

impl ReleaseArtifactFormat {
    pub fn label(&self) -> &'static str {
        match self {
            ReleaseArtifactFormat::TarGz => "tar.gz",
            ReleaseArtifactFormat::Zip => "zip",
            ReleaseArtifactFormat::Apk => "apk",
            ReleaseArtifactFormat::Ipa => "ipa",
        }
    }
    pub fn installable(&self) -> bool {
        matches!(
            self,
            ReleaseArtifactFormat::TarGz | ReleaseArtifactFormat::Zip
        )
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseArtifact {
    pub platform: String,
    pub format: ReleaseArtifactFormat,
    pub filename: String,
    /// Mandatory. A release artifact without an exact digest is refused at
    /// validation time - "we'll add checksums later" is how a supply-chain
    /// gap ships.
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Bin names the artifact must provide at the extraction root (or
    /// inside its single top-level directory). Each must answer
    /// `--version` with the release version before publication.
    #[serde(default)]
    pub bins: Vec<String>,
    /// Self-contained directory (e.g. `yggterm.app`, `yggterm-linux/`)
    /// published as one unit. Presence and the version probe are verified;
    /// no desktop integration is implied.
    #[serde(default)]
    pub bundle: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseVersionProbe {
    /// Extraction-root-relative file whose bytes must contain the release
    /// version string.
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseLaunch {
    /// Extraction-root-relative path to the executable a desktop plane
    /// would launch.
    pub entry: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ReleaseHandoff {
    /// Where the official package lives (store page or the release's own
    /// asset page). https only.
    pub url: String,
    #[serde(default)]
    pub installer_hint: Option<String>,
}

// ===== validation =====

fn safe_package_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('-')
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if !ok {
        bail!("package id {id:?} is invalid: want 1-64 chars of [a-z0-9._-], no leading '-'");
    }
    Ok(())
}

/// Release asset filenames are joined into scratch paths verbatim, so a
/// traversal-spelled filename (`../ynpm`, `a/b`, absolute, control chars)
/// must be refused at validation, before any path is built from it.
pub fn safe_artifact_filename(filename: &str) -> Result<()> {
    let ok = !filename.is_empty()
        && filename.len() <= 255
        && filename != "."
        && filename != ".."
        && !filename.contains('/')
        && !filename.contains('\\')
        && !filename.chars().any(|ch| ch.is_control() || ch.is_whitespace());
    if !ok {
        bail!("artifact filename {filename:?} is not a safe flat filename");
    }
    Ok(())
}

/// Extraction-root-relative content paths (version probe, launch entry,
/// bundle dir) must stay inside the tree: no absolute, no `..`, no
/// backslash, no empty or dot-only components.
pub fn safe_content_relpath(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') {
        bail!("content path {rel:?} must be a relative forward-slash path");
    }
    for component in rel.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            bail!("content path {rel:?} has an unsafe component {component:?}");
        }
    }
    Ok(())
}

fn safe_tag(tag: &str) -> Result<()> {
    let ok = !tag.is_empty()
        && tag.len() <= 128
        && tag
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+'));
    if !ok {
        bail!("release tag {tag:?} is not a safe git tag");
    }
    Ok(())
}

fn normalize_sha256(value: &str) -> Result<String> {
    let lowered = value.trim().to_ascii_lowercase();
    let ok = lowered.len() == 64 && lowered.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !ok {
        bail!("artifact digest {value:?} is not 64 hex characters");
    }
    Ok(lowered)
}

/// Validate one manifest document against the schema-1 contract. Every
/// refusal names the defect: a malformed manifest must never be "fixed up"
/// by the installer.
pub fn validate_release_manifest(doc: &serde_json::Value) -> Result<ReleaseManifest> {
    let manifest: ReleaseManifest = serde_json::from_value(doc.clone())
        .context("release manifest is not valid schema-1 JSON")?;
    if manifest.schema != RELEASE_MANIFEST_SCHEMA {
        bail!(
            "release manifest schema {} is unsupported (this ynpm understands schema {RELEASE_MANIFEST_SCHEMA})",
            manifest.schema
        );
    }
    crate::SemVer::parse(&manifest.release.version)
        .with_context(|| format!("release version {:?}", manifest.release.version))?;
    safe_tag(&manifest.release.tag)?;
    if manifest.packages.is_empty() {
        bail!("release manifest declares no packages");
    }
    let mut seen_ids = BTreeSet::new();
    for package in &manifest.packages {
        safe_package_id(&package.id)?;
        if !seen_ids.insert(package.id.clone()) {
            bail!("duplicate package id {:?} in release manifest", package.id);
        }
        validate_release_package(package)?;
    }
    Ok(manifest)
}

fn validate_release_package(package: &ReleasePackage) -> Result<()> {
    if package.artifacts.is_empty() {
        bail!("package {:?} declares no artifacts", package.id);
    }
    let mut seen_platforms = BTreeSet::new();
    for artifact in &package.artifacts {
        if !KNOWN_RELEASE_PLATFORMS.contains(&artifact.platform.as_str()) {
            bail!(
                "package {:?} names platform {:?}, which is not a known release platform",
                package.id,
                artifact.platform
            );
        }
        if !seen_platforms.insert(artifact.platform.clone()) {
            bail!(
                "package {:?} declares platform {:?} twice",
                package.id,
                artifact.platform
            );
        }
        safe_artifact_filename(&artifact.filename)?;
        normalize_sha256(&artifact.sha256)?;
        if let Some(size) = artifact.size {
            if size == 0 {
                bail!("package {:?} artifact {:?} declares size 0", package.id, artifact.platform);
            }
        }
        let mobile_only_format = matches!(
            artifact.format,
            ReleaseArtifactFormat::Apk | ReleaseArtifactFormat::Ipa
        );
        match package.role {
            ReleasePackageRole::Mobile => {
                if !mobile_only_format {
                    bail!(
                        "mobile package {:?} artifact {:?} must be an apk/ipa, not {}",
                        package.id,
                        artifact.platform,
                        artifact.format.label()
                    );
                }
            }
            _ => {
                if mobile_only_format {
                    bail!(
                        "package {:?} artifact {:?} uses mobile format {} but the package role is not mobile",
                        package.id,
                        artifact.platform,
                        artifact.format.label()
                    );
                }
                if !artifact.format.installable() {
                    bail!(
                        "package {:?} artifact {:?} format {} is not installable",
                        package.id,
                        artifact.platform,
                        artifact.format.label()
                    );
                }
            }
        }
        for bin in &artifact.bins {
            safe_artifact_filename(bin)
                .with_context(|| format!("package {:?} bin name", package.id))?;
        }
        if let Some(bundle) = &artifact.bundle {
            safe_content_relpath(bundle)
                .with_context(|| format!("package {:?} bundle path", package.id))?;
        }
        // Version verification is mandatory for roles ynpm installs: a
        // bin-gated artifact or a probed bundle, never nothing. Web and
        // mobile artifacts are validated for shape here but their
        // verification belongs to the plane that delivers them.
        let ynpm_installed = matches!(
            package.role,
            ReleasePackageRole::Seed | ReleasePackageRole::Desktop
        );
        if artifact.bins.is_empty() {
            if ynpm_installed && package.version_probe.is_none() {
                bail!(
                    "package {:?} artifact {:?} has no bins and the package declares no version_probe; \
                     a release with no version verification is refused",
                    package.id,
                    artifact.platform
                );
            }
        } else if package.role == ReleasePackageRole::Mobile {
            bail!(
                "mobile package {:?} must not declare bins",
                package.id
            );
        }
    }
    if let Some(probe) = &package.version_probe {
        safe_content_relpath(&probe.file)
            .with_context(|| format!("package {:?} version_probe.file", package.id))?;
    }
    if let Some(launch) = &package.launch {
        safe_content_relpath(&launch.entry)
            .with_context(|| format!("package {:?} launch.entry", package.id))?;
    }
    if let Some(handoff) = &package.handoff {
        if !handoff.url.starts_with("https://") {
            bail!("package {:?} handoff.url must be https", package.id);
        }
    }
    match package.role {
        ReleasePackageRole::Mobile => {
            if package.handoff.is_none() {
                bail!(
                    "mobile package {:?} must declare a handoff (official package/store URL); \
                     ynpm never installs mobile artifacts",
                    package.id
                );
            }
        }
        ReleasePackageRole::Desktop => {
            if package.launch.is_none() && package.artifacts.iter().all(|a| a.bins.is_empty()) {
                bail!(
                    "desktop package {:?} declares neither bins nor a launch entry; \
                     a desktop surface must say how it is entered",
                    package.id
                );
            }
        }
        ReleasePackageRole::Seed => {
            if package.artifacts.iter().any(|a| a.bins.is_empty()) {
                bail!(
                    "seed package {:?} artifact has no bins; the seed's bins are its whole contract",
                    package.id
                );
            }
        }
        ReleasePackageRole::Web => {
            if package.launch.is_some() {
                bail!(
                    "web package {:?} must not declare a desktop launch entry",
                    package.id
                );
            }
        }
    }
    if package.app.is_some() {
        let has_bins = package.artifacts.iter().any(|a| !a.bins.is_empty());
        if !has_bins {
            bail!(
                "package {:?} carries an app block but no bins; the app block names a bin key",
                package.id
            );
        }
    }
    Ok(())
}

// ===== archive listing + guards =====

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveEntryKind {
    File,
    Dir,
    Symlink(String),
    /// Hard links and anything exotic: never installed.
    Other(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub size: Option<u64>,
    pub kind: ArchiveEntryKind,
}

pub fn all_digits(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
}

fn looks_like_iso_date(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && token
            .chars()
            .enumerate()
            .all(|(index, ch)| ch.is_ascii_digit() || matches!(index, 4 | 7))
}

/// Parse one `tar -tvzf` listing. Both GNU tar (fleet substrate:
/// `perms owner/group size YYYY-MM-DD HH:MM name`) and bsdtar (macOS:
/// `perms nlink owner group size Mon DD time name`) shapes are accepted;
/// ANY unparseable line refuses the archive - a listing we cannot read is
/// a listing we cannot guard.
pub fn parse_tar_listing_line(line: &str) -> Result<ArchiveEntry> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    // GNU shape
    if tokens.len() >= 6
        && tokens[1].contains('/')
        && all_digits(tokens[2])
        && looks_like_iso_date(tokens[3])
    {
        let name = tokens[5..].join(" ");
        return entry_from_listing_name(&name, tokens[2].parse::<u64>().ok());
    }
    // bsdtar shape (macOS): `perms nlink owner group size Mon DD time name`.
    // GNU lines cannot reach here: their owner/group token carries a '/' and
    // is checked first.
    if tokens.len() >= 9 && all_digits(tokens[1]) && all_digits(tokens[4]) {
        let name = tokens[8..].join(" ");
        return entry_from_listing_name(&name, tokens[4].parse::<u64>().ok());
    }
    bail!("archive listing line does not match a known tar -tv format");
}

fn entry_from_listing_name(listing_name: &str, size: Option<u64>) -> Result<ArchiveEntry> {
    if let Some((name, target)) = listing_name.split_once(" -> ") {
        return Ok(ArchiveEntry {
            name: name.trim_end_matches('/').to_string(),
            size,
            kind: ArchiveEntryKind::Symlink(target.to_string()),
        });
    }
    // GNU -tv spells hard links as "<name> link to <target>".
    if let Some(position) = listing_name.find(" link to ") {
        let name = listing_name[..position].trim_end_matches('/').to_string();
        return Ok(ArchiveEntry {
            name,
            size,
            kind: ArchiveEntryKind::Other("hardlink"),
        });
    }
    if listing_name.ends_with('/') {
        return Ok(ArchiveEntry {
            name: listing_name.trim_end_matches('/').to_string(),
            size,
            kind: ArchiveEntryKind::Dir,
        });
    }
    Ok(ArchiveEntry {
        name: listing_name.to_string(),
        size,
        kind: ArchiveEntryKind::File,
    })
}

/// Lexically resolve whether a symlink target stays inside the archive
/// root: no absolute targets, and no `..` walk that escapes the link's own
/// directory. Lexical only - a link is refused or kept as spelled, never
/// resolved against the live filesystem.
pub fn symlink_target_is_contained(entry_name: &str, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains('\\') {
        return false;
    }
    let mut depth: i64 = entry_name.rsplit('/').count() as i64; // components above the link's name
    // entry "a/b/link" -> rsplit count 3, but depth above "a/b" is 2; the
    // name component itself contributes no depth.
    depth -= 1;
    if depth < 0 {
        depth = 0;
    }
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// Refuse an entry name that would escape or confuse extraction: absolute
/// paths, `..` components, backslashes, control characters, Windows drive
/// spelling, and empty names.
pub fn safe_archive_entry_name(raw_name: &str) -> Result<()> {
    let name = raw_name.trim_end_matches('/');
    if name.is_empty() {
        bail!("archive contains an entry with an empty name");
    }
    if name.starts_with('/') {
        bail!("archive entry {name:?} is an absolute path");
    }
    if name.contains('\\') {
        bail!("archive entry {name:?} contains a backslash");
    }
    // Windows drive spelling "C:/..." - refused for portability of the
    // guard across extractors.
    if name.len() >= 2
        && name.as_bytes()[1] == b':'
        && name.as_bytes()[0].is_ascii_alphabetic()
    {
        bail!("archive entry {name:?} names a Windows drive path");
    }
    for component in name.split('/') {
        if component == ".." {
            bail!("archive entry {name:?} contains a '..' traversal component");
        }
    }
    if raw_name.chars().any(|ch| ch.is_control()) {
        bail!("archive entry {name:?} contains control characters");
    }
    Ok(())
}

/// Apply every structural guard to a parsed listing: entry budget, size
/// budget (per file and total), name safety, and link containment. Any
/// exotic entry kind is a refusal - the release pipeline ships plain
/// files, directories and (inside bundles) relative symlinks only.
pub fn guard_archive_entries(entries: &[ArchiveEntry]) -> Result<()> {
    if entries.is_empty() {
        bail!("archive listing has no entries; refusing an unreadable archive");
    }
    if entries.len() > MAX_ARCHIVE_ENTRIES {
        bail!(
            "archive declares {} entries, above the {MAX_ARCHIVE_ENTRIES} guard",
            entries.len()
        );
    }
    let mut total: u64 = 0;
    for entry in entries {
        safe_archive_entry_name(&entry.name)?;
        match &entry.kind {
            ArchiveEntryKind::File => {
                if let Some(size) = entry.size {
                    if size > MAX_DECOMPRESSED_FILE_BYTES {
                        bail!(
                            "archive entry {:?} is {} bytes, above the {MAX_DECOMPRESSED_FILE_BYTES} per-file guard",
                            entry.name,
                            size
                        );
                    }
                    total += size;
                }
            }
            ArchiveEntryKind::Dir => {}
            ArchiveEntryKind::Symlink(target) => {
                if !symlink_target_is_contained(&entry.name, target) {
                    bail!(
                        "archive symlink {:?} -> {:?} escapes the extraction root",
                        entry.name,
                        target
                    );
                }
            }
            ArchiveEntryKind::Other(kind) => {
                bail!(
                    "archive entry {:?} is a {kind}; release archives carry plain files, directories and relative symlinks only",
                    entry.name
                );
            }
        }
    }
    if total > MAX_DECOMPRESSED_TOTAL_BYTES {
        bail!(
            "archive declares {} decompressed bytes, above the {MAX_DECOMPRESSED_TOTAL_BYTES} guard",
            total
        );
    }
    Ok(())
}

/// Post-extraction sweep for zip-shaped archives whose listing cannot name
/// symlinks: any link that appeared on disk is a refusal.
pub fn tree_has_symlinks(root: &Path) -> Result<bool> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("statting {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Ok(true);
            }
            if metadata.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_platform_maps_and_refuses_honestly() {
        assert_eq!(release_platform("linux", "x86_64").unwrap(), "linux-x64");
        assert_eq!(release_platform("macos", "aarch64").unwrap(), "macos-arm64");
        assert_eq!(release_platform("windows", "x86_64").unwrap(), "windows-x64");
        let error = release_platform("sunos", "sparc").unwrap_err();
        assert!(error.to_string().contains("sunos/sparc"));
    }

    fn manifest_value(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    const VALID_DESKTOP_MANIFEST: &str = r#"{
      "schema": 1,
      "release": { "version": "4.0.0", "tag": "v4.0.0" },
      "packages": [
        {
          "id": "example-gui",
          "role": "desktop",
          "summary": "invented example desktop porcelain",
          "artifacts": [
            {
              "platform": "linux-x64",
              "format": "tar.gz",
              "filename": "example-gui-4.0.0-linux-x86_64.tar.gz",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 1024,
              "bundle": "example-gui-linux",
              "bins": ["example-gui"],
              "launch": { "entry": "example-gui-linux/example-gui" }
            }
          ],
          "launch": { "entry": "example-gui-linux/example-gui" }
        }
      ]
    }"#;

    #[test]
    fn valid_desktop_manifest_passes() {
        let manifest = validate_release_manifest(&manifest_value(VALID_DESKTOP_MANIFEST)).unwrap();
        assert_eq!(manifest.release.version, "4.0.0");
        assert_eq!(manifest.packages[0].role, ReleasePackageRole::Desktop);
        assert_eq!(manifest.packages[0].artifacts[0].format.label(), "tar.gz");
    }

    #[test]
    fn wrong_schema_version_is_refused_loudly() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["schema"] = serde_json::json!(2);
        let error = validate_release_manifest(&doc).unwrap_err();
        assert!(error.to_string().contains("schema 2 is unsupported"));
    }

    #[test]
    fn missing_sha256_is_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["packages"][0]["artifacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        assert!(validate_release_manifest(&doc).is_err());
    }

    #[test]
    fn malformed_sha256_is_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["packages"][0]["artifacts"][0]["sha256"] = serde_json::json!("deadbeef");
        assert!(validate_release_manifest(&doc).is_err());
    }

    #[test]
    fn traversal_filename_is_refused() {
        for evil in ["../evil.tar.gz", "/abs/evil.tar.gz", "a\\b.tar.gz", "evil tar.gz"] {
            let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
            doc["packages"][0]["artifacts"][0]["filename"] = serde_json::json!(evil);
            let error = validate_release_manifest(&doc).unwrap_err();
            assert!(
                error.to_string().contains("safe flat filename"),
                "filename {evil:?} was not refused: {error:#}"
            );
        }
    }

    #[test]
    fn duplicate_platform_and_duplicate_id_are_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        let clone = doc["packages"][0]["artifacts"][0].clone();
        doc["packages"][0]["artifacts"].as_array_mut().unwrap().push(clone);
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("twice"));

        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        let clone = doc["packages"][0].clone();
        doc["packages"].as_array_mut().unwrap().push(clone);
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("duplicate package id"));
    }

    #[test]
    fn unknown_platform_is_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["packages"][0]["artifacts"][0]["platform"] = serde_json::json!("linux-x8664");
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("not a known release platform"));
    }

    #[test]
    fn desktop_without_version_verification_is_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["packages"][0]["artifacts"][0]["bins"] = serde_json::json!([]);
        let error = validate_release_manifest(&doc).unwrap_err();
        assert!(error.to_string().contains("version_probe"));
    }

    #[test]
    fn mobile_requires_https_handoff_and_refuses_bins() {
        let mobile = r#"{
          "schema": 1,
          "release": { "version": "4.0.0", "tag": "v4.0.0" },
          "packages": [
            {
              "id": "example-mobile",
              "role": "mobile",
              "artifacts": [
                {
                  "platform": "android-arm64",
                  "format": "apk",
                  "filename": "example-4.0.0.apk",
                  "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }
              ],
              "handoff": { "url": "https://example.test/releases/v4.0.0" }
            }
          ]
        }"#;
        let manifest = validate_release_manifest(&manifest_value(mobile)).unwrap();
        assert_eq!(manifest.packages[0].role, ReleasePackageRole::Mobile);

        let mut doc = manifest_value(mobile);
        doc["packages"][0].as_object_mut().unwrap().remove("handoff");
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("handoff"));

        let mut doc = manifest_value(mobile);
        doc["packages"][0]["handoff"]["url"] = serde_json::json!("http://example.test/apk");
        assert!(validate_release_manifest(&doc).is_err());

        let mut doc = manifest_value(mobile);
        doc["packages"][0]["artifacts"][0]["bins"] = serde_json::json!(["example"]);
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("must not declare bins"));
    }

    #[test]
    fn seed_requires_bins_on_every_artifact() {
        let seed = r#"{
          "schema": 1,
          "release": { "version": "4.0.0", "tag": "v4.0.0" },
          "packages": [
            {
              "id": "example-seed",
              "role": "seed",
              "artifacts": [
                {
                  "platform": "linux-x64",
                  "format": "tar.gz",
                  "filename": "example-seed-4.0.0-linux-x86_64.tar.gz",
                  "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                  "bins": ["ygg", "ynpm", "ynpx"]
                }
              ]
            }
          ]
        }"#;
        assert!(validate_release_manifest(&manifest_value(seed)).is_ok());

        let mut doc = manifest_value(seed);
        doc["packages"][0]["artifacts"][0]["bins"] = serde_json::json!([]);
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("no bins"));
    }

    #[test]
    fn apk_format_on_non_mobile_role_is_refused() {
        let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
        doc["packages"][0]["artifacts"][0]["format"] = serde_json::json!("apk");
        assert!(validate_release_manifest(&doc)
            .unwrap_err()
            .to_string()
            .contains("not mobile"));
    }

    #[test]
    fn unsafe_content_paths_are_refused() {
        for evil in ["/abs/probe", "../probe", "a/../probe", "a\\probe"] {
            let mut doc = manifest_value(VALID_DESKTOP_MANIFEST);
            doc["packages"][0]["launch"]["entry"] = serde_json::json!(evil);
            assert!(
                validate_release_manifest(&doc).is_err(),
                "launch path {evil:?} was not refused"
            );
        }
    }

    #[test]
    fn tar_listing_parses_gnu_bsdtar_and_refuses_unknown() {
        let gnu = "-rw-r--r-- pi/pi 4096 2026-10-28 12:00 bin/ygg";
        let entry = parse_tar_listing_line(gnu).unwrap();
        assert_eq!(entry.name, "bin/ygg");
        assert_eq!(entry.size, Some(4096));
        assert_eq!(entry.kind, ArchiveEntryKind::File);

        let gnu_dir = "drwxr-xr-x pi/pi 0 2026-10-28 12:00 bin/";
        let entry = parse_tar_listing_line(gnu_dir).unwrap();
        assert_eq!(entry.kind, ArchiveEntryKind::Dir);

        let gnu_link = "lrwxrwxrwx pi/pi 0 2026-10-28 12:00 bundle/current -> bundle/1.0";
        let entry = parse_tar_listing_line(gnu_link).unwrap();
        assert_eq!(
            entry.kind,
            ArchiveEntryKind::Symlink("bundle/1.0".to_string())
        );

        let bsdtar = "-rw-r--r--  0 pi      staff    8192 Oct 28 12:00 Yggterm.app/Contents/MacOS/yggterm";
        let entry = parse_tar_listing_line(bsdtar).unwrap();
        assert_eq!(entry.name, "Yggterm.app/Contents/MacOS/yggterm");
        assert_eq!(entry.size, Some(8192));

        let bsdtar_link =
            "lrwxr-xr-x  0 pi staff 0 Oct 28 2024 Yggterm.app/Versions/Current -> A";
        let entry = parse_tar_listing_line(bsdtar_link).unwrap();
        assert_eq!(
            entry.kind,
            ArchiveEntryKind::Symlink("A".to_string())
        );

        assert!(parse_tar_listing_line("garbage line").is_err());
        assert!(parse_tar_listing_line("").is_err());
    }

    #[test]
    fn guards_refuse_traversal_absolute_and_oversize() {
        let entries = |names: &[(&str, Option<u64>)]| -> Vec<ArchiveEntry> {
            names
                .iter()
                .map(|(name, size)| ArchiveEntry {
                    name: name.to_string(),
                    size: *size,
                    kind: ArchiveEntryKind::File,
                })
                .collect()
        };
        assert!(guard_archive_entries(&entries(&[("../evil", Some(1))])).is_err());
        assert!(guard_archive_entries(&entries(&[("/etc/passwd", Some(1))])).is_err());
        assert!(guard_archive_entries(&entries(&[("a\\b", Some(1))])).is_err());
        assert!(guard_archive_entries(&entries(&[(
            "bomb",
            Some(MAX_DECOMPRESSED_FILE_BYTES + 1)
        )]))
        .is_err());
        let mut many: Vec<ArchiveEntry> = (0..MAX_ARCHIVE_ENTRIES + 1)
            .map(|index| ArchiveEntry {
                name: format!("f{index}"),
                size: Some(1),
                kind: ArchiveEntryKind::File,
            })
            .collect();
        many.truncate(MAX_ARCHIVE_ENTRIES + 1);
        assert!(guard_archive_entries(&many).is_err());
        let ok = entries(&[("bin/ygg", Some(12)), ("share/docs/a.md", Some(3))]);
        guard_archive_entries(&ok).unwrap();
        assert!(guard_archive_entries(&[]).is_err());
    }

    #[test]
    fn symlink_containment_is_lexical() {
        assert!(symlink_target_is_contained("app/current", "1.0"));
        assert!(symlink_target_is_contained("app/current", "sub/1.0"));
        assert!(symlink_target_is_contained("app/sub/current", "../Versions/A"));
        assert!(!symlink_target_is_contained("app/current", "/etc"));
        assert!(!symlink_target_is_contained("app/current", "../../etc"));
        assert!(!symlink_target_is_contained("app/current", "..\\windows"));
        assert!(!symlink_target_is_contained("app/current", ""));
    }

    #[test]
    fn hardlinks_and_exotic_entries_are_refused() {
        let hardlink = parse_tar_listing_line("-rw-r--r-- pi/pi 12 2026-10-28 12:00 dup link to bin/ygg")
            .unwrap();
        assert_eq!(hardlink.kind, ArchiveEntryKind::Other("hardlink"));
        assert!(guard_archive_entries(&[hardlink]).is_err());
    }
}
