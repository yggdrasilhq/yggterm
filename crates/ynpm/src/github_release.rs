//! The GitHub-release install transaction for release-manifest packages.
//!
//! Flow: `ygg ynpm install https://github.com/<owner>/<repo>` resolves the
//! release (latest tag redirect or an explicit tag), fetches the
//! `release-manifest.json` asset, validates it against the schema-1
//! contract, selects the package/artifact for this host, verifies the
//! archive's exact sha256, extracts behind the safety guards, gates every
//! declared bin's `--version`, and only then stages and publishes a new
//! generation. Any failure before publication leaves the previous
//! generation and its links exactly as they were.
//!
//! The network half (resolve + download) is deliberately separate from the
//! offline half (`install_from_release_manifest` and below) so the
//! transaction - selection, verification, staging, publication, rollback
//! behaviour - is testable against local fixtures without a network.

use crate::release_contract::*;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{
    copy_dir_all, extract_tarball, package_integration, package_storage_key, publish_link,
    run_version, set_executable, sha256_file, sync_app_registration, trace_safe_text,
    version_answer_matches, ynpm_trace, InstallOutcome, Paths,
};

/// `https://github.com/<owner>/<repo>[@tag]` and `github:<owner>/<repo>[@tag]`
/// are the same source; a repo path deeper than owner/repo (`.../tree/main`,
/// `.../releases`) is refused with the correction, never followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubReleaseSource {
    pub repo: String,
    pub tag: Option<String>,
}

pub fn looks_like_github_release_source(spec: &str) -> bool {
    spec.starts_with("https://github.com/") || spec.starts_with("github:")
}

/// Owner and repository name components are joined into download URLs and
/// persisted into state, so each must be a strict slug: plaintext HTTP is a
/// downgrade vector and anything outside `[A-Za-z0-9._-]` (or a `..`
/// traversal) must never reach a URL.
fn safe_repo_slug(slug: &str) -> Result<()> {
    let ok = !slug.is_empty()
        && slug.len() <= 100
        && slug != "."
        && slug != ".."
        && slug
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if !ok {
        bail!("github repository component {slug:?} is not a safe owner/name slug");
    }
    Ok(())
}

pub fn parse_github_release_source(spec: &str) -> Result<GitHubReleaseSource> {
    let raw = spec
        .strip_prefix("https://github.com/")
        .or_else(|| spec.strip_prefix("github:"))
        .context(
            "not a github release source; use https://github.com/<owner>/<repo> \
             or github:<owner>/<repo> (plaintext http is refused)",
        )?;
    // An explicit tag rides a trailing @tag on either spelling. Splitting
    // BEFORE path checks keeps `.../tree/main@v4` from looking like a tag.
    let (path, tag) = match raw.rsplit_once('@') {
        Some((base, tag)) if !base.is_empty() && !tag.is_empty() => {
            (base, Some(tag.to_string()))
        }
        _ => (raw, None),
    };
    if let Some(tag) = &tag {
        safe_tag(tag).with_context(|| format!("release tag {tag:?}"))?;
    }
    let repo = path.trim_end_matches('/').trim_end_matches(".git");
    let mut pieces = repo.split('/');
    let owner = pieces
        .next()
        .filter(|part| !part.is_empty())
        .context("github release source has no owner")?;
    let name = pieces
        .next()
        .filter(|part| !part.is_empty())
        .context("github release source has no repository")?;
    if pieces.next().is_some() {
        bail!(
            "github release source must name a repository root (owner/repo), \
             not a subpath - pass https://github.com/{owner}/{name}"
        );
    }
    if owner == "." || name == "." {
        bail!("github release source has an empty path segment");
    }
    safe_repo_slug(owner)?;
    safe_repo_slug(name)?;
    Ok(GitHubReleaseSource {
        repo: format!("{owner}/{name}"),
        tag,
    })
}

fn release_download_url(repo: &str, tag: &str, filename: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{filename}")
}

/// The latest release tag, read the one honest way GitHub offers a plain
/// HTTPS client: the `releases/latest` redirect's final URL. No API token,
/// no rate-limit dance.
pub fn resolve_latest_tag(repo: &str) -> Result<String> {
    let latest_url = format!("https://github.com/{repo}/releases/latest");
    let null_device = if cfg!(target_os = "windows") {
        "NUL"
    } else {
        "/dev/null"
    };
    let output = Command::new("curl")
        .args(["-fsSL", "-o", null_device, "-w", "%{url_effective}"])
        .arg(&latest_url)
        .output()
        .context("resolving the latest github release (is curl installed?)")?;
    if !output.status.success() {
        bail!(
            "curl {latest_url} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let final_url = String::from_utf8_lossy(&output.stdout);
    let tag = final_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(|value| value.split(['?', '#']).next().unwrap_or(value))
        .context("latest-release redirect carried no tag")?;
    safe_tag(tag)
        .with_context(|| format!("latest-release redirect named {tag:?}"))?;
    Ok(tag.to_string())
}

// ===== listing + extraction =====

fn tar_listing(archive: &Path) -> Result<Vec<ArchiveEntry>> {
    let output = Command::new("tar")
        .args(["-tvzf"])
        .arg(archive)
        .output()
        .context("running tar -tv (the ynpm substrate; is tar installed?)")?;
    if !output.status.success() {
        bail!(
            "tar -tvzf {} failed: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        entries.push(parse_tar_listing_line(line).with_context(|| {
            format!(
                "cannot guard an archive whose listing ynpm cannot read (line class: {:?})",
                trace_safe_text(line)
            )
        })?);
    }
    Ok(entries)
}

fn unzip_listing(archive: &Path) -> Result<Vec<ArchiveEntry>> {
    let output = Command::new("unzip")
        .args(["-l"])
        .arg(archive)
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "zip artifacts need `unzip` ({error}); windows desktop packaging is \
                 DECLARED but this host cannot extract a zip - macOS ships tar.gz \
                 bundles and is unaffected"
            )
        })?;
    if !output.status.success() {
        bail!(
            "unzip -l {} failed: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in text.lines().skip(3) {
        let trimmed = line.trim_end();
        if trimmed.is_empty()
            || trimmed.starts_with("----")
            || trimmed.ends_with(" files")
            || trimmed.ends_with(" file")
        {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 4 || !all_digits(tokens[0]) {
            bail!(
                "zip listing line does not match the unzip -l format: {:?}",
                trace_safe_text(line)
            );
        }
        let name = tokens[3..].join(" ");
        entries.push(ArchiveEntry {
            name,
            size: tokens[0].parse::<u64>().ok(),
            kind: ArchiveEntryKind::File,
        });
    }
    Ok(entries)
}

fn extract_with_guards(
    archive: &Path,
    format: ReleaseArtifactFormat,
    into: &Path,
) -> Result<()> {
    let compressed_size = std::fs::metadata(archive)
        .with_context(|| format!("statting {}", archive.display()))?
        .len();
    if compressed_size > MAX_COMPRESSED_ARCHIVE_BYTES {
        bail!(
            "compressed archive is {compressed_size} bytes, above the \
             {MAX_COMPRESSED_ARCHIVE_BYTES} guard"
        );
    }
    match format {
        ReleaseArtifactFormat::TarGz => {
            let entries = tar_listing(archive)?;
            guard_archive_entries(&entries)?;
            extract_tarball(archive, into)?;
        }
        ReleaseArtifactFormat::Zip => {
            let entries = unzip_listing(archive)?;
            guard_archive_entries(&entries)?;
            let output = Command::new("unzip")
                .args(["-q"])
                .arg(archive)
                .arg("-d")
                .arg(into)
                .stdout(Stdio::null())
                .output()
                .map_err(|error| {
                    anyhow::anyhow!("zip artifacts need `unzip` ({error}) to extract")
                })?;
            if !output.status.success() {
                bail!(
                    "unzip {} failed: {}",
                    archive.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            // unzip -l cannot name symlinks; the post-extraction sweep is
            // the guard for zip-carried links.
            if tree_has_symlinks(into)? {
                bail!(
                    "zip archive produced symlinks; release zips must carry plain \
                     files only - refusing"
                );
            }
        }
        ReleaseArtifactFormat::Apk | ReleaseArtifactFormat::Ipa => {
            bail!("{} artifacts are never extracted by ynpm", format.label())
        }
    }
    Ok(())
}

/// Release archives may wrap their content in one top-level directory
/// (GitHub source archives and common release tooling do). When the
/// extraction holds exactly one directory and nothing else, that directory
/// is the package root; otherwise the extraction root is.
/// The probe must NAME the release version as a whole whitespace-delimited
/// token, not merely contain it as a substring: a probe reading
/// `14.0.0` or `4.0.0-rc1` must not validate against `4.0.0`.
fn probe_text_names_version(text: &str, version: &str) -> bool {
    text.split_whitespace().any(|token| token == version)
}

fn release_extraction_root(extracted: &Path, bundle: Option<&str>) -> Result<PathBuf> {
    let mut entries = std::fs::read_dir(extracted)
        .with_context(|| format!("reading {}", extracted.display()))?;
    let first = entries.next();
    let Some(first) = first else {
        bail!("extraction produced an empty tree");
    };
    let first = first.with_context(|| format!("reading {}", extracted.display()))?;
    if entries.next().is_some() {
        return Ok(extracted.to_path_buf());
    }
    let path = first.path();
    if first.file_type().with_context(|| format!("statting {}", path.display()))?.is_dir() {
        // A bundle-shaped archive whose only top-level entry IS the declared
        // bundle (e.g. `yggterm.app/`) must keep the extraction root: the
        // bundle checks resolve against root.join(bundle), and unwrapping
        // would look for the bundle inside itself.
        let is_declared_bundle = bundle.is_some_and(|bundle| {
            path.file_name().and_then(|name| name.to_str()) == Some(bundle)
        });
        if is_declared_bundle {
            Ok(extracted.to_path_buf())
        } else {
            Ok(path)
        }
    } else {
        Ok(extracted.to_path_buf())
    }
}

// ===== the offline transaction core =====

/// What one selected package produced. Mobile selections never install:
/// they become an official-store handoff. Web selections are declined with
/// their owning plane named.
#[derive(Debug, Clone)]
pub enum ReleaseInstallResult {
    Installed(InstallOutcome),
    Handoff {
        package: String,
        url: String,
        installer_hint: Option<String>,
    },
    Declined {
        package: String,
        reason: String,
    },
}

pub struct ReleaseInstallArgs<'a> {
    pub manifest_doc: &'a serde_json::Value,
    /// `--package <id>` selection; None lets a single-candidate release
    /// install unattended.
    pub package_filter: Option<&'a str>,
    pub destination: Option<&'a Path>,
    /// Audit locator recorded in state, e.g. `github:owner/repo@v4.0.0`.
    pub origin: String,
    /// True when the caller pinned a tag: same-version reinstall and
    /// explicit version requests are operator intent.
    pub explicit_tag: bool,
    /// Pre-verified archive bytes keyed by artifact filename. The network
    /// path fills this; tests fill it from fixture files.
    pub archives: &'a BTreeMap<String, PathBuf>,
}

/// Select and install every package a release manifest offers this host.
/// All-or-nothing per package, never across packages: a later package's
/// failure does not unwind an earlier one's publication.
pub fn install_from_release_manifest(
    paths: &Paths,
    args: &ReleaseInstallArgs,
) -> Result<Vec<ReleaseInstallResult>> {
    let manifest = validate_release_manifest(args.manifest_doc)
        .context("refusing the release before any install")?;
    if let Some(tag) = args
        .origin
        .rsplit_once('@')
        .map(|(_, tag)| tag.to_string())
        .filter(|tag| !tag.is_empty())
    {
        if manifest.release.tag != tag {
            bail!(
                "manifest declares tag {:?} but the release resolved to {tag:?}; \
                 refusing a mismatched manifest",
                manifest.release.tag
            );
        }
    }
    let host_platform = release_platform_for_host()?;
    let selected: Vec<&ReleasePackage> = match args.package_filter {
        Some(wanted) => {
            let Some(package) = manifest
                .packages
                .iter()
                .find(|package| package.id == wanted)
            else {
                let ids: Vec<String> = manifest
                    .packages
                    .iter()
                    .map(|package| format!("{} ({:?})", package.id, package.role))
                    .collect();
                bail!(
                    "release {:?} has no package {:?}; it ships: {}",
                    manifest.release.tag,
                    wanted,
                    ids.join(", ")
                );
            };
            vec![package]
        }
        None => {
            // Unattended selection installs desktop packages only; seed
            // packages are activated by their own transaction.
            let candidates: Vec<&ReleasePackage> = manifest
                .packages
                .iter()
                .filter(|package| {
                    package.role == ReleasePackageRole::Desktop
                        && package
                            .artifacts
                            .iter()
                            .any(|artifact| artifact.platform == host_platform)
                })
                .collect();
            match candidates.len() {
                0 => {
                    let inventory: Vec<String> = manifest
                        .packages
                        .iter()
                        .map(|package| {
                            let platforms: Vec<String> =
                                package.artifacts.iter().map(|a| a.platform.clone()).collect();
                            format!(
                                "{} ({:?}) ships [{}]",
                                package.id,
                                package.role,
                                platforms.join(", ")
                            )
                        })
                        .collect();
                    let seed_note = if manifest
                        .packages
                        .iter()
                        .any(|package| package.role == ReleasePackageRole::Seed)
                    {
                        "; seed packages in this release are activated with `ynpm seed-activate`"
                    } else {
                        ""
                    };
                    bail!(
                        "release {:?} ships no installable package for {host_platform}; \
                         it declares: {}{}",
                        manifest.release.tag,
                        inventory.join("; "),
                        seed_note
                    );
                }
                1 => candidates,
                _ => {
                    let ids: Vec<String> =
                        candidates.iter().map(|package| package.id.clone()).collect();
                    bail!(
                        "release {:?} ships {} installable packages for {host_platform} ({}); \
                         choose one with --package <id>",
                        manifest.release.tag,
                        candidates.len(),
                        ids.join(", ")
                    );
                }
            }
        }
    };
    let mut results = Vec::new();
    for package in selected {
        let result = install_release_package(paths, &manifest, package, args, &host_platform);
        match result {
            Ok(outcome) => results.push(outcome),
            // A selected package is one transaction; its failure must be
            // loud, never absorbed into a partial success report.
            Err(error) => return Err(error).with_context(|| format!("package {}", package.id)),
        }
    }
    Ok(results)
}

fn install_release_package(
    paths: &Paths,
    manifest: &ReleaseManifest,
    package: &ReleasePackage,
    args: &ReleaseInstallArgs,
    host_platform: &str,
) -> Result<ReleaseInstallResult> {
    ynpm_trace(
        paths,
        "release.package.install.begin",
        serde_json::json!({
            "package": &package.id,
            "role": format!("{:?}", package.role),
            "origin": trace_safe_text(&args.origin),
        }),
    );
    match package.role {
        ReleasePackageRole::Mobile => {
            let handoff = package
                .handoff
                .as_ref()
                .context("validated mobile package lost its handoff")?;
            ynpm_trace(
                paths,
                "release.handoff",
                serde_json::json!({
                    "package": &package.id,
                    "url": trace_safe_text(&handoff.url),
                }),
            );
            eprintln!(
                "ynpm: {} is a mobile porcelain; install it from the official package: {}{}",
                package.id,
                handoff.url,
                handoff
                    .installer_hint
                    .as_deref()
                    .map(|hint| format!(" ({hint})"))
                    .unwrap_or_default()
            );
            return Ok(ReleaseInstallResult::Handoff {
                package: package.id.clone(),
                url: handoff.url.clone(),
                installer_hint: handoff.installer_hint.clone(),
            });
        }
        ReleasePackageRole::Web => {
            ynpm_trace(
                paths,
                "release.declined",
                serde_json::json!({ "package": &package.id, "role": "web" }),
            );
            eprintln!(
                "ynpm: {} is a web porcelain; the yggterm web plane delivers it - \
                 this transaction installs desktop and seed packages only",
                package.id
            );
            return Ok(ReleaseInstallResult::Declined {
                package: package.id.clone(),
                reason: "web delivery is owned by the yggterm web plane".to_string(),
            });
        }
        ReleasePackageRole::Seed => {
            // One policy owner: the seed transaction. Letting a seed package
            // through the desktop generation path would stage
            // `~/.local/bin/{ygg,ynpm,ynpx}` links and a desktop state row,
            // and a later desktop removal would unlink the seed's binaries
            // out from under every session.
            ynpm_trace(
                paths,
                "release.declined",
                serde_json::json!({ "package": &package.id, "role": "seed" }),
            );
            bail!(
                "{} is a seed package; the seed transaction owns it - run \
                 `ynpm seed-activate --archive <archive> --manifest <manifest>` \
                 instead of the desktop generation path",
                package.id
            );
        }
        ReleasePackageRole::Desktop => {}
    }
    let Some(artifact) = package
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == host_platform)
    else {
        let platforms: Vec<String> =
            package.artifacts.iter().map(|a| a.platform.clone()).collect();
        bail!(
            "package {} ships [{}] and has no {host_platform} artifact",
            package.id,
            platforms.join(", ")
        );
    };
    let archive = args
        .archives
        .get(&artifact.filename)
        .context("caller did not stage the selected artifact's archive")?;
    let outcome = install_release_package_from_archive(
        paths,
        manifest,
        package,
        artifact,
        archive,
        args.destination,
        &args.origin,
        args.explicit_tag,
    )?;
    Ok(ReleaseInstallResult::Installed(outcome))
}

/// Verify → extract → gate → stage → publish for ONE artifact. Every step
/// before publication is recoverable: a failure deletes staging and leaves
/// the installed state untouched.
#[allow(clippy::too_many_arguments)]
pub fn install_release_package_from_archive(
    paths: &Paths,
    manifest: &ReleaseManifest,
    package: &ReleasePackage,
    artifact: &ReleaseArtifact,
    archive: &Path,
    destination: Option<&Path>,
    origin: &str,
    explicit_tag: bool,
) -> Result<InstallOutcome> {
    let version = manifest.release.version.clone();
    let key = package_storage_key(&package.id);
    // Downgrade policy mirrors the registry channel: an implicit "latest"
    // never silently moves backwards; an explicit tag is operator intent.
    {
        let state = paths.load_state()?;
        if let Some(entry) = state.packages.get(&key) {
            if let (Ok(current), Ok(target)) =
                (crate::SemVer::parse(&entry.current), crate::SemVer::parse(&version))
            {
                if crate::SemVer::cmp_semver(&current, &target) == std::cmp::Ordering::Greater
                    && !explicit_tag
                {
                    bail!(
                        "refusing to move {} from {} back to {version} without an explicit tag",
                        package.id,
                        entry.current
                    );
                }
            }
        }
    }
    let expected_digest = artifact.sha256.to_ascii_lowercase();
    let actual_digest = sha256_file(archive)
        .with_context(|| format!("hashing {}", archive.display()))?;
    if actual_digest != expected_digest {
        bail!(
            "artifact {} sha256 mismatch: expected {expected_digest}, got {actual_digest}",
            artifact.filename
        );
    }
    if let Some(size) = artifact.size {
        let actual_size = std::fs::metadata(archive)
            .with_context(|| format!("statting {}", archive.display()))?
            .len();
        if actual_size != size {
            bail!(
                "artifact {} declares {size} bytes but the staged archive is {actual_size}",
                artifact.filename
            );
        }
    }
    ynpm_trace(
        paths,
        "release.artifact.verified",
        serde_json::json!({
            "package": &package.id,
            "platform": &artifact.platform,
            "sha256": actual_digest,
        }),
    );
    if !artifact.format.installable() {
        bail!(
            "artifact format {} is not installable",
            artifact.format.label()
        );
    }
    let scratch = paths.scratch().join(format!(
        "release-{}-{}",
        key,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .with_context(|| format!("creating {}", scratch.display()))?;
    let extracted = scratch.join("extracted");
    let extraction = extract_with_guards(archive, artifact.format, &extracted);
    if let Err(error) = extraction {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(error);
    }
    let root = match release_extraction_root(&extracted, artifact.bundle.as_deref()) {
        Ok(root) => root,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(error);
        }
    };
    let verification = verify_release_content(&root, manifest, package, artifact);
    if let Err(error) = verification {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(error);
    }
    ynpm_trace(
        paths,
        "release.content.verified",
        serde_json::json!({
            "package": &package.id,
            "bins": &artifact.bins,
            "bundle": &artifact.bundle,
        }),
    );
    let publication = publish_release_generation(
        paths, package, artifact, &root, &key, &version, destination, origin,
    );
    let _ = std::fs::remove_dir_all(&scratch);
    publication
}

/// Content gates BEFORE any bytes move into generations/: every declared
/// bin answers `--version` with the release version, the bundle (when
/// declared) exists, the launch entry exists and is executable, and the
/// version probe file contains the release version.
fn verify_release_content(
    root: &Path,
    manifest: &ReleaseManifest,
    package: &ReleasePackage,
    artifact: &ReleaseArtifact,
) -> Result<()> {
    let version = &manifest.release.version;
    for bin in &artifact.bins {
        let bin_path = root.join(bin);
        if !bin_path.is_file() {
            bail!(
                "package {} artifact declares bin {bin:?}, absent from the archive",
                package.id
            );
        }
        set_executable(&bin_path)?;
        let answer = run_version(&bin_path)
            .with_context(|| format!("gating {bin} of package {}", package.id))?;
        if !version_answer_matches(&answer, version) {
            bail!(
                "REFUSED: {} of package {} answered {:?}, not the release version {version}",
                bin,
                package.id,
                answer.trim()
            );
        }
    }
    if let Some(bundle) = &artifact.bundle {
        let bundle_path = root.join(bundle);
        if !bundle_path.is_dir() {
            bail!(
                "package {} artifact declares bundle {bundle:?}, absent from the archive",
                package.id
            );
        }
    }
    if let Some(launch) = &package.launch {
        let entry = root.join(&launch.entry);
        if !entry.is_file() {
            bail!(
                "package {} launch entry {:?} is absent from the archive",
                package.id,
                launch.entry
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&entry)
                .with_context(|| format!("statting {}", entry.display()))?
                .permissions()
                .mode();
            if mode & 0o111 == 0 {
                bail!(
                    "package {} launch entry {:?} is not executable",
                    package.id,
                    launch.entry
                );
            }
        }
    }
    if let Some(probe) = &package.version_probe {
        let probe_path = root.join(&probe.file);
        let text = std::fs::read(&probe_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .with_context(|| {
                format!(
                    "reading version probe {:?} of package {}",
                    probe.file, package.id
                )
            })?;
        if !probe_text_names_version(&text, version) {
            bail!(
                "version probe {:?} of package {} does not name the release version {version} \
                 as an exact token",
                probe.file,
                package.id
            );
        }
    }
    Ok(())
}

/// Stage the verified tree into the generation store, then publish. The
/// staging directory is discarded or renamed wholesale - the generations
/// tree never observes a partial package.
#[allow(clippy::too_many_arguments)]
fn publish_release_generation(
    paths: &Paths,
    package: &ReleasePackage,
    artifact: &ReleaseArtifact,
    root: &Path,
    key: &str,
    version: &str,
    destination: Option<&Path>,
    origin: &str,
) -> Result<InstallOutcome> {
    let destination = destination
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.dest());
    if artifact.bundle.is_some() && !cfg!(unix) {
        // publish_link copies on Windows; a bundle directory cannot be
        // published that way. Windows desktop packaging stays DECLARED
        // (the schema validates it) and honestly unshipped here.
        bail!(
            "desktop bundle publication needs symlink support; windows desktop \
             packaging is declared in the release schema but not shipped by \
             this transaction"
        );
    }
    let generation = paths.generation_dir(key, version);
    let staging = paths
        .generations()
        .join(format!(".release-{key}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("creating staging {}", staging.display()))?;
    let stage_result = (|| -> Result<()> {
        copy_dir_all(root, &staging)?;
        for bin in &artifact.bins {
            let source = staging.join(bin);
            if !source.is_file() {
                bail!("staged generation lost bin {bin:?}");
            }
            let target = staging.join("bin").join(bin);
            std::fs::create_dir_all(staging.join("bin"))?;
            std::fs::rename(&source, &target)
                .with_context(|| format!("moving bin {bin} into the generation bin table"))?;
            set_executable(&target)?;
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    // A complete same-version generation is kept (idempotent repoint); an
    // incomplete one is replaced wholesale.
    if generation.exists() {
        let complete = artifact.bins.iter().all(|bin| {
            generation.join("bin").join(bin).is_file()
        }) && artifact
            .bundle
            .as_ref()
            .map(|bundle| generation.join(bundle).is_dir())
            .unwrap_or(true);
        if complete {
            let _ = std::fs::remove_dir_all(&staging);
        } else {
            std::fs::remove_dir_all(&generation)
                .with_context(|| format!("replacing incomplete {}", generation.display()))?;
            std::fs::rename(&staging, &generation)
                .with_context(|| format!("publishing {}", generation.display()))?;
        }
    } else {
        // The generation lives at generations/<key>/<version>; the per-package
        // parent is created here, never assumed, or the rename loses to ENOENT.
        let parent = generation
            .parent()
            .context("generation path has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        std::fs::rename(&staging, &generation)
            .with_context(|| format!("publishing {}", generation.display()))?;
    }
    // State and links only move AFTER the generation exists.
    let mut state = paths.load_state()?;
    let previous = state
        .packages
        .get(key)
        .map(|entry| entry.current.clone());
    let previous_integration = state
        .packages
        .get(key)
        .and_then(|entry| entry.integration.clone());
    let bins_map: BTreeMap<String, String> = artifact
        .bins
        .iter()
        .map(|bin| (bin.clone(), format!("bin/{bin}")))
        .collect();
    let entry = state
        .packages
        .entry(key.to_string())
        .or_insert(crate::Package {
            package_name: Some(package.id.clone()),
            current: version.to_string(),
            versions: Vec::new(),
            bins: BTreeMap::new(),
            external_prev: None,
            destination: Some(destination.display().to_string()),
            dev: None,
            dev_generation: None,
            channel: Some("github-release".to_string()),
            source: Some(origin.to_string()),
            integration: None,
        });
    if entry.versions.last() != Some(&version.to_string()) {
        entry.versions.push(version.to_string());
    }
    entry.package_name = Some(package.id.clone());
    entry.current = version.to_string();
    entry.bins = bins_map.clone();
    entry.destination = Some(destination.display().to_string());
    entry.dev = None;
    entry.dev_generation = None;
    entry.channel = Some("github-release".to_string());
    entry.source = Some(origin.to_string());
    let integration = match &package.app {
        Some(app) => package_integration(&serde_json::json!({ "yggterm": app }))
            .context("release manifest app block")?,
        None => None,
    };
    entry.integration = integration.clone();
    std::fs::create_dir_all(&destination)?;
    for bin in artifact.bins.iter() {
        publish_link(&generation.join("bin").join(bin), &destination.join(bin))?;
    }
    if let Some(bundle) = &artifact.bundle {
        publish_link(&generation.join(bundle), &destination.join(bundle))?;
    }
    paths.save_state(&state)?;
    let app_manifest = crate::app_manifest_from_metadata(
        &package.id,
        integration.as_ref(),
        &destination,
        &bins_map,
    )?;
    sync_app_registration(paths, &package.id, previous_integration.as_ref(), app_manifest)?;
    ynpm_trace(
        paths,
        "release.publish.complete",
        serde_json::json!({
            "package": &package.id,
            "version": version,
            "origin": trace_safe_text(origin),
            "destination": destination.display().to_string(),
        }),
    );
    println!(
        "ynpm: {}@{} ({:?}) installed from {} -> {}",
        package.id,
        version,
        package.role,
        origin,
        destination.display()
    );
    Ok(InstallOutcome {
        name: package.id.clone(),
        version: version.to_string(),
        previous,
        bins: artifact.bins.clone(),
    })
}

// ===== the network orchestration =====

/// `ygg ynpm install https://github.com/<owner>/<repo>[@tag]`: resolve the
/// release, fetch the manifest, download the selected artifacts, and hand
/// the verified bytes to the offline transaction.
pub fn install_github_release(
    paths: &Paths,
    source: &GitHubReleaseSource,
    package_filter: Option<&str>,
    destination: Option<&Path>,
) -> Result<Vec<ReleaseInstallResult>> {
    let tag = match &source.tag {
        Some(tag) => tag.clone(),
        None => resolve_latest_tag(&source.repo)
            .with_context(|| format!("resolving the latest release of {}", source.repo))?,
    };
    let manifest_url = release_download_url(&source.repo, &tag, RELEASE_MANIFEST_ASSET);
    ynpm_trace(
        paths,
        "release.resolve",
        serde_json::json!({
            "repo": trace_safe_text(&source.repo),
            "tag": &tag,
            "manifest": trace_safe_text(&manifest_url),
        }),
    );
    let manifest_bytes = crate::curl(&manifest_url)
        .with_context(|| format!("fetching the release manifest {manifest_url}"))?;
    let manifest_doc: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .context("parsing the release manifest")?;
    // Pre-validate so the download list is exactly the manifest's list.
    let manifest = validate_release_manifest(&manifest_doc)
        .context("refusing the release before any download")?;
    let host_platform = release_platform_for_host()?;
    let wanted: Vec<&ReleasePackage> = match package_filter {
        Some(id) => vec![manifest
            .packages
            .iter()
            .find(|package| package.id == id)
            .with_context(|| format!("release {tag} has no package {id}"))?],
        None => manifest
            .packages
            .iter()
            .filter(|package| {
                package.role == ReleasePackageRole::Desktop
                    && package
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.platform == host_platform)
            })
            .collect(),
    };
    let mut archives = BTreeMap::new();
    for package in &wanted {
        let Some(artifact) = package
            .artifacts
            .iter()
            .find(|artifact| artifact.platform == host_platform)
        else {
            continue;
        };
        if !artifact.format.installable() {
            continue;
        }
        let url = release_download_url(&source.repo, &tag, &artifact.filename);
        let body = crate::curl(&url)
            .with_context(|| format!("downloading {url}"))?;
        let staged = paths.scratch().join(format!(
            "release-dl-{}-{}-{}",
            package_storage_key(&package.id),
            tag,
            std::process::id()
        ));
        std::fs::create_dir_all(staged.parent().context("scratch has no parent")?)?;
        std::fs::write(&staged, &body)
            .with_context(|| format!("staging {}", staged.display()))?;
        archives.insert(artifact.filename.clone(), staged);
    }
    let origin = format!("github:{}@{}", source.repo, tag);
    let args = ReleaseInstallArgs {
        manifest_doc: &manifest_doc,
        package_filter,
        destination,
        origin,
        explicit_tag: source.tag.is_some(),
        archives: &archives,
    };
    let results = install_from_release_manifest(paths, &args);
    for archive in archives.values() {
        let _ = std::fs::remove_file(archive);
    }
    results
}

// ===== seed activation (single transaction owner) =====

/// `ynpm seed-activate --archive PATH --manifest PATH [--seed-root DIR]
/// [--expect-version V] [--package ID] [--json]`
///
/// The seed-activation transaction an installer calls ONCE: verify the
/// local seed archive against the manifest, gate its bins, publish
/// `<seed-root>/generations/<version>-<digest12>/`, then atomically flip
/// `<seed-root>/current`. Alias/PATH wiring stays with the installer; this
/// verb owns the generation policy so shell/PowerShell installers never
/// reimplement it.
pub fn verb_seed_activate(paths: &Paths, args: &[String]) -> Result<()> {
    let mut archive: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut seed_root: Option<PathBuf> = None;
    let mut expect_version: Option<String> = None;
    let mut package_filter: Option<String> = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--archive" => {
                index += 1;
                archive = Some(PathBuf::from(
                    args.get(index).context("--archive wants a path")?,
                ));
            }
            "--manifest" => {
                index += 1;
                manifest = Some(PathBuf::from(
                    args.get(index).context("--manifest wants a path")?,
                ));
            }
            "--seed-root" => {
                index += 1;
                seed_root = Some(PathBuf::from(
                    args.get(index).context("--seed-root wants a directory")?,
                ));
            }
            "--expect-version" => {
                index += 1;
                expect_version = Some(
                    args.get(index)
                        .context("--expect-version wants a version")?
                        .clone(),
                );
            }
            "--package" => {
                index += 1;
                package_filter = Some(
                    args.get(index)
                        .context("--package wants an id")?
                        .clone(),
                );
            }
            "--json" => json = true,
            other => bail!("unknown seed-activate option '{other}'"),
        }
        index += 1;
    }
    let archive = archive.context("seed-activate wants --archive PATH")?;
    let manifest_path = manifest.context("seed-activate wants --manifest PATH")?;
    if cfg!(windows) {
        // The unix flip is a staged-symlink rename over `current`. On
        // Windows publish_link copies instead of linking, and the source
        // here is a whole generation directory - a copy the pointer needs
        // but cannot become. Until a native junction-based current flip is
        // qualified, refuse BEFORE any byte moves.
        bail!(
            "seed activation is declared in the release schema but the windows \
             generation pointer (a junction-based `current` flip) is not yet \
             qualified; refusing before any bytes move"
        );
    }
    if !archive.is_file() {
        bail!("seed archive {} is not a file", archive.display());
    }
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest_doc: serde_json::Value = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let manifest = validate_release_manifest(&manifest_doc)
        .context("refusing the seed release before activation")?;
    if let Some(expected) = &expect_version {
        if *expected != manifest.release.version {
            bail!(
                "installer expected seed version {expected} but the manifest declares {}; \
                 refusing to activate a mismatched seed",
                manifest.release.version
            );
        }
    }
    let seed_packages: Vec<&ReleasePackage> = manifest
        .packages
        .iter()
        .filter(|package| package.role == ReleasePackageRole::Seed)
        .collect();
    let package = match (&package_filter, seed_packages.as_slice()) {
        (Some(id), _) => seed_packages
            .iter()
            .find(|package| package.id == *id)
            .copied()
            .with_context(|| format!("seed release has no package {id}"))?,
        (None, [only]) => *only,
        (None, _) => bail!(
            "seed release carries {} seed packages; pass --package <id>",
            seed_packages.len()
        ),
    };
    let host_platform = release_platform_for_host()?;
    let artifact = package
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == host_platform)
        .with_context(|| {
            format!(
                "seed package {} has no {host_platform} artifact",
                package.id
            )
        })?;
    let digest = sha256_file(&archive)?;
    let expected_digest = artifact.sha256.to_ascii_lowercase();
    if digest != expected_digest {
        bail!(
            "seed archive sha256 mismatch: expected {expected_digest}, got {digest}"
        );
    }
    if !artifact.format.installable() {
        bail!("seed artifact format {} is not installable", artifact.format.label());
    }
    let seed_root = seed_root.unwrap_or_else(|| paths.home.join(".yggterm/seed"));
    let digest12 = &digest[..12];
    let generation_dir = seed_root
        .join("generations")
        .join(format!("{}-{}", manifest.release.version, digest12));
    // Idempotent: the same version+digest already active is a no-op.
    if let Ok(link) = std::fs::read_link(seed_root.join("current")) {
        if link.file_name().and_then(|name| name.to_str())
            == Some(&format!("{}-{}", manifest.release.version, digest12))
        {
            report_seed_activation(
                paths,
                &seed_root,
                &generation_dir,
                &manifest,
                &package.id,
                digest12,
                "already-active",
                json,
            )?;
            return Ok(());
        }
    }
    let scratch = paths
        .scratch()
        .join(format!("seed-{}-{}", package_storage_key(&package.id), std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let extraction = (|| -> Result<PathBuf> {
        let extracted = scratch.join("extracted");
        extract_with_guards(&archive, artifact.format, &extracted)?;
        release_extraction_root(&extracted, artifact.bundle.as_deref())
    })();
    let root = match extraction {
        Ok(root) => root,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(error);
        }
    };
    let verification = verify_release_content(&root, &manifest, package, artifact);
    if let Err(error) = verification {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(error);
    }
    let staging = seed_root
        .join("generations")
        .join(format!(".seed-{}-{}", package_storage_key(&package.id), std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let stage_result = (|| -> Result<()> {
        std::fs::create_dir_all(seed_root.join("generations"))?;
        copy_dir_all(&root, &staging)?;
        for bin in &artifact.bins {
            let source = staging.join(bin);
            if !source.is_file() {
                bail!("staged seed lost bin {bin:?}");
            }
            let target = staging.join("bin").join(bin);
            std::fs::create_dir_all(staging.join("bin"))?;
            std::fs::rename(&source, &target)?;
            set_executable(&target)?;
        }
        Ok(())
    })();
    if let Err(error) = stage_result {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(error);
    }
    if generation_dir.exists() {
        // Keep a COMPLETE generation (idempotent repoint); an incomplete or
        // corrupt one - a crashed earlier activation, an emptied directory -
        // is replaced wholesale, never trusted and flipped onto `current`.
        let complete = artifact
            .bins
            .iter()
            .all(|bin| generation_dir.join("bin").join(bin).is_file());
        if complete {
            let _ = std::fs::remove_dir_all(&staging);
        } else {
            std::fs::remove_dir_all(&generation_dir).with_context(|| {
                format!(
                    "replacing incomplete seed generation {}",
                    generation_dir.display()
                )
            })?;
            std::fs::rename(&staging, &generation_dir)
                .with_context(|| format!("publishing {}", generation_dir.display()))?;
        }
    } else {
        std::fs::rename(&staging, &generation_dir)
            .with_context(|| format!("publishing {}", generation_dir.display()))?;
    }
    let _ = std::fs::remove_dir_all(&scratch);
    // The flip is the last write: a staged-symlink rename swaps current
    // atomically, and any earlier failure left the previous generation in
    // charge.
    publish_link(&generation_dir, &seed_root.join("current"))?;
    report_seed_activation(
        paths,
        &seed_root,
        &generation_dir,
        &manifest,
        &package.id,
        digest12,
        "activated",
        json,
    )?;
    Ok(())
}

fn report_seed_activation(
    paths: &Paths,
    seed_root: &Path,
    generation_dir: &Path,
    manifest: &ReleaseManifest,
    package_id: &str,
    digest12: &str,
    outcome: &str,
    json: bool,
) -> Result<()> {
    ynpm_trace(
        paths,
        "seed.activate",
        serde_json::json!({
            "package": package_id,
            "version": &manifest.release.version,
            "digest12": digest12,
            "outcome": outcome,
            "generation": generation_dir.display().to_string(),
        }),
    );
    if json {
        println!(
            "{}",
            serde_json::json!({
                "outcome": outcome,
                "version": manifest.release.version,
                "tag": manifest.release.tag,
                "digest12": digest12,
                "current": seed_root.join("current").display().to_string(),
                "generation": generation_dir.display().to_string(),
            })
        );
    } else {
        println!(
            "ynpm: seed {} ({outcome}) at {}",
            manifest.release.version,
            generation_dir.display()
        );
        println!(
            "  current -> {}",
            seed_root.join("current").display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Sandbox {
        home: PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .expect("test home")
                .join(".yggterm/scratchpad/ynpm")
                .join(format!("rel-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&home);
            fs::create_dir_all(&home).unwrap();
            Self { home }
        }
        fn paths(&self) -> Paths {
            Paths::new(self.home.clone())
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn bin_script(text: &str) -> String {
        format!("#!/bin/sh\ncat <<'EOF'\n{text}\nEOF\n")
    }

    /// Build a tar.gz fixture whose extracted tree declares `bins` (each
    /// answering `--version` with `version`) and optionally a bundle dir
    /// carrying a version marker file. Returns the archive path and its
    /// real sha256.
    fn fixture_archive(
        sandbox: &Sandbox,
        name: &str,
        version: &str,
        bins: &[&str],
        bundle: Option<&str>,
    ) -> (PathBuf, String) {
        let build = sandbox.home.join(format!("build-{name}"));
        fs::create_dir_all(&build).unwrap();
        for bin in bins {
            let path = build.join(bin);
            fs::write(&path, bin_script(&format!("example-{bin} {version}"))).unwrap();
            set_executable(&path).unwrap();
        }
        // Every fixture carries a top-level version probe file so manifests
        // can gate `version_probe: {file: "version"}`.
        fs::write(build.join("version"), format!("fixture {version}\n")).unwrap();
        if let Some(bundle) = bundle {
            fs::create_dir_all(build.join(bundle)).unwrap();
            fs::write(
                build.join(bundle).join("version"),
                format!("example bundle {version}\n"),
            )
            .unwrap();
        }
        let archive = sandbox.home.join(format!("{name}.tar.gz"));
        let mut args = vec![
            "-czf".to_string(),
            archive.display().to_string(),
            "-C".to_string(),
            build.display().to_string(),
        ];
        for bin in bins {
            args.push(bin.to_string());
        }
        args.push("version".to_string());
        if let Some(bundle) = bundle {
            args.push(bundle.to_string());
        }
        let output = Command::new("tar")
            .args(&args)
            .output()
            .expect("tar fixture");
        assert!(
            output.status.success(),
            "tar fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let digest = sha256_file(&archive).unwrap();
        (archive, digest)
    }

    fn manifest_doc(
        version: &str,
        tag: &str,
        package_id: &str,
        role: &str,
        filename: &str,
        digest: &str,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "release": { "version": version, "tag": tag },
            "packages": [serde_json::Value::Object(
                [
                    (
                        "id".to_string(),
                        serde_json::Value::String(package_id.to_string())
                    ),
                    ("role".to_string(), serde_json::Value::String(role.to_string())),
                    (
                        "artifacts".to_string(),
                        serde_json::json!([
                            {
                                "platform": release_platform_for_host().unwrap(),
                                "format": "tar.gz",
                                "filename": filename,
                                "sha256": digest,
                                "bins": ["example-gui"],
                            }
                        ])
                    ),
                ]
                .into_iter()
                .chain(extra.as_object().unwrap().clone())
                .collect()
            )]
        })
    }

    fn run_install(
        sandbox: &Sandbox,
        doc: &serde_json::Value,
        archives: &BTreeMap<String, PathBuf>,
        filter: Option<&str>,
    ) -> Result<Vec<ReleaseInstallResult>> {
        // The simulated release resolves to the tag the manifest itself
        // declares; a helper pinned to one tag fails every other version.
        let tag = doc["release"]["tag"].as_str().unwrap_or("v4.0.0");
        let args = ReleaseInstallArgs {
            manifest_doc: doc,
            package_filter: filter,
            destination: None,
            origin: format!("github:example/example@{tag}"),
            explicit_tag: false,
            archives,
        };
        install_from_release_manifest(&sandbox.paths(), &args)
    }

    #[test]
    fn parse_accepts_url_github_shorthand_and_refuses_subpaths() {
        let plain = parse_github_release_source("https://github.com/example/example").unwrap();
        assert_eq!(plain.repo, "example/example");
        assert_eq!(plain.tag, None);
        let shorthand = parse_github_release_source("github:example/example@v4.0.0").unwrap();
        assert_eq!(shorthand.tag.as_deref(), Some("v4.0.0"));
        let trailing = parse_github_release_source("https://github.com/example/example/").unwrap();
        assert_eq!(trailing.repo, "example/example");
        let error =
            parse_github_release_source("https://github.com/example/example/tree/main").unwrap_err();
        assert!(error.to_string().contains("repository root"));
        assert!(!looks_like_github_release_source("https://gitlab.com/a/b"));
    }

    #[test]
    fn plaintext_http_and_hostile_slugs_are_refused() {
        // Plaintext http is a transport downgrade, never a synonym for https.
        assert!(!looks_like_github_release_source("http://github.com/example/example"));
        assert!(parse_github_release_source("http://github.com/example/example").is_err());
        // Slugs are joined into download URLs and persisted into state; a
        // traversal or stray character never reaches one.
        for evil in [
            "https://github.com/../evil",
            "https://github.com/example/..",
            "https://github.com/a b/c",
            "https://github.com/example/exa mple",
        ] {
            let error = parse_github_release_source(evil).unwrap_err();
            assert!(
                error.to_string().contains("slug"),
                "{evil:?} was not refused as a slug: {error:#}"
            );
        }
    }

    #[test]
    fn install_publishes_generation_and_links() {
        let sandbox = Sandbox::new("publish");
        let (archive, digest) = fixture_archive(
            &sandbox,
            "gui",
            "4.0.0",
            &["example-gui"],
            Some("example-gui-linux"),
        );
        let doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "gui.tar.gz",
            &digest,
            serde_json::json!({
                "version_probe": { "file": "example-gui-linux/version" },
                "launch": { "entry": "example-gui" },
            }),
        );
        let mut archives = BTreeMap::new();
        archives.insert("gui.tar.gz".to_string(), archive);
        let results = run_install(&sandbox, &doc, &archives, None).unwrap();
        assert_eq!(results.len(), 1);
        let outcome = match &results[0] {
            ReleaseInstallResult::Installed(outcome) => outcome,
            other => panic!("expected installed, got {other:?}"),
        };
        assert_eq!(outcome.name, "example-gui");
        assert_eq!(outcome.version, "4.0.0");
        let paths = sandbox.paths();
        let destination = paths.dest();
        assert!(destination.join("example-gui").symlink_metadata().is_ok());
        let generation = paths.generation_dir("example-gui", "4.0.0");
        assert!(generation.join("bin/example-gui").is_file());
        assert!(generation.join("example-gui-linux/version").is_file());
        let state = paths.load_state().unwrap();
        let entry = state.packages.get("example-gui").unwrap();
        assert_eq!(entry.channel.as_deref(), Some("github-release"));
        assert_eq!(entry.source.as_deref(), Some("github:example/example@v4.0.0"));
    }

    #[test]
    fn wrong_digest_refuses_and_leaves_no_state() {
        let sandbox = Sandbox::new("baddigest");
        let (archive, _real_digest) =
            fixture_archive(&sandbox, "gui", "4.0.0", &["example-gui"], None);
        let doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "gui.tar.gz",
            &"0".repeat(64),
            serde_json::json!({}),
        );
        let mut archives = BTreeMap::new();
        archives.insert("gui.tar.gz".to_string(), archive);
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        // The digest gate reports under the per-package context chain; a bare
        // to_string() hides it.
        assert!(
            format!("{error:#}").contains("sha256 mismatch"),
            "digest refusal lost its context: {error:#}"
        );
        let state = sandbox.paths().load_state().unwrap();
        assert!(state.packages.is_empty());
        assert!(!sandbox
            .paths()
            .generations()
            .join("example-gui")
            .exists());
    }

    #[test]
    fn bin_answering_wrong_version_refuses_before_publication() {
        let sandbox = Sandbox::new("badversion");
        let build = sandbox.home.join("build-bad");
        fs::create_dir_all(&build).unwrap();
        fs::write(
            build.join("example-gui"),
            bin_script("example-gui 9.9.9"),
        )
        .unwrap();
        set_executable(&build.join("example-gui")).unwrap();
        let archive = sandbox.home.join("bad.tar.gz");
        let output = Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&build)
            .arg("example-gui")
            .output()
            .unwrap();
        assert!(output.status.success());
        let digest = sha256_file(&archive).unwrap();
        let doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "bad.tar.gz",
            &digest,
            serde_json::json!({}),
        );
        let mut archives = BTreeMap::new();
        archives.insert("bad.tar.gz".to_string(), archive);
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("REFUSED"), "version refusal lost context: {message}");
        assert!(message.contains("9.9.9"));
        assert!(sandbox.paths().load_state().unwrap().packages.is_empty());
    }

    #[test]
    fn upgrade_repoints_and_keeps_previous_generation() {
        let sandbox = Sandbox::new("upgrade");
        let mut archives = BTreeMap::new();
        let mut docs = Vec::new();
        for version in ["4.0.0", "4.1.0"] {
            let (archive, digest) =
                fixture_archive(&sandbox, &format!("gui-{version}"), version, &["example-gui"], None);
            archives.insert(format!("{version}.tar.gz"), archive);
            docs.push(manifest_doc(
                version,
                &format!("v{version}"),
                "example-gui",
                "desktop",
                &format!("{version}.tar.gz"),
                &digest,
                serde_json::json!({ "version_probe": { "file": "version" } }),
            ));
        }
        run_install(&sandbox, &docs[0], &archives, None).unwrap();
        run_install(&sandbox, &docs[1], &archives, None).unwrap();
        let paths = sandbox.paths();
        let destination = paths.dest();
        let link = std::fs::read_link(destination.join("example-gui")).unwrap();
        assert!(link.display().to_string().contains("4.1.0"));
        assert!(paths.generation_dir("example-gui", "4.0.0").exists());
        assert!(paths.generation_dir("example-gui", "4.1.0").exists());
        let state = paths.load_state().unwrap();
        let entry = state.packages.get("example-gui").unwrap();
        assert_eq!(entry.current, "4.1.0");
        assert_eq!(entry.versions, vec!["4.0.0".to_string(), "4.1.0".to_string()]);
    }

    #[test]
    fn implicit_latest_refuses_downgrade_but_explicit_tag_allows() {
        let sandbox = Sandbox::new("downgrade");
        let mut archives = BTreeMap::new();
        let mut docs = Vec::new();
        for version in ["4.1.0", "4.0.0"] {
            let (archive, digest) =
                fixture_archive(&sandbox, &format!("gui-{version}"), version, &["example-gui"], None);
            archives.insert(format!("{version}.tar.gz"), archive);
            docs.push(manifest_doc(
                version,
                &format!("v{version}"),
                "example-gui",
                "desktop",
                &format!("{version}.tar.gz"),
                &digest,
                serde_json::json!({ "version_probe": { "file": "version" } }),
            ));
        }
        run_install(&sandbox, &docs[0], &archives, None).unwrap();
        // 4.0.0 is a downgrade; an IMPLICIT latest refuses it.
        let mut explicit = ReleaseInstallArgs {
            manifest_doc: &docs[1],
            package_filter: None,
            destination: None,
            origin: "github:example/example@v4.0.0".to_string(),
            explicit_tag: false,
            archives: &archives,
        };
        let error = install_from_release_manifest(&sandbox.paths(), &explicit).unwrap_err();
        assert!(error.to_string().contains("explicit tag"));
        // An explicit tag is operator intent and succeeds.
        explicit.explicit_tag = true;
        install_from_release_manifest(&sandbox.paths(), &explicit).unwrap();
        assert_eq!(
            sandbox.paths().load_state().unwrap().packages["example-gui"].current,
            "4.0.0"
        );
    }

    #[test]
    fn mobile_selection_becomes_handoff_never_install() {
        let sandbox = Sandbox::new("mobile");
        let doc = serde_json::json!({
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
                            "filename": "example.apk",
                            "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                        }
                    ],
                    "handoff": {
                        "url": "https://example.test/releases/v4.0.0",
                        "installer_hint": "Obtainium or the official store",
                    },
                }
            ],
        });
        let archives = BTreeMap::new();
        let results = run_install(&sandbox, &doc, &archives, Some("example-mobile")).unwrap();
        match &results[0] {
            ReleaseInstallResult::Handoff { url, .. } => {
                assert_eq!(url, "https://example.test/releases/v4.0.0");
            }
            other => panic!("expected handoff, got {other:?}"),
        }
        // No generation, no state row, no destination bytes: the handoff
        // installed nothing, and adb was never part of the path.
        assert!(sandbox.paths().load_state().unwrap().packages.is_empty());
        assert!(!sandbox.paths().generations().join("example-mobile").exists());
    }

    #[test]
    fn web_selection_is_declined_with_owner_named() {
        let sandbox = Sandbox::new("web");
        let doc = serde_json::json!({
            "schema": 1,
            "release": { "version": "4.0.0", "tag": "v4.0.0" },
            "packages": [
                {
                    "id": "example-web",
                    "role": "web",
                    "artifacts": [
                        {
                            "platform": "linux-x64",
                            "format": "tar.gz",
                            "filename": "web.tar.gz",
                            "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                        }
                    ],
                }
            ],
        });
        let archives = BTreeMap::new();
        let results = run_install(&sandbox, &doc, &archives, Some("example-web")).unwrap();
        match &results[0] {
            ReleaseInstallResult::Declined { reason, .. } => {
                assert!(reason.contains("web plane"));
            }
            other => panic!("expected declined, got {other:?}"),
        }
        assert!(sandbox.paths().load_state().unwrap().packages.is_empty());
    }

    #[test]
    fn ambiguous_multi_package_release_requires_selection() {
        let sandbox = Sandbox::new("ambiguous");
        let (archive, digest) = fixture_archive(&sandbox, "gui", "4.0.0", &["example-gui"], None);
        let mut doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "gui.tar.gz",
            &digest,
            serde_json::json!({ "version_probe": { "file": "version" } }),
        );
        let mut second = doc["packages"][0].clone();
        second["id"] = serde_json::json!("example-gui-b");
        doc["packages"].as_array_mut().unwrap().push(second);
        let mut archives = BTreeMap::new();
        archives.insert("gui.tar.gz".to_string(), archive);
        // Two desktop candidates for one host: unattended is ambiguous.
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        assert!(error.to_string().contains("--package"));
        // Explicit selection installs exactly the chosen package.
        let results = run_install(&sandbox, &doc, &archives, Some("example-gui-b")).unwrap();
        assert!(matches!(
            &results[0],
            ReleaseInstallResult::Installed(_)
        ));
    }

    #[test]
    fn seed_package_is_never_installed_through_the_desktop_transaction() {
        let sandbox = Sandbox::new("seedowner");
        let (archive, digest) = fixture_archive(
            &sandbox,
            "seed",
            "4.0.0",
            &["ygg", "ynpm", "ynpx"],
            None,
        );
        let mut doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "ygg-seed",
            "seed",
            "seed.tar.gz",
            &digest,
            serde_json::json!({}),
        );
        // The seed fixture archive ships ygg/ynpm/ynpx; the artifact declares
        // the bins the archive actually carries, never a desktop bin.
        doc["packages"][0]["artifacts"][0]["bins"] =
            serde_json::json!(["ygg", "ynpm", "ynpx"]);
        let mut archives = BTreeMap::new();
        archives.insert("seed.tar.gz".to_string(), archive);
        // Explicit selection is refused with the owning transaction named:
        // one policy owner, so a desktop `--package` can never stage seed
        // links a later removal would tear out.
        let error = run_install(&sandbox, &doc, &archives, Some("ygg-seed")).unwrap_err();
        assert!(
            format!("{error:#}").contains("seed-activate"),
            "seed ownership refusal lost context: {error:#}"
        );
        // Unattended selection is desktop-only; a seed-only release says
        // where its seed goes instead of silently installing nothing.
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("ships no installable package"));
        assert!(message.contains("seed-activate"));
        assert!(sandbox.paths().load_state().unwrap().packages.is_empty());
    }

    #[test]
    fn top_level_bundle_directory_is_not_unwrapped() {
        let sandbox = Sandbox::new("bundle");
        // An archive whose ONLY top-level entry IS the declared bundle: the
        // extraction root must stay the root, or the bundle check would look
        // for the bundle inside itself and refuse a good release.
        let build = sandbox.home.join("build-bundle");
        fs::create_dir_all(build.join("example-gui-linux")).unwrap();
        fs::write(
            build.join("example-gui-linux/version"),
            "example bundle 4.0.0\n",
        )
        .unwrap();
        set_executable(&build.join("example-gui-linux/version")).unwrap();
        let archive = sandbox.home.join("bundle.tar.gz");
        let output = Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&build)
            .arg("example-gui-linux")
            .output()
            .expect("tar fixture");
        assert!(output.status.success());
        let digest = sha256_file(&archive).unwrap();
        let mut doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "bundle.tar.gz",
            &digest,
            serde_json::json!({
                "version_probe": { "file": "example-gui-linux/version" },
                "launch": { "entry": "example-gui-linux/version" },
            }),
        );
        doc["packages"][0]["artifacts"][0]["bins"] = serde_json::json!([]);
        // The top-level directory IS the bundle; declaring it is what keeps
        // the extraction root un-unwrapped.
        doc["packages"][0]["artifacts"][0]["bundle"] =
            serde_json::json!("example-gui-linux");
        let mut archives = BTreeMap::new();
        archives.insert("bundle.tar.gz".to_string(), archive);
        let results = run_install(&sandbox, &doc, &archives, None).unwrap();
        assert!(matches!(&results[0], ReleaseInstallResult::Installed(_)));
        let paths = sandbox.paths();
        assert!(paths.dest().join("example-gui-linux").symlink_metadata().is_ok());
        assert!(paths
            .generation_dir("example-gui", "4.0.0")
            .join("example-gui-linux/version")
            .is_file());
    }

    #[test]
    fn version_probe_requires_the_exact_version_token() {
        let sandbox = Sandbox::new("probe");
        let build = sandbox.home.join("build-probe");
        fs::create_dir_all(&build).unwrap();
        fs::write(build.join("example-gui"), bin_script("example-gui 4.0.0")).unwrap();
        set_executable(&build.join("example-gui")).unwrap();
        // A probe naming a DIFFERENT version must not validate; under the
        // old substring rule "14.0.0" would have matched a "4.0.0" release.
        fs::write(build.join("release-notes"), "yggterm 14.0.0 released\n").unwrap();
        let archive = sandbox.home.join("probe.tar.gz");
        let output = Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&build)
            .arg("example-gui")
            .arg("release-notes")
            .output()
            .expect("tar fixture");
        assert!(output.status.success());
        let digest = sha256_file(&archive).unwrap();
        let doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "example-gui",
            "desktop",
            "probe.tar.gz",
            &digest,
            serde_json::json!({ "version_probe": { "file": "release-notes" } }),
        );
        let mut archives = BTreeMap::new();
        archives.insert("probe.tar.gz".to_string(), archive);
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        assert!(
            format!("{error:#}").contains("does not name the release version"),
            "probe refusal lost context: {error:#}"
        );
        assert!(sandbox.paths().load_state().unwrap().packages.is_empty());
    }

    #[test]
    fn seed_activate_replaces_incomplete_generation_and_keeps_complete() {
        let sandbox = Sandbox::new("seedrepair");
        let (archive, digest) = fixture_archive(
            &sandbox,
            "seed",
            "4.0.0",
            &["ygg", "ynpm", "ynpx"],
            None,
        );
        let mut doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "ygg-seed",
            "seed",
            "seed.tar.gz",
            &digest,
            serde_json::json!({}),
        );
        // The seed fixture archive ships ygg/ynpm/ynpx; the artifact declares
        // the bins the archive actually carries, never a desktop bin.
        doc["packages"][0]["artifacts"][0]["bins"] =
            serde_json::json!(["ygg", "ynpm", "ynpx"]);
        let manifest_path = sandbox.home.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_string(&doc).unwrap()).unwrap();
        let seed_root = sandbox.home.join("seed");
        let args = vec![
            "--archive".to_string(),
            archive.display().to_string(),
            "--manifest".to_string(),
            manifest_path.display().to_string(),
            "--seed-root".to_string(),
            seed_root.display().to_string(),
        ];
        verb_seed_activate(&sandbox.paths(), &args).unwrap();
        let generation_dir = seed_root
            .join("generations")
            .join(format!("4.0.0-{}", &digest[..12]));
        assert!(generation_dir.join("bin/ygg").is_file());
        // A complete generation is kept across a re-activation (idempotent
        // repoint), never restaged over.
        fs::write(generation_dir.join(".marker"), "kept").unwrap();
        let _ = std::fs::remove_file(seed_root.join("current"));
        let gen_name = format!("4.0.0-{}", &digest[..12]);
        verb_seed_activate(&sandbox.paths(), &args).unwrap();
        assert_eq!(
            std::fs::read_link(seed_root.join("current"))
                .unwrap()
                .file_name(),
            Some(std::ffi::OsStr::new(gen_name.as_str()))
        );
        assert!(generation_dir.join(".marker").exists());
        // An incomplete generation - a crashed earlier activation - is
        // replaced wholesale, never trusted and flipped onto `current`.
        std::fs::remove_dir_all(&generation_dir).unwrap();
        std::fs::create_dir_all(&generation_dir).unwrap();
        verb_seed_activate(&sandbox.paths(), &args).unwrap();
        assert!(generation_dir.join("bin/ygg").is_file());
        assert!(generation_dir.join("bin/ynpm").is_file());
        assert!(generation_dir.join("bin/ynpx").is_file());
        assert!(!generation_dir.join(".marker").exists());
    }

    #[test]
    fn missing_host_platform_is_honest() {
        let sandbox = Sandbox::new("noplatform");
        let host = release_platform_for_host().unwrap();
        let other = if host == "linux-arm64" {
            "linux-x64"
        } else {
            "linux-arm64"
        };
        let doc = serde_json::json!({
            "schema": 1,
            "release": { "version": "4.0.0", "tag": "v4.0.0" },
            "packages": [
                {
                    "id": "example-gui",
                    "role": "desktop",
                    "artifacts": [
                        {
                            "platform": other,
                            "format": "tar.gz",
                            "filename": "gui.tar.gz",
                            "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                        }
                    ],
                    "version_probe": { "file": "version" },
                    "launch": { "entry": "launcher" },
                }
            ],
        });
        let archives = BTreeMap::new();
        let error = run_install(&sandbox, &doc, &archives, None).unwrap_err();
        let message = error.to_string();
        assert!(message.contains(&host) || message.contains("ships no installable package"));
    }

    #[test]
    fn seed_activate_publishes_generation_and_flips_current_atomically() {
        let sandbox = Sandbox::new("seed");
        let (archive, digest) = fixture_archive(
            &sandbox,
            "seed",
            "4.0.0",
            &["ygg", "ynpm", "ynpx"],
            None,
        );
        let mut doc = manifest_doc(
            "4.0.0",
            "v4.0.0",
            "ygg-seed",
            "seed",
            "seed.tar.gz",
            &digest,
            serde_json::json!({}),
        );
        // The seed fixture archive ships ygg/ynpm/ynpx; the artifact declares
        // the bins the archive actually carries, never a desktop bin.
        doc["packages"][0]["artifacts"][0]["bins"] =
            serde_json::json!(["ygg", "ynpm", "ynpx"]);
        let manifest_path = sandbox.home.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_string(&doc).unwrap()).unwrap();
        let seed_root = sandbox.home.join("seed");
        let args = vec![
            "--archive".to_string(),
            archive.display().to_string(),
            "--manifest".to_string(),
            manifest_path.display().to_string(),
            "--seed-root".to_string(),
            seed_root.display().to_string(),
        ];
        verb_seed_activate(&sandbox.paths(), &args).unwrap();
        let current = std::fs::read_link(seed_root.join("current")).unwrap();
        assert!(current
            .display()
            .to_string()
            .starts_with("generations/4.0.0-"));
        assert!(current.join("bin/ygg").is_file());
        assert!(current.join("bin/ynpm").is_file());
        assert!(current.join("bin/ynpx").is_file());
        // Idempotent re-run reports already-active and keeps the link.
        verb_seed_activate(&sandbox.paths(), &args).unwrap();
        let current2 = std::fs::read_link(seed_root.join("current")).unwrap();
        assert_eq!(current, current2);
        // Wrong expected version refuses before anything moves.
        let bad = vec![
            "--archive".to_string(),
            archive.display().to_string(),
            "--manifest".to_string(),
            manifest_path.display().to_string(),
            "--seed-root".to_string(),
            seed_root.display().to_string(),
            "--expect-version".to_string(),
            "9.9.9".to_string(),
        ];
        assert!(verb_seed_activate(&sandbox.paths(), &bad).is_err());
        let current3 = std::fs::read_link(seed_root.join("current")).unwrap();
        assert_eq!(current, current3);
    }

    #[test]
    fn zip_listing_reads_an_empty_zip_and_refuses_garbage() {
        // A 22-byte zip end-of-central-directory record is a complete empty
        // archive; `unzip -l` must parse it and the guard must refuse its
        // zero entries (a real package is never empty).
        if Command::new("unzip").arg("-v").output().is_err() {
            // The capability refusal IS the honest path on hosts without
            // unzip; the parser cannot be exercised here.
            return;
        }
        let zip_dir = sandbox_home();
        std::fs::create_dir_all(&zip_dir).unwrap();
        let empty_zip = zip_dir.join("empty.zip");
        std::fs::write(
            &empty_zip,
            [0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let entries = unzip_listing(&empty_zip).unwrap();
        assert!(entries.is_empty());
        let guard = guard_archive_entries(&entries).unwrap_err();
        assert!(guard.to_string().contains("no entries"));
        let garbage = zip_dir.join("garbage.zip");
        std::fs::write(&garbage, b"this is not a zip").unwrap();
        assert!(unzip_listing(&garbage).is_err());
    }

    fn sandbox_home() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("test home")
            .join(".yggterm/scratchpad/ynpm")
            .join(format!("zip-listing-{}", std::process::id()))
    }
}
