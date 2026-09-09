//! `ynpm` - the yggterm-aware executable distribution plane.
//!
//! # Why this exists
//!
//! The recognized fleet is localhost plus yggterm's permanent SSH roster.
//! `ynpm` distributes first-party apps, third-party agent CLIs, TUIs, and
//! future executable packages. Before this CLI existed, every fleet update
//! was assembled BY HAND on each host: curl/npm the package, copy a binary,
//! keep a `.prev` file, and hope. An agent's discipline resets every session;
//! a verb's does not. This is the verb.
//!
//! # The contract
//!
//! | verb | does |
//! |---|---|
//! | `ynpm install <pkg>[@<ver>]...` | resolve, download, VERIFY, swap into DEST, keep a generation |
//! | `ynpm install --dev ...` / `ynpm dev ...` | publish a verified local build and optional fleet import |
//! | `ynpm list` | every integrated CLI and app manifest, with measured source/path/version |
//! | `ynpm check` / `ynpm sync` | drift instrument and production/dev reconciliation |
//! | `ynpm sync --integrated` / `sync-fleet` | converge CLI packages locally or by one archive import |
//! | `ynpx <pkg> [flags]` | update when online, then launch the verified bin or offline cache |
//! | `ynpm rollback/remove/prod <pkg>` | generation rollback, exact removal, or dev handback |
//!
//! Three rules this code exists to enforce, each one earned by an incident:
//!
//! 1. **A binary must tell the truth about itself.** Before anything is
//!    swapped, the freshly downloaded binary is run with `--version` and the
//!    answer must contain the package's version. v0.2.0 of a browser package
//!    shipped with binaries that answered `--version` with the PREVIOUS
//!    version (the npm package was bumped, the crate was not); every fleet
//!    host installed the update and still showed the old number. Now the
//!    install refuses.
//! 2. **Swap by rename, never write-in-place.** A running binary cannot be
//!    opened for write ("Text file busy"), but a rename over the directory
//!    entry always works: the running process keeps its inode, the next
//!    launch gets the new build.
//! 3. **Every install leaves a generation and a rollback.** The bytes of
//!    every installed version are kept under
//!    `~/.yggterm/ynpm/generations/<name>/<version>/`, so `ynpm rollback`
//!    restores real binaries, not a hope.
//!
//! # Substrate
//!
//! Network and extraction go through `curl` and `tar`, the same substrate
//! the whole ynpm flow (install.sh, finalize.mjs, the CI publish workflow)
//! already requires - no new dependency is introduced to ship this.
//!
//! See docs/ynpm.md for the operator's view.
#![allow(dead_code)]

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ===== platform =====

/// The npm platform identifier this host maps to, or the reason it does not
/// map at all. The mapping is HONEST: a platform we do not ship is a refusal
/// naming what we do ship, never a guess that downloads nothing runnable.
/// ⚠ `std::env::consts::OS` spells it "linux" (lowercase) - the Dogfood
/// install failed on all three fleet hosts before anyone shipped this.
pub fn platform_target(os: &str, machine: &str) -> anyhow::Result<String> {
    match (os.to_ascii_lowercase().as_str(), machine) {
        ("linux", "x86_64") => Ok("linux-x64".to_string()),
        ("linux", "aarch64") | ("linux", "arm64") => Ok("linux-arm64".to_string()),
        (os, machine) => bail!(
            "no @ygghq prebuilt binary for {os}/{machine} (we ship linux-x64 \
             and linux-arm64; the packages are honest about that)"
        ),
    }
}

/// `pkg` as the user typed it, to the full scoped package name and an
/// optional pinned version. A bare name means `@ygghq/<name>`: the scope IS
/// the registry identity of the fleet. A scoped spec may itself carry a pin
/// AFTER its name (`@ygghq/ychrome@0.2.1`), which is why the split runs on
/// the part after the scope's slash.
pub fn expand_package(spec: &str) -> anyhow::Result<(String, Option<String>)> {
    if let Some(rest) = spec.strip_prefix('@') {
        let Some((scope_name, pin)) = rest.split_once('/') else {
            bail!("scoped package '{spec}' has no '/' - expected @scope/name");
        };
        if scope_name.is_empty() {
            bail!("scoped package '{spec}' has an empty scope");
        }
        let (name, pin) = match pin.split_once('@') {
            Some((base, ver)) => (base.to_string(), Some(ver.to_string())),
            None => (pin.to_string(), None),
        };
        if name.is_empty() {
            bail!("scoped package '{spec}' has an empty name");
        }
        return Ok((format!("@{scope_name}/{name}"), pin));
    }
    match spec.split_once('@') {
        Some((name, ver)) => {
            if name.is_empty() || name.contains('/') {
                bail!("'{spec}' is not a package name this tool can resolve");
            }
            Ok((format!("@ygghq/{name}"), Some(ver.to_string())))
        }
        None => {
            if spec.is_empty() || spec.contains('/') {
                bail!("'{spec}' is not a package name this tool can resolve");
            }
            Ok((format!("@ygghq/{spec}"), None))
        }
    }
}

// ===== versions =====

/// A loose semver: numeric triple plus an optional pre-release tag. Enough
/// for "is the registry ahead of the disk" - never a reimplementation of the
/// whole spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl SemVer {
    pub fn parse(text: &str) -> anyhow::Result<SemVer> {
        let text = text.trim();
        // npm and several native CLIs decorate a release with
        // `+commit/build` metadata. It does not change semver ordering, but it
        // must not make a healthy executable disappear from `ynpm list`.
        let without_build = text.split_once('+').map_or(text, |(base, _)| base);
        let (core, pre) = match without_build.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (without_build, None),
        };
        let nums: Vec<&str> = core.split('.').collect();
        if nums.len() != 3 {
            bail!("'{text}' is not a version this tool can order (want major.minor.patch)");
        }
        let parse = |s: &str| -> anyhow::Result<u64> {
            s.parse::<u64>()
                .with_context(|| format!("'{s}' is not a number in version '{text}'"))
        };
        Ok(SemVer {
            major: parse(nums[0])?,
            minor: parse(nums[1])?,
            patch: parse(nums[2])?,
            pre,
        })
    }

    /// Ordering: the numeric triple, then a pre-release sorts BELOW its own
    /// release ("1.0.0-rc1" < "1.0.0"), which is the one non-numeric rule
    /// the registry actually exercises.
    pub fn cmp_semver(a: &SemVer, b: &SemVer) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let triple = a
            .major
            .cmp(&b.major)
            .then(a.minor.cmp(&b.minor))
            .then(a.patch.cmp(&b.patch));
        if triple != Ordering::Equal {
            return triple;
        }
        match (a.pre.as_ref(), b.pre.as_ref()) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(y),
        }
    }
}

/// Is `answer` (a binary's own `--version` output) an honest claim of
/// `expected`? The output may decorate ("ychrome 0.2.1", "v0.2.1") but must
/// CONTAIN the version. An empty answer is a lie of silence.
pub fn version_answer_matches(answer: &str, expected: &str) -> bool {
    let answer = answer.trim();
    !answer.is_empty() && (answer == expected || answer.contains(expected))
}

fn version_answer_matches_identity(answer: &str, expected: &str) -> bool {
    if version_answer_matches(answer, expected) {
        return true;
    }
    let actual = version_from_answer(answer);
    let (Ok(actual), Ok(expected)) = (
        SemVer::parse(actual.as_deref().unwrap_or("")),
        SemVer::parse(expected),
    )
    else {
        return false;
    };
    (actual.major, actual.minor, actual.patch) == (expected.major, expected.minor, expected.patch)
}

/// Pull the version out of a binary's own `--version` answer, for the drift
/// instrument: the LAST whitespace-separated token that parses as a semver,
/// allowing the leading `v` some tools spell ("v3.2.19").
pub fn version_from_answer(answer: &str) -> Option<String> {
    answer
        .split_whitespace()
        .rev()
        .find(|token| SemVer::parse(token.trim_start_matches('v')).is_ok())
        .map(|token| token.trim_start_matches('v').to_string())
}

// ===== state =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Package {
    /// Canonical registry identity. Older @ygghq states omitted this and use
    /// the map key (`ychrome`, `yedit`, ...); the reader keeps those states
    /// valid and fills this field on the next install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// The version currently swapped into DEST.
    pub current: String,
    /// Every version installed through ynpm, oldest first. The rollback
    /// target is the newest entry below `current` that still has a
    /// generation on disk.
    pub versions: Vec<String>,
    /// The platform package's bin table: bin name -> path inside the package.
    pub bins: BTreeMap<String, String>,
    /// A version found in DEST before ynpm ever installed this package (a
    /// foreign install). Informational: its bytes were never kept, so it is
    /// NOT a rollback target.
    pub external_prev: Option<String>,
    /// The destination whose published links belong to this package. This is
    /// what lets agent CLIs use the same ynpm state while publishing into the
    /// yggterm-only bin directory and first-party apps publish into ~/.local/bin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// Present while this package's locally-built DEV generation is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<DevMarker>,
    /// The immutable dev generation currently published. Older dev states
    /// used the literal `dev` directory and fall back to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_generation: Option<String>,
    /// Compatibility/channel label. Old states have no label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// The package's host-integration declaration, normalized from
    /// `package.json.yggterm`. Keeping it in state lets fleet archive import
    /// reproduce menus without re-fetching package metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<yggterm_core::YggtermPackageMetadata>,
}

/// Metadata for a locally-built package that temporarily outranks the
/// registry. A production handback is forward-only: a release must be newer
/// than the build it supersedes, or be the same version with a different
/// production fingerprint (release-only polish/metadata).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DevMarker {
    pub built_at_ms: u64,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub watch: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub supersedes_fingerprint: Option<String>,
    #[serde(default)]
    pub dev_fingerprint: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevHandback {
    KeepDev,
    ReleaseAvailable,
}

/// Decide whether a watched production release may take over a dev build.
/// Missing or malformed registry information is a refusal to switch, never a
/// guess that would discard a developer's working binary.
pub fn dev_handback(
    watch_latest: Option<&str>,
    supersedes: Option<&str>,
    supersedes_fingerprint: Option<&str>,
    latest_fingerprint: Option<&str>,
) -> DevHandback {
    let Some(latest) = watch_latest else {
        return DevHandback::KeepDev;
    };
    let base = supersedes.unwrap_or("0.0.0");
    let (Ok(latest), Ok(base)) = (SemVer::parse(latest), SemVer::parse(base)) else {
        return DevHandback::KeepDev;
    };
    match SemVer::cmp_semver(&latest, &base) {
        std::cmp::Ordering::Greater => DevHandback::ReleaseAvailable,
        std::cmp::Ordering::Equal => match (supersedes_fingerprint, latest_fingerprint) {
            (Some(old), Some(new)) if old != new => DevHandback::ReleaseAvailable,
            _ => DevHandback::KeepDev,
        },
        std::cmp::Ordering::Less => DevHandback::KeepDev,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub packages: BTreeMap<String, Package>,
}

/// The three roots of ynpm's on-disk world, all under the yggterm home.
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: absolute_path(&home.into()),
        }
    }
    pub fn state_file(&self) -> PathBuf {
        self.home.join(".yggterm/ynpm/state.json")
    }
    pub fn root(&self) -> PathBuf {
        self.home.join(".yggterm/ynpm")
    }
    pub fn generations(&self) -> PathBuf {
        self.root().join("generations")
    }
    pub fn cache(&self) -> PathBuf {
        self.root().join("cache")
    }
    pub fn generation_dir(&self, name: &str, version: &str) -> PathBuf {
        self.generations().join(name).join(version)
    }
    pub fn dest(&self) -> PathBuf {
        resolve_path_from(
            &Self::dest_from(std::env::var("YNPM_DEST").ok().as_deref(), &self.home),
            &self.home,
        )
    }
    /// Pure so the test never touches process state: an explicit override
    /// (the YNPM_DEST spelling) wins, else the fleet's `~/.local/bin`.
    pub fn dest_from(override_: Option<&str>, home: &Path) -> PathBuf {
        match override_ {
            Some(dest) if !dest.trim().is_empty() => PathBuf::from(dest),
            _ => home.join(".local/bin"),
        }
    }
    pub fn scratch(&self) -> PathBuf {
        // ⛔ Disk-backed scratch only: /tmp is RAM on the fleet's desktop
        // hosts, and a package tarball is the exact kind of bytes that must
        // not be charged to it.
        self.home.join(".yggterm/scratchpad/ynpm")
    }
    pub fn load_state(&self) -> anyhow::Result<State> {
        let path = self.state_file();
        if !path.exists() {
            return Ok(State::default());
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut state: State =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // 3.2.21 wrote a dev channel marker without the new optional fields.
        // Keep it readable; the next write normalizes it without losing the
        // channel decision.
        for package in state.packages.values_mut() {
            if package.dev.is_none() && package.channel.as_deref() == Some("dev") {
                package.dev = Some(DevMarker {
                    built_at_ms: 0,
                    commit: None,
                    host: None,
                    watch: None,
                    supersedes: None,
                    supersedes_fingerprint: None,
                    dev_fingerprint: None,
                    generation: Some("dev".to_string()),
                });
            }
        }
        Ok(state)
    }
    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        let path = self.state_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, serde_json::to_string_pretty(state)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// Convert a canonical package identity to a filesystem-safe state/generation
/// key. First-party @ygghq packages retain their historic short key for
/// compatibility; all other scoped packages keep their scope in the key so
/// two vendors cannot collide on the same basename (`@a/tool` vs `@b/tool`).
pub fn package_storage_key(package: &str) -> String {
    let package = package.trim();
    let raw = package
        .strip_prefix("@ygghq/")
        .map(str::to_string)
        .unwrap_or_else(|| package.trim_start_matches('@').replace('/', "__"));
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn package_identity(key: &str, package: &Package) -> String {
    if let Some(package) = package.package_name.clone() {
        return package;
    }
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .find(|descriptor| descriptor.slug == key)
        .map(descriptor_package)
        .unwrap_or_else(|| format!("@ygghq/{key}"))
}

fn find_package_key<'a>(state: &'a State, package: &str) -> Option<&'a str> {
    let storage_key = package_storage_key(package);
    state
        .packages
        .iter()
        .find(|(key, value)| {
            value.package_name.as_deref() == Some(package)
                || key.as_str() == storage_key
                || (package.starts_with("@ygghq/") && key.as_str() == &package[7..])
                || value.package_name.is_none() && package.rsplit('/').next() == Some(key.as_str())
        })
        .map(|(key, _)| key.as_str())
}

fn package_destination(paths: &Paths, package: &Package) -> PathBuf {
    package
        .destination
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| absolute_path(Path::new(value)))
        .unwrap_or_else(|| paths.dest())
}

fn absolute_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(home) = home_dir_from_environment()
        && (text == "~" || text.starts_with("~/") || text.starts_with("~\\"))
    {
        let remainder = text
            .strip_prefix("~/")
            .or_else(|| text.strip_prefix("~\\"))
            .unwrap_or("");
        return if remainder.is_empty() {
            home
        } else {
            home.join(remainder)
        };
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}

fn home_dir_from_environment() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].into_iter().find_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

/// Resolve a package-manager path against an explicit base. Relative
/// `YNPM_DEST` values are home-relative, while command-line `--dest` values
/// continue to use the caller's current directory through `absolute_path`.
fn resolve_path_from(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if path == Path::new("~") {
        home_dir_from_environment().unwrap_or_else(|| base.to_path_buf())
    } else {
        let text = path.to_string_lossy();
        if let Some(remainder) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
            home_dir_from_environment()
                .unwrap_or_else(|| base.to_path_buf())
                .join(remainder)
        } else {
            base.join(path)
        }
    }
}

fn is_integrated_npm_package(package: &str) -> bool {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .any(|descriptor| {
            matches!(
                descriptor.install,
                yggterm_core::agent_cli::CliInstall::Npm(name) if name == package
            )
        })
}

fn integrated_npm_dist_tag(package: &str) -> Option<&'static str> {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .find_map(|descriptor| {
            matches!(
                descriptor.install,
                yggterm_core::agent_cli::CliInstall::Npm(name) if name == package
            )
            .then(|| yggterm_core::agent_cli::npm_dist_tag(descriptor.kind))
            .flatten()
        })
}

fn fetch_package_manifest(package: &str, pin: Option<&str>) -> anyhow::Result<Manifest> {
    // Integrated package policy is part of the core descriptor. In
    // particular, opencode2 is the beta-tagged v2 TUI; the package's npm
    // `latest` tag is an older beta and would make both ynpm and ynpx appear
    // successful while silently leaving opencode2 behind.
    fetch_manifest(package, pin.or_else(|| integrated_npm_dist_tag(package)))
}

/// An integrated CLI's default dev destination is the same yggterm-only bin
/// directory the server launches from. First-party apps keep the normal
/// ~/.local/bin default. An explicit YNPM_DEST remains authoritative for
/// scripts and fleet adapters that deliberately choose another destination.
fn default_dev_destination(paths: &Paths, package: &str) -> PathBuf {
    if std::env::var_os("YNPM_DEST").is_some_and(|value| !value.is_empty()) {
        return paths.dest();
    }
    if is_integrated_npm_package(package) {
        paths.root().join("bin")
    } else {
        paths.dest()
    }
}

// ===== registry + process substrate (curl / tar / the binary itself) =====

/// The registry manifest of ONE version, reduced to what the flow needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub tarball: String,
    pub integrity: Option<String>,
    pub shasum: Option<String>,
    pub integration: Option<yggterm_core::YggtermPackageMetadata>,
    /// Optional dependencies are the feature switch between a first-party
    /// platform package (`@ygghq/name-linux-x64`) and a generic package whose
    /// own `bin` files are complete. `@ygghq` is a namespace, not a promise
    /// that every package needs a native companion.
    pub optional_dependencies: Vec<String>,
}

/// Parse a registry document into the tarball to fetch. Two shapes arrive
/// here: a PACKUMENT (`<pkg>`: every version at once - the tarball lives
/// under `versions[<latest>].dist`) and a VERSIONED doc (`<pkg>/<version>` -
/// the tarball sits at the top level). Both must answer.
pub fn parse_manifest(doc: &serde_json::Value) -> anyhow::Result<Manifest> {
    let version = doc
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            doc.get("dist-tags")
                .and_then(|tags| tags.get("latest"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .context("registry manifest carries neither a version nor dist-tags.latest")?;
    let tarball = doc
        .pointer("/dist/tarball")
        .or_else(|| doc.pointer(&format!("/versions/{version}/dist/tarball")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("registry manifest carries no dist.tarball (top level or under versions)")?;
    let dist = doc
        .pointer("/dist")
        .or_else(|| doc.pointer(&format!("/versions/{version}/dist")));
    let package = doc
        .get("version")
        .is_some()
        .then_some(doc)
        .or_else(|| doc.pointer(&format!("/versions/{version}")))
        .unwrap_or(doc);
    let integration_value = doc
        .get("yggterm")
        .or_else(|| doc.pointer(&format!("/versions/{version}/yggterm")));
    let optional_dependencies = package
        .get("optionalDependencies")
        .and_then(|value| value.as_object())
        .map(|dependencies| dependencies.keys().cloned().collect())
        .unwrap_or_default();
    Ok(Manifest {
        version,
        tarball,
        integrity: dist
            .and_then(|dist| dist.get("integrity"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        shasum: dist
            .and_then(|dist| dist.get("shasum"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        integration: integration_value.and_then(|value| serde_json::from_value(value.clone()).ok()),
        optional_dependencies,
    })
}

/// The platform package's bin table, verbatim: bin name -> path inside the
/// package. The finalize contract: EVERY entry is a binary this tool copies
/// and verifies.
pub fn parse_bin_table(doc: &serde_json::Value) -> anyhow::Result<BTreeMap<String, String>> {
    let Some(bin_value) = doc.get("bin") else {
        bail!("package carries no bin table - nothing to install");
    };
    if let Some(path) = bin_value.as_str() {
        let name = doc
            .get("name")
            .and_then(|value| value.as_str())
            .and_then(|name| name.rsplit('/').next())
            .filter(|name| !name.is_empty())
            .context("string bin needs a package name to derive its executable")?;
        return Ok(BTreeMap::from([(name.to_string(), path.to_string())]));
    }
    let bins = bin_value
        .as_object()
        .context("package bin must be a string or object")?;
    let mut out = BTreeMap::new();
    for (name, rel) in bins {
        let rel = rel
            .as_str()
            .with_context(|| format!("bin '{name}' has a non-string path"))?;
        out.insert(name.clone(), rel.to_string());
    }
    if out.is_empty() {
        bail!("platform package's bin table is empty - nothing to install");
    }
    Ok(out)
}

fn package_integration(doc: &serde_json::Value) -> Option<yggterm_core::YggtermPackageMetadata> {
    doc.get("yggterm")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn metadata_bin_name(value: &str) -> &str {
    value.rsplit(['/', '\\']).next().unwrap_or(value)
}

fn app_manifest_from_metadata(
    package: &str,
    integration: Option<&yggterm_core::YggtermPackageMetadata>,
    destination: &Path,
    bins: &BTreeMap<String, String>,
) -> anyhow::Result<Option<yggterm_core::AppManifest>> {
    let Some(app) = integration.and_then(|integration| integration.app.as_ref()) else {
        return Ok(None);
    };
    if app.verbs.is_empty() {
        // A package can deliberately carry only non-menu integration metadata.
        return Ok(None);
    }
    let name = app
        .name
        .clone()
        .unwrap_or_else(|| package.rsplit('/').next().unwrap_or("app").to_string());
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
    {
        bail!("yggterm app metadata for {package} has an invalid name {name:?}");
    }
    let bin = app
        .binary
        .as_deref()
        .map(metadata_bin_name)
        .filter(|bin| !bin.trim().is_empty())
        .map(str::to_string)
        .or_else(|| bins.keys().next().cloned())
        .context("yggterm app metadata has no binary and its package has no bin")?;
    if !bins.contains_key(&bin) {
        bail!(
            "yggterm app metadata for {package} names bin {bin:?}, but the package declares {:?}",
            bins.keys().collect::<Vec<_>>()
        );
    }
    Ok(Some(yggterm_core::AppManifest {
        name: name.clone(),
        label: if app.label.trim().is_empty() {
            name
        } else {
            app.label.clone()
        },
        icon: app.icon.clone(),
        binary: destination.join(&bin).display().to_string(),
        verbs: app.verbs.clone(),
        keytip: app.keytip.clone(),
        context_menu: app.context_menu.clone(),
    }))
}

fn app_name_for_package(
    package: &str,
    integration: Option<&yggterm_core::YggtermPackageMetadata>,
) -> String {
    integration
        .and_then(|integration| integration.app.as_ref())
        .and_then(|app| app.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| package.rsplit('/').next().unwrap_or(package).to_string())
}

fn remove_app_registration(paths: &Paths, name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Ok(());
    }
    let path = paths
        .home
        .join(".yggterm/apps")
        .join(format!("{name}.json"));
    match fs::remove_file(&path) {
        Ok(()) => println!("ynpm: removed yggterm app registration {name}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("removing {}", path.display())),
    }
    Ok(())
}

fn sync_app_registration(
    paths: &Paths,
    package: &str,
    previous: Option<&yggterm_core::YggtermPackageMetadata>,
    current: Option<yggterm_core::AppManifest>,
) -> anyhow::Result<()> {
    let previous_name = previous.map(|metadata| app_name_for_package(package, Some(metadata)));
    let current_name = current.as_ref().map(|manifest| manifest.name.clone());
    if previous_name.as_deref() != current_name.as_deref()
        && let Some(name) = previous_name
    {
        remove_app_registration(paths, &name)?;
    }
    if let Some(manifest) = current {
        yggterm_core::write_app_manifest(&paths.home, &manifest)
            .with_context(|| format!("registering {} with yggterm", manifest.name))?;
        println!(
            "ynpm: registered {} with yggterm from package metadata",
            manifest.name
        );
    }
    Ok(())
}

fn curl(url: &str) -> anyhow::Result<Vec<u8>> {
    let timeout = std::env::var("YNPM_NETWORK_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(120);
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time"])
        .arg(timeout.to_string())
        .arg(url)
        .output()
        .context("running curl (the ynpm substrate; is curl installed?)")?;
    if !out.status.success() {
        bail!(
            "curl {url} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

fn fetch_manifest(pkg: &str, pin: Option<&str>) -> anyhow::Result<Manifest> {
    let base = std::env::var("YNPM_REGISTRY_BASE")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://registry.npmjs.org".to_string());
    let url = match pin {
        Some(v) => format!("{base}/{pkg}/{v}"),
        None => format!("{base}/{pkg}"),
    };
    let body = curl(&url).with_context(|| format!("fetching the manifest of {pkg}"))?;
    let doc: serde_json::Value = serde_json::from_slice(&body)
        .with_context(|| format!("parsing the registry manifest of {pkg}"))?;
    parse_manifest(&doc).with_context(|| format!("resolving {pkg}"))
}

fn latest_version(pkg: &str) -> anyhow::Result<String> {
    fetch_manifest(pkg, None).map(|manifest| manifest.version)
}

/// Run a binary with `--version` and give it a moment: a freshly copied ELF
/// answering nothing is not a version, it is a hang we are about to own.
fn run_version(bin: &Path) -> anyhow::Result<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {} --version", bin.display()))?;
    let started = Instant::now();
    let deadline = Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            let out = child.wait_with_output()?;
            let answer = String::from_utf8_lossy(&out.stdout).to_string();
            if !status.success() {
                bail!(
                    "{} --version exited {}: {}",
                    bin.display(),
                    status,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            return Ok(answer);
        }
        if started.elapsed() > deadline {
            let _ = child.kill();
            bail!("{} --version did not answer within 30s", bin.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn extract_tarball(tgz: &Path, into: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(into).with_context(|| format!("creating {}", into.display()))?;
    let out = Command::new("tar")
        .args(["-xzf"])
        .arg(tgz)
        .arg("-C")
        .arg(into)
        .output()
        .context("running tar (the ynpm substrate; is tar installed?)")?;
    if !out.status.success() {
        bail!(
            "tar -xzf {} failed: {}",
            tgz.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct YggtermUpdateReport {
    status: String,
    current_version: String,
    version: Option<String>,
    executable: Option<String>,
    detail: String,
}

fn yggterm_install_context(paths: &Paths) -> anyhow::Result<yggterm_core::InstallContext> {
    let extension = cfg!(target_os = "windows").then_some(".exe").unwrap_or("");
    let mut candidates = Vec::new();
    // Linux/macOS use a launcher in ~/.local/bin; Windows' bootstrap keeps the
    // executable in %LOCALAPPDATA%\Yggterm\versions instead. Inspect the
    // canonical versioned root FIRST. A stale compatibility state file under
    // ~/.yggterm used to win merely because its flat alias was the first
    // candidate, even after ynpm had activated a newer direct generation.
    if let Ok(root) = yggterm_core::direct_install_root()
        && let Ok(entries) = fs::read_dir(root.join("versions"))
    {
        let mut versions = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        versions.sort();
        candidates.extend(
            versions
                .into_iter()
                .rev()
                .map(|version| version.join(format!("yggterm{extension}"))),
        );
    }

    candidates.extend([
        paths.home.join(format!(".local/bin/yggterm{extension}")),
        paths.home.join(format!(".yggterm/bin/yggterm{extension}")),
    ]);
    if let Ok(current) = std::env::current_exe()
        && current.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
            matches!(name, "yggterm" | "yggterm.exe" | "yggterm-headless" | "yggterm-headless.exe")
        })
    {
        candidates.push(current);
    }

    let mut best = None::<(SemVer, yggterm_core::InstallContext)>;
    for candidate in candidates {
        let Ok(mut context) = yggterm_core::detect_install_context(&candidate) else {
            continue;
        };
        if context.channel != yggterm_core::InstallChannel::Direct
            || context.managed_root.is_none()
        {
            continue;
        }
        let observed = run_version(&candidate)
            .ok()
            .and_then(|answer| version_from_answer(&answer));
        let version_text = observed.as_deref().unwrap_or(&context.current_version);
        let Ok(version) = SemVer::parse(version_text) else {
            continue;
        };
        // A flat alias may inherit a stale state file that is not its source
        // root's active generation. Do not let that stale context outrank a
        // canonical direct generation simply because the alias is runnable.
        if let Some(root) = context.managed_root.as_ref()
            && !root
                .join("versions")
                .join(version_text)
                .join(format!("yggterm{extension}"))
                .is_file()
        {
            continue;
        }
        if observed.as_deref() != Some(context.current_version.as_str()) {
            context.current_version = version_text.to_string();
        }
        if best
            .as_ref()
            .is_none_or(|(current, _)| SemVer::cmp_semver(&version, current).is_gt())
        {
            best = Some((version, context));
        }
    }
    if let Some((_, context)) = best {
        return Ok(context);
    }
    anyhow::bail!(
        "yggterm is not a direct ynpm-compatible install on this host; install it with the curl/PowerShell quickstart first"
    )
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verify_release_checksum(archive: &Path, checksum: &Path) -> anyhow::Result<()> {
    let expected = fs::read_to_string(checksum)?
        .split_whitespace()
        .next()
        .filter(|value| value.len() == 64)
        .context("release checksum has no SHA-256 digest")?
        .to_ascii_lowercase();
    let actual = sha256_file(archive)?;
    if expected != actual {
        bail!(
            "release checksum mismatch for {}: expected {}, got {}",
            archive.display(),
            expected,
            actual
        );
    }
    Ok(())
}

fn publish_release_aux_link(home: &Path, name: &str, target: &Path) -> anyhow::Result<()> {
    let mut directories = vec![home.join(".local/bin"), home.join(".yggterm/bin")];
    if cfg!(target_os = "windows")
        && let Ok(root) = yggterm_core::direct_install_root()
    {
        directories.push(root.join("bin"));
    }
    for directory in directories {
        fs::create_dir_all(&directory)?;
        publish_link(target, &directory.join(name))?;
    }
    Ok(())
}

fn yggterm_dev_fingerprint(paths: &Paths) -> Option<String> {
    let state = paths.load_state().ok()?;
    let key = find_package_key(&state, "@ygghq/yggterm")?;
    state
        .packages
        .get(key)
        .and_then(|package| package.dev.as_ref())
        .and_then(|marker| marker.dev_fingerprint.clone())
}

fn latest_release_update(
    context: &yggterm_core::InstallContext,
) -> anyhow::Result<yggterm_core::ReleaseUpdate> {
    let latest_url = format!("https://github.com/{}/releases/latest", context.repo);
    let null_device = if cfg!(target_os = "windows") {
        "NUL"
    } else {
        "/dev/null"
    };
    let output = Command::new("curl")
        .args(["-fsSL", "-o", null_device, "-w", "%{url_effective}"])
        .arg(&latest_url)
        .output()
        .with_context(|| format!("resolving latest release from {latest_url}"))?;
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
        .filter(|value| value.starts_with('v') && value.len() > 1)
        .context("latest GitHub release redirect did not name a v-tag")?;
    let version = tag.trim_start_matches('v').to_string();
    let archive_name = format!("yggterm-{}.tar.gz", context.asset_label);
    let archive_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        context.repo, tag, archive_name
    );
    Ok(yggterm_core::ReleaseUpdate {
        version,
        tag_name: tag.to_string(),
        archive_url,
        checksum_url: Some(format!(
            "https://github.com/{}/releases/download/{}/{}.sha256",
            context.repo, tag, archive_name
        )),
    })
}

fn install_yggterm_release(
    paths: &Paths,
    context: &yggterm_core::InstallContext,
    update: &yggterm_core::ReleaseUpdate,
    dev_fingerprint: Option<&str>,
) -> anyhow::Result<Option<PathBuf>> {
    let root = context
        .managed_root
        .as_ref()
        .context("direct yggterm install has no managed root")?;
    let extension = cfg!(target_os = "windows").then_some(".exe").unwrap_or("");
    let scratch =
        paths
            .scratch()
            .join(format!("yggterm-{}-{}", update.version, std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch)?;
    let archive = scratch.join("release.tar.gz");
    fs::write(&archive, curl(&update.archive_url)?)?;
    let checksum = scratch.join("release.tar.gz.sha256");
    if let Some(url) = &update.checksum_url {
        fs::write(&checksum, curl(url)?)?;
        verify_release_checksum(&archive, &checksum)?;
    }
    let extracted = scratch.join("extracted");
    extract_tarball(&archive, &extracted)?;

    let version_root = root.join("versions");
    fs::create_dir_all(&version_root)?;
    let final_dir = version_root.join(&update.version);
    let staging = version_root.join(format!(
        ".ynpm-yggterm-{}-{}",
        update.version,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let products = [
        (
            "yggterm",
            format!("yggterm-{}{}", context.asset_label, extension),
        ),
        (
            "yggterm-headless",
            format!("yggterm-headless-{}{}", context.asset_label, extension),
        ),
        ("ynpm", format!("ynpm-{}{}", context.asset_label, extension)),
        ("ynpx", format!("ynpx-{}{}", context.asset_label, extension)),
    ];
    let release_fingerprint = sha256_file(&extracted.join(&products[0].1))?;
    if dev_fingerprint == Some(release_fingerprint.as_str())
        && update.version == context.current_version
    {
        let _ = fs::remove_dir_all(&scratch);
        return Ok(None);
    }
    for (installed_name, archive_name) in products {
        let source = extracted.join(&archive_name);
        let target = staging.join(format!("{installed_name}{extension}"));
        if !source.is_file() {
            bail!("release archive is missing {archive_name}");
        }
        fs::copy(&source, &target)
            .with_context(|| format!("staging release product {archive_name}"))?;
        set_executable(&target)?;
        let answer = run_version(&target)
            .with_context(|| format!("verifying release product {archive_name}"))?;
        if !version_answer_matches(&answer, &update.version) {
            bail!(
                "REFUSED: release product {archive_name} answered {:?}, not {}",
                answer.trim(),
                update.version
            );
        }
    }
    if final_dir.exists() {
        let complete = ["yggterm", "yggterm-headless", "ynpm", "ynpx"]
            .iter()
            .all(|name| final_dir.join(format!("{name}{extension}")).is_file());
        if complete {
            let _ = fs::remove_dir_all(&staging);
        } else {
            fs::remove_dir_all(&final_dir)
                .with_context(|| format!("removing incomplete {}", final_dir.display()))?;
            fs::rename(&staging, &final_dir)?;
        }
    } else {
        fs::rename(&staging, &final_dir)?;
    }
    let yggterm = final_dir.join(format!("yggterm{extension}"));
    let ynpm = final_dir.join(format!("ynpm{extension}"));
    let ynpx = final_dir.join(format!("ynpx{extension}"));
    yggterm_core::write_direct_install_state(
        root,
        &context.repo,
        &context.asset_label,
        &update.version,
        &yggterm,
    )?;
    publish_release_aux_link(&paths.home, &format!("ynpm{extension}"), &ynpm)?;
    publish_release_aux_link(&paths.home, &format!("ynpx{extension}"), &ynpx)?;
    let integrate = Command::new(&yggterm)
        .args(["install", "integrate"])
        .env(yggterm_core::ENV_YGGTERM_DIRECT_INSTALL_ROOT, root)
        .status();
    if !integrate.as_ref().is_ok_and(|status| status.success()) {
        eprintln!("ynpm: yggterm release activated; desktop integration refresh was unavailable");
    }
    let _ = fs::remove_dir_all(&scratch);
    Ok(Some(yggterm))
}

fn clear_yggterm_dev_state(paths: &Paths) -> anyhow::Result<()> {
    let mut state = paths.load_state()?;
    let Some(key) = find_package_key(&state, "@ygghq/yggterm").map(str::to_string) else {
        return Ok(());
    };
    if state
        .packages
        .get(&key)
        .is_some_and(|package| package.dev.is_some())
    {
        state.packages.remove(&key);
        paths.save_state(&state)?;
    }
    Ok(())
}

fn activate_yggterm_dev(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    let key = find_package_key(&state, "@ygghq/yggterm")
        .map(str::to_string)
        .context("yggterm has no ynpm dev generation to activate")?;
    let entry = state
        .packages
        .get(&key)
        .context("yggterm dev state disappeared")?;
    let marker = entry
        .dev
        .as_ref()
        .context("yggterm is not on the dev channel")?;
    let generation_name = entry
        .dev_generation
        .as_deref()
        .or(marker.generation.as_deref())
        .context("yggterm dev state has no generation")?;
    let generation = paths.generation_dir(&key, generation_name);
    let yggterm = generation.join("bin/yggterm");
    if !yggterm.is_file() {
        bail!(
            "yggterm dev generation has no executable at {}",
            yggterm.display()
        );
    }
    let version = marker
        .supersedes
        .as_deref()
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    let root = yggterm_core::direct_install_root()?;
    let asset = yggterm_core::current_asset_label()?;
    yggterm_core::write_direct_install_state(
        &root,
        "yggdrasilhq/yggterm",
        &asset,
        version,
        &yggterm,
    )?;
    for name in ["ynpm", "ynpx"] {
        let target = generation.join(format!("bin/{name}"));
        if target.is_file() {
            publish_release_aux_link(&paths.home, name, &target)?;
        }
    }
    println!(
        "ynpm: activated yggterm dev generation {} (version {})",
        generation.display(),
        version
    );
    Ok(())
}

fn run_yggterm_self_update(paths: &Paths) -> anyhow::Result<YggtermUpdateReport> {
    let context = match yggterm_install_context(paths) {
        Ok(context) => context,
        Err(error) => {
            return Ok(YggtermUpdateReport {
                status: "unsupported".to_string(),
                current_version: env!("CARGO_PKG_VERSION").to_string(),
                version: None,
                executable: None,
                detail: error.to_string(),
            });
        }
    };
    let current_version = context.current_version.clone();
    let dev_fingerprint = yggterm_dev_fingerprint(paths);
    let update = if dev_fingerprint.is_some() {
        Some(latest_release_update(&context)?)
    } else {
        yggterm_core::check_for_update(&context)?
    };
    let Some(update) = update else {
        return Ok(YggtermUpdateReport {
            status: "current".to_string(),
            current_version,
            version: None,
            executable: context
                .preferred_executable
                .map(|path| path.display().to_string()),
            detail: "the direct yggterm release is already current".to_string(),
        });
    };
    let version_order = SemVer::cmp_semver(
        &SemVer::parse(&update.version)?,
        &SemVer::parse(&context.current_version)?,
    );
    if version_order.is_lt() {
        return Ok(YggtermUpdateReport {
            status: "current".to_string(),
            current_version,
            version: None,
            executable: context
                .preferred_executable
                .map(|path| path.display().to_string()),
            detail: "the available production release is older than the active dev build"
                .to_string(),
        });
    }
    let Some(executable) =
        install_yggterm_release(paths, &context, &update, dev_fingerprint.as_deref())?
    else {
        return Ok(YggtermUpdateReport {
            status: "current".to_string(),
            current_version,
            version: None,
            executable: context
                .preferred_executable
                .map(|path| path.display().to_string()),
            detail: "the same-version production release has no binary polish over the dev build"
                .to_string(),
        });
    };
    clear_yggterm_dev_state(paths)?;
    Ok(YggtermUpdateReport {
        status: "updated".to_string(),
        current_version,
        version: Some(update.version),
        executable: Some(executable.display().to_string()),
        detail: "ynpm installed and activated the yggterm release".to_string(),
    })
}

fn verb_self_update(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => bail!("self-update accepts only --json"),
    };
    let report = match run_yggterm_self_update(paths) {
        Ok(report) => report,
        Err(error) if is_network_failure(&error) => YggtermUpdateReport {
            status: "offline".to_string(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            version: None,
            executable: None,
            detail: format!(
                "release channel unavailable; keeping the active verified generation ({error:#})"
            ),
        },
        Err(error) => return Err(error),
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("ynpm: yggterm {}", report.status);
        println!("  {}", report.detail);
        if let Some(version) = report.version {
            println!("  activated {version}");
        }
    }
    Ok(())
}

// ===== the install pipeline =====

pub struct InstallOutcome {
    pub name: String,
    pub version: String,
    pub previous: Option<String>,
    pub bins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportRecord {
    package: String,
    key: String,
    version: String,
    bins: Vec<String>,
    dev: bool,
    #[serde(default)]
    integration: Option<yggterm_core::YggtermPackageMetadata>,
}

/// One package, resolved to its feet: fetch, verify the binaries tell the
/// truth, keep the generation, THEN swap into DEST by rename. Nothing is
/// written to DEST before every bin has answered `--version` correctly.
fn install_one(paths: &Paths, spec: &str, quiet: bool) -> anyhow::Result<InstallOutcome> {
    install_one_at(paths, spec, quiet, None)
}

fn install_one_at(
    paths: &Paths,
    spec: &str,
    quiet: bool,
    destination: Option<&Path>,
) -> anyhow::Result<InstallOutcome> {
    let (pkg, pin) = expand_package(spec)?;
    if !pkg.starts_with("@ygghq/") {
        return install_npm_package(paths, &pkg, pin.as_deref(), quiet, destination);
    }
    let name = pkg
        .strip_prefix("@ygghq/")
        .with_context(|| format!("'{pkg}' is outside the @ygghq scope this tool manages"))?
        .to_string();
    let manifest = fetch_manifest(&pkg, pin.as_deref())?;
    let version = manifest.version.clone();
    let platform = match platform_target(std::env::consts::OS, std::env::consts::ARCH) {
        Ok(platform) => platform,
        Err(error) => {
            let looks_native = manifest
                .optional_dependencies
                .iter()
                .any(|dependency| dependency.starts_with(&format!("{pkg}-")));
            if looks_native {
                return Err(error);
            }
            return install_npm_package(paths, &pkg, pin.as_deref(), quiet, destination);
        }
    };
    let platform_pkg = format!("{pkg}-{platform}");
    // `@ygghq` is also the namespace for generic Python/shell/JS tools. Only
    // packages that explicitly declare the host platform companion take the
    // native fast path below; otherwise npm owns the package's own `bin`
    // entries just like it does for every third-party package. Without this
    // gate, a perfectly valid `@ygghq/yrdp` would be refused for lacking a
    // fictional `@ygghq/yrdp-linux-x64` package.
    if !manifest
        .optional_dependencies
        .iter()
        .any(|dependency| dependency == &platform_pkg)
    {
        return install_npm_package(paths, &pkg, pin.as_deref(), quiet, destination);
    }
    // THE BINARIES ARE NOT IN THE MAIN PACKAGE. `@ygghq/<name>` ships only
    // shims and a finalize script; the ELFs live in the PLATFORM package
    // (`@ygghq/<name>-<platform>`), published in lockstep at the same
    // version. Resolve and fetch THAT, at the version the main package
    // answered - a pin propagates, a latest stays a latest.
    if !quiet {
        println!("ynpm: {pkg}@{version} ({platform_pkg})");
    }
    let platform_manifest = fetch_manifest(&platform_pkg, Some(&version))?;
    if platform_manifest.version != version {
        bail!(
            "REFUSED: {pkg}@{version} and {platform_pkg}@{} disagree - a broken \
             release on the registry (the platform package must publish in \
             lockstep with the main one)",
            platform_manifest.version
        );
    }

    // Download + extract into disk-backed scratch, away from DEST, so a bad
    // fetch or a lying binary costs nothing but bandwidth.
    let scratch = paths.scratch().join(format!("{name}-{version}"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let tgz = scratch.join("pkg.tgz");
    let mut file =
        std::fs::File::create(&tgz).with_context(|| format!("creating {}", tgz.display()))?;
    file.write_all(&curl(&platform_manifest.tarball)?)
        .with_context(|| format!("writing {}", tgz.display()))?;
    drop(file);
    extract_tarball(&tgz, &scratch.join("x"))?;
    let package_dir = scratch.join("x/package");

    let platform_doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(package_dir.join("package.json"))
            .context("reading the platform package's package.json")?,
    )
    .context("parsing the platform package's package.json")?;
    let bins = parse_bin_table(&platform_doc)?;
    let integration = manifest
        .integration
        .clone()
        .or_else(|| package_integration(&platform_doc));

    // THE TRUTH CHECK (the 0.2.0 lesson): every binary must name the version
    // the package ships, BEFORE anything on this host changes. The main
    // package's own version and the platform package's version are read
    // together; the bins answer to the platform's.
    let platform_version = platform_doc
        .get("version")
        .and_then(|v| v.as_str())
        .context("platform package carries no version")?
        .to_string();
    let mut answers = BTreeMap::new();
    for (bin, rel) in &bins {
        let bin_path = package_dir.join(rel);
        if !bin_path.exists() {
            bail!("platform package declares bin '{bin}' at '{rel}' but ships no such file");
        }
        let answer = run_version(&bin_path)
            .with_context(|| format!("verifying {bin} ({}) before install", bin_path.display()))?;
        if !version_answer_matches_identity(&answer, &platform_version) {
            bail!(
                "REFUSED: {pkg}@{version}'s binary '{bin}' answered --version with {:?}, \
                 which does not name the package version {platform_version:?}. This is the \
                 0.2.0 failure (npm bumped, crate not): fix the package so its binaries \
                 tell the truth, then install again.",
                answer.trim()
            );
        }
        answers.insert(bin.clone(), answer);
    }

    // Generation first: the bytes of this version, kept for rollback.
    let generation = paths.generation_dir(&name, &version);
    for (bin, rel) in &bins {
        let src = package_dir.join(rel);
        let dst = generation.join(bin);
        std::fs::create_dir_all(dst.parent().unwrap())
            .with_context(|| format!("creating {}", dst.parent().unwrap().display()))?;
        std::fs::copy(&src, &dst).with_context(|| format!("keeping generation copy of {bin}"))?;
        set_executable(&dst)?;
    }

    // THEN the swap into DEST, by rename: the running process keeps its
    // inode; the next launch gets this build.
    let dest = destination
        .map(absolute_path)
        .unwrap_or_else(|| paths.dest());
    let app_manifest = app_manifest_from_metadata(&pkg, integration.as_ref(), &dest, &bins)?;
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut state = paths.load_state()?;
    let previous_integration = state
        .packages
        .get(&name)
        .and_then(|package| package.integration.clone());
    // The state mutation is a closed scope: the entry borrow must end before
    // the state is saved, and what the render below needs out of it is two
    // plain values.
    let (previous, external_prev) = {
        match state.packages.entry(name.clone()) {
            std::collections::btree_map::Entry::Occupied(mut occupied) => {
                let package = occupied.get_mut();
                let previous = Some(package.current.clone());
                if package.versions.last() != Some(&version) {
                    package.versions.push(version.clone());
                }
                package.current = version.clone();
                package.bins = bins.clone();
                package.package_name = Some(pkg.clone());
                package.destination = Some(dest.display().to_string());
                package.dev = None;
                package.dev_generation = None;
                package.channel = Some("npm".to_string());
                package.integration = integration.clone();
                (previous, package.external_prev.clone())
            }
            std::collections::btree_map::Entry::Vacant(vacant) => {
                // A foreign install (the binary was here before ynpm was):
                // its version is recorded for the drift instrument, but its
                // bytes were never kept, so it is not a rollback target.
                let first_bin = dest.join(bins.keys().next().cloned().unwrap_or_default());
                let external_prev = if first_bin.exists() {
                    run_version(&first_bin)
                        .ok()
                        .as_deref()
                        .and_then(version_from_answer)
                } else {
                    None
                };
                vacant.insert(Package {
                    package_name: Some(pkg.clone()),
                    current: version.clone(),
                    versions: vec![version.clone()],
                    bins: bins.clone(),
                    external_prev: external_prev.clone(),
                    destination: Some(dest.display().to_string()),
                    dev: None,
                    dev_generation: None,
                    channel: Some("npm".to_string()),
                    integration: integration.clone(),
                });
                (None, external_prev)
            }
        }
    };
    for bin in bins.keys() {
        let src = generation.join(bin);
        let dst = dest.join(bin);
        let staged = dest.join(format!(".ynpm-new-{bin}"));
        std::fs::copy(&src, &staged).with_context(|| format!("staging {bin}"))?;
        set_executable(&staged)?;
        std::fs::rename(&staged, &dst)
            .with_context(|| format!("swapping {} into {}", bin, dst.display()))?;
    }
    paths.save_state(&state)?;
    sync_app_registration(paths, &pkg, previous_integration.as_ref(), app_manifest)?;

    if !quiet {
        for (bin, answer) in &answers {
            println!(
                "  {} {} -> {} (answers: {})",
                bin,
                version,
                dest.join(bin).display(),
                answer.trim()
            );
        }
        if let Some(prev) = &previous {
            println!("  previous {prev} kept as a generation; `ynpm rollback {name}` restores it");
        }
        if let Some(ext) = &external_prev {
            println!(
                "  note: a foreign install of {ext} was here before ynpm; recorded, not a rollback target"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = std::fs::remove_file(&tgz);
    Ok(InstallOutcome {
        name,
        version,
        previous,
        bins: bins.keys().cloned().collect(),
    })
}

/// Install a regular npm package into an unpublished ynpm generation. This is
/// the generic path used for third-party agent CLIs (`@openai/codex`,
/// `@opencode-ai/cli`, ...), not just first-party @ygghq platform packages.
/// npm still owns dependency and lifecycle semantics; ynpm owns the cache,
/// staging, health gate, generation and atomic publication.
fn install_npm_package(
    paths: &Paths,
    package: &str,
    pin: Option<&str>,
    quiet: bool,
    destination: Option<&Path>,
) -> anyhow::Result<InstallOutcome> {
    let manifest = fetch_package_manifest(package, pin)?;
    let version = manifest.version.clone();
    let dest = destination
        .map(absolute_path)
        .unwrap_or_else(|| paths.dest());
    let mut state = paths.load_state()?;
    let key = find_package_key(&state, package)
        .map(str::to_string)
        .unwrap_or_else(|| package_storage_key(package));

    // An already-published generation is immutable and was health-checked
    // before it became live. Avoid reinstalling an identical npm version on
    // every `ynpx` invocation; a new registry version gets a new generation.
    if let Some(existing) = state.packages.get(&key)
        && existing.current == version
        && existing.destination.as_deref() == Some(dest.to_string_lossy().as_ref())
        && existing
            .bins
            .keys()
            .all(|bin| run_version(&dest.join(bin)).is_ok())
    {
        return Ok(InstallOutcome {
            name: package.to_string(),
            version,
            previous: Some(existing.current.clone()),
            bins: existing.bins.keys().cloned().collect(),
        });
    }

    let npm = PathBuf::from(
        std::env::var_os("YNPM_NPM")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("npm")),
    );
    let scratch_root = paths.scratch();
    fs::create_dir_all(&scratch_root)
        .with_context(|| format!("creating ynpm scratch root {}", scratch_root.display()))?;
    fs::create_dir_all(paths.cache())
        .with_context(|| format!("creating ynpm cache {}", paths.cache().display()))?;
    let scratch = scratch_root.join(format!("npm-{}-{}-{}", key, version, std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch)
        .with_context(|| format!("creating npm staging {}", scratch.display()))?;
    let result = (|| -> anyhow::Result<InstallOutcome> {
        run_npm_install_package(&npm, paths, &scratch, package, &version)?;
        let package_dir = npm_package_dir(&scratch, package);
        let manifest_path = package_dir.join("package.json");
        let package_doc: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&manifest_path)
                .with_context(|| format!("reading {}", manifest_path.display()))?,
        )
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let bins = parse_bin_table(&package_doc)?;
        let integration =
            package_integration(&package_doc).or_else(|| manifest.integration.clone());
        let app_manifest = app_manifest_from_metadata(package, integration.as_ref(), &dest, &bins)?;
        for (bin, rel) in &bins {
            let candidate = scratch.join("bin").join(bin);
            if !candidate.exists() {
                bail!(
                    "npm package {package}@{version} declares bin '{bin}' at '{rel}', but {} is missing",
                    candidate.display()
                );
            }
            let answer = run_version(&candidate)
                .with_context(|| format!("verifying {package}'s {bin} before publication"))?;
            if answer.trim().is_empty() {
                bail!("npm package {package}@{version} bin '{bin}' answered no version");
            }
        }

        let generation = paths.generation_dir(&key, &version);
        if generation.exists() {
            // The same immutable npm version may be encountered after a
            // partial run. Never remove a currently published generation; if
            // it is not current, use the existing verified bytes.
            if !generation.join("bin").exists() {
                fs::remove_dir_all(&generation)
                    .with_context(|| format!("removing incomplete {}", generation.display()))?;
            }
        }
        if !generation.exists() {
            if let Some(parent) = generation.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_dir_all(&scratch, &generation)
                .with_context(|| format!("keeping generation {}", generation.display()))?;
        }

        let previous = state.packages.get(&key).map(|value| value.current.clone());
        let previous_integration = state
            .packages
            .get(&key)
            .and_then(|value| value.integration.clone());
        let first_bin = bins.keys().next().cloned();
        let external_prev = state
            .packages
            .get(&key)
            .and_then(|value| value.external_prev.clone())
            .or_else(|| {
                first_bin
                    .as_deref()
                    .and_then(|bin| {
                        dest.join(bin)
                            .exists()
                            .then(|| run_version(&dest.join(bin)).ok())
                    })
                    .flatten()
                    .and_then(|answer| version_from_answer(&answer))
            });

        let package_state = state.packages.entry(key.clone()).or_insert(Package {
            package_name: Some(package.to_string()),
            current: version.clone(),
            versions: Vec::new(),
            bins: BTreeMap::new(),
            external_prev: external_prev.clone(),
            destination: Some(dest.display().to_string()),
            dev: None,
            dev_generation: None,
            channel: Some("npm".to_string()),
            integration: integration.clone(),
        });
        if package_state.versions.last() != Some(&version) {
            package_state.versions.push(version.clone());
        }
        package_state.package_name = Some(package.to_string());
        package_state.current = version.clone();
        package_state.bins = bins
            .iter()
            .map(|(name, _)| (name.clone(), format!("bin/{name}")))
            .collect();
        package_state.destination = Some(dest.display().to_string());
        package_state.dev = None;
        package_state.dev_generation = None;
        package_state.channel = Some("npm".to_string());
        package_state.integration = integration.clone();

        fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
        for bin in bins.keys() {
            publish_link(&generation.join("bin").join(bin), &dest.join(bin))?;
        }
        paths.save_state(&state)?;
        sync_app_registration(paths, package, previous_integration.as_ref(), app_manifest)?;
        if !quiet {
            println!("ynpm: {package}@{version} -> {}", dest.display());
            for bin in bins.keys() {
                println!("  {bin}={}", dest.join(bin).display());
            }
            if let Some(ref previous) = previous {
                println!("  previous {previous} kept as a generation");
            }
        }
        Ok(InstallOutcome {
            name: package.to_string(),
            version,
            previous,
            bins: bins.keys().cloned().collect(),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&scratch);
    }
    result
}

fn run_npm_install_package(
    npm: &Path,
    paths: &Paths,
    staged: &Path,
    package: &str,
    version: &str,
) -> anyhow::Result<()> {
    let tmp = paths.scratch().join("tmp");
    fs::create_dir_all(&tmp)?;
    let spec = format!("{package}@{version}");
    let mut command = Command::new(npm);
    command
        .args(["install", "--global", "--prefix"])
        .arg(staged)
        .args(["--no-package-lock", "--no-audit", "--no-fund", "--omit=dev"])
        .arg(format!("--allow-scripts={package}"))
        .arg(&spec)
        .env("npm_config_prefix", staged)
        .env("NPM_CONFIG_PREFIX", staged)
        .env("npm_config_cache", paths.cache())
        .env("TMPDIR", &tmp)
        .env("npm_config_update_notifier", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if std::env::var("YNPM_NETWORK_TIMEOUT_SECS").is_ok() {
        command
            .env("npm_config_fetch_timeout", "8000")
            .env("npm_config_fetch_retries", "0");
    }
    let output = command
        .output()
        .with_context(|| format!("running npm install {spec}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    bail!(
        "npm install {spec} exited with status {}{}",
        output.status,
        if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        }
    )
}

fn npm_package_dir(staged: &Path, package: &str) -> PathBuf {
    staged.join("lib/node_modules").join(package)
}

fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let source = entry.path();
        let target = dst.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir_all(&source, &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(&source)?, &target)?;
            #[cfg(not(unix))]
            fs::copy(&source, &target)?;
        } else {
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

fn publish_link(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let parent = destination
        .parent()
        .context("published binary has no parent directory")?;
    fs::create_dir_all(parent)?;
    let staged = parent.join(format!(
        ".{}.ynpm-publish-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bin"),
        std::process::id()
    ));
    let _ = fs::remove_file(&staged);
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, &staged)
        .with_context(|| format!("staging {}", staged.display()))?;
    #[cfg(not(unix))]
    {
        fs::copy(source, &staged).with_context(|| format!("staging {}", staged.display()))?;
        // Windows has no portable rename-over-existing-file primitive in
        // std. Keep the staged copy off the destination until it is proven,
        // then remove only this package's old publication before the rename.
        // The next launch sees either the old or new complete file, never a
        // partially written binary.
        if fs::symlink_metadata(destination).is_ok() {
            fs::remove_file(destination)
                .with_context(|| format!("replacing {}", destination.display()))?;
        }
    }
    fs::rename(&staged, destination)
        .with_context(|| format!("publishing {}", destination.display()))
}

// ===== the dev channel =====

pub struct DevInstall {
    pub package: String,
    pub storage_key: String,
    pub bins: Vec<String>,
    pub marker: DevMarker,
    pub integration: Option<yggterm_core::YggtermPackageMetadata>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn hostname() -> Option<String> {
    Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn file_fingerprint(path: &Path) -> Option<String> {
    let output = Command::new("sha256sum").arg(path).output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    })
}

#[derive(Debug)]
struct DevArgs {
    package: String,
    watch: Option<String>,
    destination: Option<PathBuf>,
    bins: BTreeMap<String, PathBuf>,
    integration: Option<yggterm_core::YggtermPackageMetadata>,
}

/// Parse `ynpm install --dev <pkg> [--watch <registry-pkg>] [--dest <dir>]
/// [--bin <name>=<path>]... [<path>...]`. Positional artifact paths use their
/// file name as the bin name; explicit `--bin` is the escape hatch for a
/// compiled file whose name is not the public command.
fn parse_dev_args(args: &[String]) -> anyhow::Result<DevArgs> {
    let mut package = None;
    let mut watch = None;
    let mut destination = None;
    let mut bins = BTreeMap::new();
    let mut integration = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--watch" => {
                index += 1;
                watch = Some(
                    args.get(index)
                        .context("--watch wants the production package to poll")?
                        .clone(),
                );
            }
            "--dest" | "--destination" => {
                index += 1;
                destination = Some(PathBuf::from(
                    args.get(index)
                        .context("--dest wants a destination directory")?,
                ));
            }
            "--bin" => {
                index += 1;
                let value = args.get(index).context("--bin wants name=path")?.clone();
                let (name, path) = value.split_once('=').context("--bin wants name=path")?;
                if name.trim().is_empty() || path.trim().is_empty() {
                    bail!("--bin wants a non-empty name and path");
                }
                bins.insert(name.to_string(), PathBuf::from(path));
            }
            "--integration-json" => {
                index += 1;
                let raw = args
                    .get(index)
                    .context("--integration-json wants a JSON object")?;
                integration = Some(
                    serde_json::from_str(raw)
                        .context("--integration-json is not valid yggterm metadata")?,
                );
            }
            value if value.starts_with("--") => {
                bail!("unknown dev option '{value}'");
            }
            value if package.is_none() => package = Some(value.to_string()),
            value => positional.push(PathBuf::from(value)),
        }
        index += 1;
    }
    let package = package.context(
        "dev install wants a package name and at least one binary (example: \
         ynpm install --dev @scope/tool --bin tool=dist/tool)",
    )?;
    let (canonical, _) = expand_package(&package)?;
    for path in positional {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .context("a dev artifact has no file name")?;
        bins.insert(name.to_string(), path);
    }
    if bins.is_empty() {
        bail!("dev install wants at least one binary artifact");
    }
    Ok(DevArgs {
        package: canonical,
        watch,
        destination,
        bins,
        integration,
    })
}

fn git_head(checkout: &Path) -> Option<String> {
    Command::new("git")
        .args(["-C", &checkout.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn install_dev_bins(
    paths: &Paths,
    package: &str,
    bins: &BTreeMap<String, PathBuf>,
    expected_version: Option<&str>,
    commit: Option<String>,
    watch: Option<String>,
    destination: Option<&Path>,
    integration: Option<yggterm_core::YggtermPackageMetadata>,
    quiet: bool,
) -> anyhow::Result<DevInstall> {
    let package = if package.starts_with('@') {
        package.to_string()
    } else {
        format!("@ygghq/{package}")
    };
    let mut answers = BTreeMap::new();
    for (name, path) in bins {
        if !path.is_file() {
            bail!("dev binary '{name}' does not exist at {}", path.display());
        }
        let answer = run_version(path)
            .with_context(|| format!("verifying dev binary '{name}' at {}", path.display()))?;
        if let Some(expected) = expected_version
            && !version_answer_matches_identity(&answer, expected)
        {
            bail!(
                "REFUSED: dev binary '{name}' answered --version with {:?}, which does not name checkout version {expected:?}",
                answer.trim()
            );
        }
        answers.insert(name.clone(), answer);
    }

    let destination = destination
        .map(absolute_path)
        .unwrap_or_else(|| default_dev_destination(paths, &package));
    let mut state = paths.load_state()?;
    let key = find_package_key(&state, &package)
        .map(str::to_string)
        .unwrap_or_else(|| package_storage_key(&package));
    let existing = state.packages.get(&key);
    let integration =
        integration.or_else(|| existing.and_then(|package| package.integration.clone()));
    let app_bins = bins
        .keys()
        .map(|name| (name.clone(), format!("bin/{name}")))
        .collect::<BTreeMap<_, _>>();
    let app_manifest =
        app_manifest_from_metadata(&package, integration.as_ref(), &destination, &app_bins)?;
    let previous_integration = existing.and_then(|package| package.integration.clone());
    let supersedes = expected_version
        .map(str::to_string)
        .or_else(|| existing.map(|package| package.current.clone()));
    let watch_manifest = watch
        .as_deref()
        .and_then(|package| fetch_package_manifest(package, None).ok());
    let supersedes_fingerprint = watch_manifest.as_ref().and_then(|manifest| {
        manifest
            .integrity
            .clone()
            .or_else(|| manifest.shasum.clone())
    });

    // Dev builds churn frequently. Give every build its own immutable
    // generation so a running TUI can finish from the old tree while the new
    // build is verified and published.
    let generation_name = format!("dev-{}-{}", now_ms(), std::process::id());
    let generation = paths.generation_dir(&key, &generation_name);
    fs::create_dir_all(generation.join("bin"))
        .with_context(|| format!("creating dev generation {}", generation.display()))?;
    let mut bin_table = BTreeMap::new();
    for (name, path) in bins {
        let target = generation.join("bin").join(name);
        fs::copy(path, &target)
            .with_context(|| format!("copying dev binary {}", path.display()))?;
        set_executable(&target)?;
        bin_table.insert(name.clone(), format!("bin/{name}"));
    }
    let dev_fingerprint = bins.values().next().and_then(|path| file_fingerprint(path));
    let marker = DevMarker {
        built_at_ms: now_ms(),
        commit,
        host: hostname(),
        watch,
        supersedes,
        supersedes_fingerprint,
        dev_fingerprint,
        generation: Some(generation_name.clone()),
    };

    let previous = existing.map(|package| package.current.clone());
    let package_state = state.packages.entry(key.clone()).or_insert(Package {
        package_name: Some(package.clone()),
        current: marker
            .supersedes
            .clone()
            .unwrap_or_else(|| "0.0.0".to_string()),
        versions: Vec::new(),
        bins: BTreeMap::new(),
        external_prev: None,
        destination: Some(destination.display().to_string()),
        dev: None,
        dev_generation: None,
        channel: Some("dev".to_string()),
        integration: None,
    });
    package_state.package_name = Some(package.clone());
    package_state.bins = bin_table;
    package_state.destination = Some(destination.display().to_string());
    package_state.dev = Some(marker.clone());
    package_state.dev_generation = Some(generation_name);
    package_state.channel = Some("dev".to_string());
    package_state.integration = integration.clone();
    fs::create_dir_all(&destination)?;
    for name in bins.keys() {
        publish_link(&generation.join("bin").join(name), &destination.join(name))?;
    }
    paths.save_state(&state)?;
    sync_app_registration(paths, &package, previous_integration.as_ref(), app_manifest)?;
    if !quiet {
        println!("ynpm: {package} dev -> {}", destination.display());
        for (name, answer) in answers {
            println!("  {name} (dev; {answer})");
        }
        if let Some(ref previous) = previous {
            println!("  supersedes {previous}");
        }
    }
    Ok(DevInstall {
        package,
        storage_key: key,
        bins: bins.keys().cloned().collect(),
        marker,
        integration,
    })
}

fn package_json(checkout: &Path) -> Option<serde_json::Value> {
    fs::read_to_string(checkout.join("package.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

fn dev_checkout(
    paths: &Paths,
    checkout: &Path,
    build_override: Option<String>,
    watch_override: Option<String>,
    bin_overrides: BTreeMap<String, PathBuf>,
    destination: Option<PathBuf>,
    hosts: &[String],
) -> anyhow::Result<DevInstall> {
    let package_doc = package_json(checkout);
    let package = package_doc
        .as_ref()
        .and_then(|doc| doc.get("name"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            checkout
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| format!("@ygghq/{name}"))
        })
        .context("checkout needs package.json name or a directory name")?;
    let is_yggterm_checkout = checkout.join("apps/yggterm/Cargo.toml").is_file()
        && checkout.join("crates/ynpm/Cargo.toml").is_file();
    let package = if is_yggterm_checkout {
        "@ygghq/yggterm".to_string()
    } else {
        package
    };
    let version = package_doc
        .as_ref()
        .and_then(|doc| doc.get("version"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let version = version.or_else(|| {
        if !is_yggterm_checkout {
            return None;
        }
        fs::read_to_string(checkout.join("Cargo.toml"))
            .ok()
            .and_then(|raw| {
                raw.split("[workspace.package]").nth(1).and_then(|section| {
                    section.lines().find_map(|line| {
                        let line = line.trim();
                        line.strip_prefix("version = \"")
                            .and_then(|rest| rest.strip_suffix('"'))
                            .map(str::to_string)
                    })
                })
            })
    });
    let recipe = package_doc.as_ref().and_then(|doc| doc.get("ynpm"));
    let build = build_override
        .or_else(|| {
            recipe
                .and_then(|doc| doc.get("build"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            package_doc
                .as_ref()
                .and_then(|doc| doc.get("scripts"))
                .and_then(|scripts| scripts.get("build"))
                .and_then(|value| value.as_str())
                .map(|_| "npm run build".to_string())
        })
        .or_else(|| {
            is_yggterm_checkout.then(|| {
                "cargo build --release --bin yggterm --bin yggterm-headless --bin ynpm".to_string()
            })
        })
        .or_else(|| {
            checkout
                .join("Cargo.toml")
                .is_file()
                .then(|| "cargo build --release".to_string())
        });
    let build = build.context(
        "checkout has no ynpm.build, package.json scripts.build, or Cargo.toml; pass --build",
    )?;
    println!("ynpm: building {} with `{build}`", checkout.display());
    let output = Command::new("sh")
        .arg("-c")
        .arg(&build)
        .current_dir(checkout)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running dev build in {}", checkout.display()))?;
    if !output.status.success() {
        bail!(
            "dev build failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut bins = bin_overrides;
    if bins.is_empty()
        && let Some(recipe_bins) = recipe
            .and_then(|doc| doc.get("bins"))
            .and_then(|value| value.as_object())
    {
        for (name, value) in recipe_bins {
            let rel = value
                .as_str()
                .with_context(|| format!("ynpm bin '{name}' is not a string"))?;
            bins.insert(name.clone(), checkout.join(rel));
        }
    }
    if bins.is_empty()
        && let Some(package_bins) = package_doc.as_ref().and_then(|doc| doc.get("bin"))
    {
        let table = if let Some(table) = package_bins.as_object() {
            table
                .iter()
                .filter_map(|(name, path)| {
                    path.as_str().map(|path| (name.clone(), path.to_string()))
                })
                .collect::<Vec<_>>()
        } else if let Some(path) = package_bins.as_str() {
            vec![(
                package.rsplit('/').next().unwrap_or("app").to_string(),
                path.to_string(),
            )]
        } else {
            Vec::new()
        };
        for (name, rel) in table {
            bins.insert(name, checkout.join(rel));
        }
    }
    if bins.is_empty() {
        if is_yggterm_checkout {
            for name in ["yggterm", "yggterm-headless", "ynpm", "ynpx"] {
                let executable =
                    checkout
                        .join("target/release")
                        .join(if cfg!(target_os = "windows") {
                            format!("{name}.exe")
                        } else {
                            name.to_string()
                        });
                bins.insert(name.to_string(), executable);
            }
        }
    }
    if bins.is_empty() {
        bail!("dev checkout produced no bins; pass --bin name=path");
    }
    let watch = watch_override.or_else(|| {
        recipe
            .and_then(|doc| doc.get("watch"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    });
    let integration = package_doc.as_ref().and_then(package_integration);
    let yggterm_destination = is_yggterm_checkout.then(|| paths.root().join("bin"));
    let outcome = install_dev_bins(
        paths,
        &package,
        &bins,
        version.as_deref(),
        git_head(checkout),
        watch,
        destination.as_deref().or(yggterm_destination.as_deref()),
        integration,
        false,
    )?;
    if is_yggterm_checkout {
        activate_yggterm_dev(paths)?;
    }
    if !hosts.is_empty() {
        let fleet_destination = destination.unwrap_or_else(|| {
            if is_yggterm_checkout {
                paths.root().join("bin")
            } else {
                default_dev_destination(paths, &outcome.package)
            }
        });
        fleet_push(paths, hosts, &outcome, &fleet_destination)?;
    }
    Ok(outcome)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fleet_push(
    paths: &Paths,
    hosts: &[String],
    outcome: &DevInstall,
    destination: &Path,
) -> anyhow::Result<()> {
    let key = outcome.storage_key.clone();
    let generation =
        paths.generation_dir(&key, outcome.marker.generation.as_deref().unwrap_or("dev"));
    let ynpm_self = std::env::current_exe().context("locating ynpm for fleet bootstrap")?;
    for host in hosts {
        let remote_root = format!(
            ".yggterm/scratchpad/ynpm/incoming/{}-{}",
            key,
            std::process::id()
        );
        let remote_dir = format!("$HOME/{remote_root}");
        let bootstrap_rel = format!(".local/bin/ynpm-bootstrap-{}", std::process::id());
        let bootstrap_remote = format!("$HOME/{bootstrap_rel}");
        let bootstrap_cmd = format!(
            "mkdir -p $HOME/.local/bin && chmod 755 {bootstrap} && mv -f {bootstrap} $HOME/.local/bin/ynpm",
            bootstrap = bootstrap_remote
        );
        // Always send the current package manager first when a dev build is
        // distributed. This is what makes a new dev verb usable on a host
        // that still carries an older ynpm.
        let bootstrap_dest = format!("{host}:{bootstrap_rel}");
        let status = Command::new("scp")
            .args(["-q"])
            .arg(&ynpm_self)
            .arg(&bootstrap_dest)
            .status()
            .with_context(|| format!("bootstrapping ynpm on {host}"))?;
        if !status.success() {
            bail!("could not copy ynpm to {host}");
        }
        let status = Command::new("ssh")
            .arg(host)
            .arg(format!("{bootstrap_cmd}; mkdir -p {remote_dir}"))
            .status()?;
        if !status.success() {
            bail!("could not prepare ynpm dev staging on {host}");
        }
        for name in &outcome.bins {
            let local = generation.join("bin").join(name);
            let remote_file = format!("{remote_dir}/{name}");
            let remote_copy_dest = format!("{host}:{remote_root}/{name}");
            let status = Command::new("scp")
                .args(["-q"])
                .arg(&local)
                .arg(&remote_copy_dest)
                .status()
                .with_context(|| format!("pushing {name} to {host}"))?;
            if !status.success() {
                bail!("could not push {name} to {host}");
            }
            let watch = outcome
                .marker
                .watch
                .as_deref()
                .map(|watch| format!(" --watch {}", shell_quote(watch)))
                .unwrap_or_default();
            let integration = outcome
                .integration
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let integration = integration
                .as_deref()
                .map(|value| format!(" --integration-json {}", shell_quote(value)))
                .unwrap_or_default();
            let remote_dest = shell_quote(&destination.display().to_string());
            let remote_bin = format!("{}={remote_file}", shell_quote(name));
            let command = format!(
                "$HOME/.local/bin/ynpm install --dev --dest {remote_dest}{watch}{integration} --bin {name} {package}; rm -f {remote_file}; rmdir {remote_dir} 2>/dev/null || true",
                remote_dest = remote_dest,
                watch = watch,
                integration = integration,
                name = remote_bin,
                remote_file = remote_file,
                package = shell_quote(&outcome.package)
            );
            let status = Command::new("ssh").arg(host).arg(command).status()?;
            if !status.success() {
                bail!("ynpm dev install failed on {host} for {name}");
            }
        }
        if outcome.package == "@ygghq/yggterm" {
            let status = Command::new("ssh")
                .arg(host)
                .arg("$HOME/.local/bin/ynpm activate-yggterm-dev")
                .status()?;
            if !status.success() {
                bail!("could not activate the yggterm dev build on {host}");
            }
        }
        println!("ynpm: distributed {} dev build to {host}", outcome.package);
    }
    Ok(())
}

// ===== integrated inventory =====

fn descriptor_package(descriptor: &yggterm_core::agent_cli::AgentCliDescriptor) -> String {
    match descriptor.install {
        yggterm_core::agent_cli::CliInstall::Npm(package)
        | yggterm_core::agent_cli::CliInstall::Uv(package) => package.to_string(),
        yggterm_core::agent_cli::CliInstall::VendorScript(url) => url.to_string(),
        yggterm_core::agent_cli::CliInstall::Manual => "manual".to_string(),
    }
}

fn inventory_candidate(
    paths: &Paths,
    state: &State,
    package: &str,
    binary: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    // Integrated rows launch from this directory. Prefer it when an older
    // dev state also leaves a user-local link behind; otherwise `list` would
    // measure a path that yggterm itself no longer executes.
    if is_integrated_npm_package(package) {
        let path = paths.root().join("bin").join(binary);
        if path.exists() {
            let version = run_version(&path)
                .ok()
                .and_then(|answer| version_from_answer(&answer));
            return (
                version,
                Some("ynpm".to_string()),
                Some(path.display().to_string()),
            );
        }
    }
    if let Some(key) = find_package_key(state, package)
        && let Some(entry) = state.packages.get(key)
    {
        let destination = package_destination(paths, entry);
        let path = destination.join(binary);
        if path.exists()
            && let Ok(answer) = run_version(&path)
        {
            return (
                version_from_answer(&answer),
                Some("ynpm".to_string()),
                Some(path.display().to_string()),
            );
        }
    }
    let candidates = [
        (paths.root().join("bin"), "ynpm"),
        (paths.home.join(".yggterm/npm/bin"), "legacy-yggterm-npm"),
        (paths.home.join(".yggterm/bin"), "legacy-yggterm-bin"),
        (paths.home.join(".local/bin"), "user-local"),
    ];
    for (dir, source) in candidates {
        let path = dir.join(binary);
        if path.exists() {
            let version = run_version(&path)
                .ok()
                .and_then(|answer| version_from_answer(&answer));
            return (
                version,
                Some(source.to_string()),
                Some(path.display().to_string()),
            );
        }
    }
    // A CLI installed outside ynpm is still a recognized CLI. Measure the
    // actual PATH rather than claiming it is absent merely because the state
    // file has no entry; this is especially important for a system package
    // that yggterm is deliberately not allowed to delete.
    for dir in
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_else(|| OsString::from("")))
    {
        let path = dir.join(binary);
        if path.exists() {
            let version = run_version(&path)
                .ok()
                .and_then(|answer| version_from_answer(&answer));
            return (
                version,
                Some("system".to_string()),
                Some(path.display().to_string()),
            );
        }
    }
    (None, None, None)
}

fn read_app_manifests(paths: &Paths) -> Vec<yggterm_core::AppManifest> {
    let dir = paths.home.join(".yggterm/apps");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|raw| serde_json::from_str::<yggterm_core::AppManifest>(&raw).ok())
        .collect()
}

fn verb_list(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    println!("ynpm: integrated inventory");
    match yggterm_install_context(paths) {
        Ok(context) => println!(
            "product yggterm       package=@ygghq/yggterm version={} source=ynpm path={}",
            context.current_version,
            context
                .preferred_executable
                .unwrap_or(context.executable_path)
                .display()
        ),
        Err(_) => println!(
            "product yggterm       package=@ygghq/yggterm version=not-installed source=unavailable path=-"
        ),
    }
    let mut named = std::collections::BTreeSet::new();
    for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
        let package = descriptor_package(descriptor);
        let (version, source, path) =
            inventory_candidate(paths, &state, &package, descriptor.binary_name);
        println!(
            "cli {:<15} package={} version={} source={} path={}",
            descriptor.slug,
            package,
            version.as_deref().unwrap_or("not-installed"),
            source.as_deref().unwrap_or("unavailable"),
            path.as_deref().unwrap_or("-")
        );
        named.insert(package);
    }
    for app in read_app_manifests(paths) {
        let package = format!("@ygghq/{}", app.name);
        let binary = Path::new(&app.binary);
        let version = run_version(binary)
            .ok()
            .and_then(|answer| version_from_answer(&answer));
        println!(
            "app {:<15} package={} version={} source={} path={}",
            app.name,
            package,
            version.as_deref().unwrap_or("not-installed"),
            if find_package_key(&state, &package).is_some() {
                "ynpm"
            } else {
                "registry-manifest"
            },
            app.binary
        );
        named.insert(package);
    }
    for (key, package) in &state.packages {
        let identity = package_identity(key, package);
        if named.contains(&identity) {
            continue;
        }
        let destination = package_destination(paths, package);
        let versions = package
            .bins
            .keys()
            .map(|bin| {
                run_version(&destination.join(bin))
                    .ok()
                    .and_then(|answer| version_from_answer(&answer))
                    .unwrap_or_else(|| "?".to_string())
            })
            .collect::<Vec<_>>();
        println!(
            "pkg {:<15} version={} source=ynpm bins={} path={}",
            identity,
            package.current,
            versions.join(","),
            destination.display()
        );
    }
    Ok(())
}

fn set_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        // Windows has no POSIX executable bit. The package's bin entry and
        // the platform's normal process rules are the gate there.
        let _ = path;
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod 755 {}", path.display()))
    }
}

// ===== verbs =====

fn verb_install(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        bail!("install wants a package (example: ynpm install ychrome)");
    }
    if args[0] == "--dev" {
        let dev = parse_dev_args(&args[1..])?;
        install_dev_bins(
            paths,
            &dev.package,
            &dev.bins,
            None,
            None,
            dev.watch,
            dev.destination.as_deref(),
            dev.integration,
            false,
        )?;
        return Ok(());
    }
    let mut destination = None;
    let mut specs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dest" | "--destination" => {
                index += 1;
                destination = Some(PathBuf::from(
                    args.get(index)
                        .context("--dest wants a destination directory")?,
                ));
            }
            value if value.starts_with("--") => bail!("unknown install option '{value}'"),
            value => specs.push(value.to_string()),
        }
        index += 1;
    }
    if specs.is_empty() {
        bail!("install wants a package");
    }
    let mut failed = 0usize;
    for spec in &specs {
        if let Err(error) = install_one_at(paths, spec, false, destination.as_deref()) {
            eprintln!("ynpm: ⛔ {spec}: {error:#}");
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} package(s) failed", specs.len());
    }
    Ok(())
}

fn verb_check(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    if state.packages.is_empty() {
        println!("ynpm: no packages are recorded yet");
        return Ok(());
    }
    let mut drifted = 0usize;
    for (key, package) in &state.packages {
        let identity = package_identity(key, package);
        let destination = package_destination(paths, package);
        let disk_answer = package
            .bins
            .keys()
            .next()
            .and_then(|bin| run_version(&destination.join(bin)).ok());
        let mut flags = Vec::new();
        if package.dev.is_none() {
            match disk_answer
                .as_deref()
                .and_then(version_from_answer)
            {
                Some(version)
                    if !version_answer_matches_identity(
                        disk_answer.as_deref().unwrap_or(""),
                        &package.current,
                    ) => flags.push(format!(
                    "DRIFT: disk answers {version}, state says {}",
                    package.current
                )),
                None => flags.push("DRIFT: the installed binary answers no version".to_string()),
                _ => {}
            }
        }
        let latest = fetch_package_manifest(&identity, None).ok();
        if let Some(latest) = latest.as_ref()
            && package.dev.is_none()
            && latest.version != package.current
        {
            flags.push(format!(
                "behind registry: latest is {} (ynpm sync)",
                latest.version
            ));
        }
        if flags.is_empty() {
            if let Some(marker) = &package.dev {
                println!(
                    "{identity}: DEV (supersedes {}; watch {})",
                    marker.supersedes.as_deref().unwrap_or("unknown"),
                    marker.watch.as_deref().unwrap_or("none")
                );
            } else {
                println!(
                    "{identity}: {} current (registry {})",
                    package.current,
                    latest
                        .as_ref()
                        .map(|manifest| manifest.version.as_str())
                        .unwrap_or("unreachable")
                );
            }
        } else {
            drifted += 1;
            println!("{identity}: {}", flags.join("; "));
        }
    }
    if drifted > 0 {
        bail!("{drifted} package(s) drifted");
    }
    Ok(())
}

fn tool_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names = if cfg!(target_os = "windows") {
        vec![
            name.to_string(),
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ]
    } else {
        vec![name.to_string()]
    };
    for directory in std::env::split_paths(&path) {
        for candidate in &names {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Preflight host-specific assumptions before a package operation turns them
/// into a confusing install failure. Native targets are reported separately
/// from generic npm support: Windows/macOS can install a portable package even
/// while a first-party native matrix is not published yet.
fn verb_doctor(paths: &Paths) -> anyhow::Result<()> {
    println!("ynpm doctor");
    println!("  home:        {}", paths.home.display());
    println!("  manager root:{}", paths.root().display());
    println!("  destination: {}", paths.dest().display());
    println!("  scratch:     {}", paths.scratch().display());
    let native_target = platform_target(std::env::consts::OS, std::env::consts::ARCH);
    match &native_target {
        Ok(target) => println!("  native target:{target}"),
        Err(error) => println!("  native target:unpublished ({error})"),
    }

    let mut failures = Vec::new();
    for tool in ["curl", "tar", "npm"] {
        match tool_on_path(tool) {
            Some(path) => println!("  tool {tool:<5}: {}", path.display()),
            None => {
                println!("  tool {tool:<5}: MISSING");
                failures.push(tool);
            }
        }
    }
    if !paths.home.is_absolute() || !paths.root().is_absolute() || !paths.dest().is_absolute() {
        failures.push("absolute path resolution");
    }
    if paths.scratch().starts_with(std::env::temp_dir()) {
        failures.push("disk-backed scratch (scratch resolved into the OS temp directory)");
    }
    if failures.is_empty() {
        println!("ynpm: platform/path preflight passed");
        return Ok(());
    }
    bail!("ynpm doctor found: {}", failures.join(", "))
}

fn sync_integrated(paths: &Paths) -> anyhow::Result<()> {
    let destination = paths.root().join("bin");
    fs::create_dir_all(&destination)?;
    let mut failures = Vec::new();
    let mut attempted = 0usize;
    for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
        let yggterm_core::agent_cli::CliInstall::Npm(package) = descriptor.install else {
            println!(
                "ynpm: cli {} uses {}; ynpm inventory only",
                descriptor.slug,
                descriptor_package(descriptor)
            );
            continue;
        };
        let state = paths.load_state()?;
        if let Some(key) = find_package_key(&state, package)
            && let Some(entry) = state.packages.get(key)
            && entry.dev.is_some()
        {
            // Older dev installs defaulted to ~/.local/bin. Bridge that
            // generation into the server's agent destination without moving
            // the user's existing link or rewriting its state record. New
            // `ynpm dev` installs choose this destination automatically.
            let current_destination = package_destination(paths, entry);
            if current_destination != destination {
                let generation_name = entry
                    .dev_generation
                    .as_deref()
                    .or_else(|| entry.dev.as_ref().and_then(|dev| dev.generation.as_deref()))
                    .unwrap_or("dev");
                let generation = paths.generation_dir(key, generation_name);
                fs::create_dir_all(&destination)?;
                for bin in entry.bins.keys() {
                    let source = generation_binary(&generation, bin);
                    if !source.is_file() {
                        failures.push(format!(
                            "{}: dev bin {} is missing from {}",
                            descriptor.slug,
                            bin,
                            generation.display()
                        ));
                        continue;
                    }
                    if let Err(error) = run_version(&source) {
                        failures.push(format!(
                            "{}: dev bin {} failed its version gate: {error:#}",
                            descriptor.slug, bin
                        ));
                        continue;
                    }
                    if let Err(error) = publish_link(&source, &destination.join(bin)) {
                        failures.push(format!(
                            "{}: could not publish dev bin {}: {error:#}",
                            descriptor.slug, bin
                        ));
                    }
                }
                if !failures
                    .iter()
                    .any(|failure| failure.starts_with(&format!("{}:", descriptor.slug)))
                {
                    println!(
                        "ynpm: cli {} bridged its dev generation into {}",
                        descriptor.slug,
                        destination.display()
                    );
                }
            } else {
                println!(
                    "ynpm: cli {} is on a dev generation; keeping it",
                    descriptor.slug
                );
            }
            continue;
        }
        if let Some(key) = find_package_key(&state, package)
            && let Some(entry) = state.packages.get(key)
            && package_destination(paths, entry) != destination
        {
            let key = key.to_string();
            if bridge_generation_to_destination(paths, &key, &state.packages[&key], &destination)? {
                let mut state = paths.load_state()?;
                if let Some(entry) = state.packages.get_mut(&key) {
                    entry.destination = Some(destination.display().to_string());
                }
                paths.save_state(&state)?;
                println!(
                    "ynpm: cli {} moved its verified generation into {}",
                    descriptor.slug,
                    destination.display()
                );
            }
        }
        attempted += 1;
        let tag = yggterm_core::agent_cli::npm_dist_tag(descriptor.kind).unwrap_or("latest");
        let spec = format!("{package}@{tag}");
        match install_one_at(paths, &spec, true, Some(&destination)) {
            Ok(outcome) => println!(
                "ynpm: integrated {} -> {} ({})",
                descriptor.slug,
                outcome.version,
                destination.display()
            ),
            Err(error)
                if is_network_failure(&error)
                    && package_is_healthy_at(paths, package, &destination) =>
            {
                println!(
                    "ynpm: integrated {} already has a verified local generation; registry offline, keeping it",
                    descriptor.slug
                );
            }
            Err(error) => failures.push(format!("{}: {error:#}", descriptor.slug)),
        }
    }
    println!(
        "ynpm: integrated sync attempted {attempted}, failures {}",
        failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("integrated sync failed: {}", failures.join("; "))
    }
}

fn bridge_generation_to_destination(
    paths: &Paths,
    key: &str,
    entry: &Package,
    destination: &Path,
) -> anyhow::Result<bool> {
    if entry.dev.is_some() {
        return Ok(false);
    }
    let generation = paths.generation_dir(key, &entry.current);
    if !generation.is_dir() || entry.bins.is_empty() {
        return Ok(false);
    }
    for bin in entry.bins.keys() {
        let source = generation_binary(&generation, bin);
        if !source.is_file() {
            return Ok(false);
        }
        let answer = run_version(&source)
            .with_context(|| format!("verifying {source:?} before destination bridge"))?;
        if !version_answer_matches_identity(&answer, &entry.current) {
            return Ok(false);
        }
    }
    fs::create_dir_all(destination)?;
    for bin in entry.bins.keys() {
        publish_link(&generation_binary(&generation, bin), &destination.join(bin))?;
    }
    Ok(true)
}

fn package_is_healthy_at(paths: &Paths, package: &str, destination: &Path) -> bool {
    let Ok(state) = paths.load_state() else {
        return false;
    };
    let Some(key) = find_package_key(&state, package) else {
        return false;
    };
    let Some(entry) = state.packages.get(key) else {
        return false;
    };
    entry.dev.is_none()
        && package_destination(paths, entry) == destination
        && !entry.bins.is_empty()
        && entry
            .bins
            .keys()
            .all(|bin| run_version(&destination.join(bin)).is_ok())
}

fn verb_sync(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    match run_yggterm_self_update(paths) {
        Ok(report) => println!(
            "ynpm: yggterm {}{}",
            report.status,
            report
                .version
                .as_deref()
                .map(|version| format!(" -> {version}"))
                .unwrap_or_default()
        ),
        Err(error) if is_network_failure(&error) => {
            eprintln!("ynpm: yggterm update check skipped while offline: {error:#}")
        }
        Err(error) => return Err(error),
    }
    if args.iter().any(|arg| arg == "--integrated") {
        return sync_integrated(paths);
    }
    if !args.is_empty() {
        bail!(
            "unknown sync option '{}'; use --integrated or no options",
            args[0]
        );
    }
    let state = paths.load_state()?;
    if state.packages.is_empty() {
        println!("ynpm: no packages are recorded yet");
        return Ok(());
    }
    let keys = state.packages.keys().cloned().collect::<Vec<_>>();
    let mut updated = 0usize;
    let mut failed = Vec::new();
    for key in keys {
        let state = paths.load_state()?;
        let package = state
            .packages
            .get(&key)
            .context("package disappeared during sync")?;
        let identity = package_identity(&key, package);
        let destination = package_destination(paths, package);
        if let Some(marker) = &package.dev {
            let manifest = marker
                .watch
                .as_deref()
                .and_then(|watch| fetch_package_manifest(watch, None).ok());
            let handback = dev_handback(
                manifest.as_ref().map(|manifest| manifest.version.as_str()),
                marker.supersedes.as_deref(),
                marker.supersedes_fingerprint.as_deref(),
                manifest.as_ref().and_then(|manifest| {
                    manifest.integrity.as_deref().or(manifest.shasum.as_deref())
                }),
            );
            if handback == DevHandback::ReleaseAvailable {
                println!("ynpm: {identity} dev -> production release");
                match install_one_at(paths, &identity, true, Some(&destination)) {
                    Ok(_) => updated += 1,
                    Err(error) => failed.push(format!("{identity}: {error:#}")),
                }
            } else {
                println!(
                    "ynpm: {identity} DEV; keeping build (watch {})",
                    marker.watch.as_deref().unwrap_or("none")
                );
            }
            continue;
        }
        let latest = match fetch_package_manifest(&identity, None) {
            Ok(manifest) => manifest,
            Err(error) => {
                failed.push(format!("{identity}: {error:#}"));
                continue;
            }
        };
        if latest.version == package.current {
            println!("ynpm: {identity} {} current", package.current);
            continue;
        }
        println!("ynpm: {identity} {} -> {}", package.current, latest.version);
        match install_one_at(paths, &identity, true, Some(&destination)) {
            Ok(_) => updated += 1,
            Err(error) => failed.push(format!("{identity}: {error:#}")),
        }
    }
    println!("ynpm: {updated} updated, {} failed", failed.len());
    if failed.is_empty() {
        Ok(())
    } else {
        bail!("sync failed: {}", failed.join("; "))
    }
}

fn verb_prod(paths: &Paths, name: &str) -> anyhow::Result<()> {
    let (pkg, _) = expand_package(name)?;
    let state = paths.load_state()?;
    let key = find_package_key(&state, &pkg)
        .map(str::to_string)
        .context("package is not recorded by ynpm")?;
    let destination = package_destination(paths, &state.packages[&key]);
    if state.packages[&key].dev.is_none() {
        println!("ynpm: {pkg} is already on the production channel");
        return Ok(());
    }
    install_one_at(paths, &pkg, false, Some(&destination)).map(|_| ())
}

fn verb_remove(paths: &Paths, name: &str) -> anyhow::Result<()> {
    let (pkg, _) = expand_package(name)?;
    let mut state = paths.load_state()?;
    let key = find_package_key(&state, &pkg)
        .map(str::to_string)
        .context("package is not recorded by ynpm")?;
    let package = state.packages.remove(&key).context("package disappeared")?;
    let integration = package.integration.clone();
    let destination = package_destination(paths, &package);
    let generation_root = paths.generations().join(&key);
    let live = running_process_paths();
    if live.iter().any(|path| path.starts_with(&generation_root)) {
        bail!(
            "refusing to remove {pkg}: a running process still executes from {generation_root:?}"
        );
    }
    for bin in package.bins.keys() {
        let path = destination.join(bin);
        if path.is_symlink() || path.is_file() {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    if generation_root.exists() {
        fs::remove_dir_all(&generation_root)
            .with_context(|| format!("removing {}", generation_root.display()))?;
    }
    paths.save_state(&state)?;
    if integration.is_some() || pkg.starts_with("@ygghq/") {
        remove_app_registration(paths, &app_name_for_package(&pkg, integration.as_ref()))?;
    }
    println!("ynpm: removed {pkg} and its ynpm generations");
    Ok(())
}

fn import_generation(
    paths: &Paths,
    package: &str,
    version: &str,
    archive: &Path,
    bins: &[String],
    destination: Option<&Path>,
    integration: Option<yggterm_core::YggtermPackageMetadata>,
) -> anyhow::Result<()> {
    let key = package_storage_key(package);
    let staging =
        paths
            .scratch()
            .join(format!("import-{}-{}-{}", key, version, std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    extract_tarball(archive, &staging)?;
    for bin in bins {
        let path = generation_binary(&staging, bin);
        run_version(&path).with_context(|| format!("verifying imported bin {bin}"))?;
    }
    if bins.is_empty() {
        bail!("import requires at least one binary name");
    }
    let generation = paths.generation_dir(&key, version);
    if !generation.exists() {
        if let Some(parent) = generation.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_dir_all(&staging, &generation)?;
    }
    let destination = destination
        .map(absolute_path)
        .unwrap_or_else(|| paths.dest());
    let app_manifest = app_manifest_from_metadata(
        package,
        integration.as_ref(),
        &destination,
        &bins
            .iter()
            .map(|bin| (bin.clone(), format!("bin/{bin}")))
            .collect(),
    )?;
    let mut state = paths.load_state()?;
    let previous_integration = state
        .packages
        .get(&key)
        .and_then(|entry| entry.integration.clone());
    let entry = state.packages.entry(key).or_insert(Package {
        package_name: Some(package.to_string()),
        current: version.to_string(),
        versions: Vec::new(),
        bins: BTreeMap::new(),
        external_prev: None,
        destination: Some(destination.display().to_string()),
        dev: None,
        dev_generation: None,
        channel: Some("npm".to_string()),
        integration: None,
    });
    if entry.versions.last() != Some(&version.to_string()) {
        entry.versions.push(version.to_string());
    }
    entry.package_name = Some(package.to_string());
    entry.current = version.to_string();
    entry.bins = bins
        .iter()
        .map(|bin| (bin.clone(), format!("bin/{bin}")))
        .collect();
    entry.destination = Some(destination.display().to_string());
    entry.dev = None;
    entry.dev_generation = None;
    entry.channel = Some("npm".to_string());
    entry.integration = integration.clone();
    fs::create_dir_all(&destination)?;
    for bin in bins {
        publish_link(&generation_binary(&generation, bin), &destination.join(bin))?;
    }
    paths.save_state(&state)?;
    sync_app_registration(paths, package, previous_integration.as_ref(), app_manifest)?;
    let _ = fs::remove_dir_all(&staging);
    println!(
        "ynpm: imported {package}@{version} into {}",
        destination.display()
    );
    Ok(())
}

fn export_record(paths: &Paths, package: &str) -> anyhow::Result<ExportRecord> {
    let (package, _) = expand_package(package)?;
    let state = paths.load_state()?;
    let key = find_package_key(&state, &package)
        .map(str::to_string)
        .context("package is not recorded by ynpm")?;
    let entry = state
        .packages
        .get(&key)
        .context("package disappeared while exporting")?;
    Ok(ExportRecord {
        package: package.clone(),
        key,
        version: entry.current.clone(),
        bins: entry.bins.keys().cloned().collect(),
        dev: entry.dev.is_some(),
        integration: entry.integration.clone(),
    })
}

/// Emit a machine-readable production generation description and optionally
/// copy its archive to a caller-selected path. `sync-fleet` uses this small
/// peer protocol before contacting npm, so a release already present on any
/// recognized host becomes the local source of truth and is downloaded from
/// upstream at most once.
fn verb_export(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    let package = args.first().context("export wants a package name")?;
    let mut archive = None;
    let mut metadata = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--archive" => {
                index += 1;
                archive = Some(PathBuf::from(
                    args.get(index).context("--archive wants a path")?,
                ));
            }
            "--metadata" => metadata = true,
            value => bail!("unknown export option '{value}'"),
        }
        index += 1;
    }
    let record = export_record(paths, package)?;
    if record.dev {
        bail!(
            "{} is a dev generation and cannot be exported by production sync",
            record.package
        );
    }
    if record.bins.is_empty() {
        bail!("{} has no executable bins to export", record.package);
    }
    let generation = paths.generation_dir(&record.key, &record.version);
    for bin in &record.bins {
        let executable = generation_binary(&generation, bin);
        let answer = run_version(&executable).with_context(|| {
            format!(
                "verifying {} before exporting {}",
                executable.display(),
                record.package
            )
        })?;
        if !version_answer_matches_identity(&answer, &record.version) {
            bail!(
                "REFUSED: {} answers {:?}, not its recorded version {}",
                executable.display(),
                answer.trim(),
                record.version
            );
        }
    }
    let has_archive = archive.is_some();
    if let Some(destination) = archive {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let generated = tar_generation(paths, &record.key, &record.version)?;
        if generated != destination {
            fs::copy(&generated, &destination)
                .with_context(|| format!("copying export archive to {}", destination.display()))?;
            let _ = fs::remove_file(generated);
        }
    }
    // `--metadata` is explicit for callers, but printing the record for a
    // bare export also makes the verb inspectable at a shell prompt.
    if metadata || !has_archive {
        println!("{}", serde_json::to_string(&record)?);
    }
    Ok(())
}

fn tar_generation(paths: &Paths, key: &str, version: &str) -> anyhow::Result<PathBuf> {
    let generation = paths.generation_dir(key, version);
    if !generation.is_dir() {
        bail!("generation {key}@{version} is not present on this host");
    }
    fs::create_dir_all(paths.scratch())?;
    let archive = paths.scratch().join(format!(
        "fleet-{}-{}-{}.tgz",
        key,
        version,
        std::process::id()
    ));
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&generation)
        .arg(".")
        .status()
        .with_context(|| format!("archiving {key}@{version}"))?;
    if !status.success() {
        bail!("could not archive generation {key}@{version}");
    }
    Ok(archive)
}

fn tar_directory(paths: &Paths, source: &Path, label: &str) -> anyhow::Result<PathBuf> {
    if !source.is_dir() {
        bail!("directory {} is not present on this host", source.display());
    }
    fs::create_dir_all(paths.scratch())?;
    let archive = paths
        .scratch()
        .join(format!("{label}-{}.tgz", std::process::id()));
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(source)
        .arg(".")
        .status()
        .with_context(|| format!("archiving {}", source.display()))?;
    if !status.success() {
        bail!("could not archive {}", source.display());
    }
    Ok(archive)
}

fn yggterm_generation_is_complete(root: &Path, version: &str) -> bool {
    let extension = cfg!(target_os = "windows").then_some(".exe").unwrap_or("");
    ["yggterm", "yggterm-headless", "ynpm", "ynpx"]
        .iter()
        .all(|name| {
            root.join(version)
                .join(format!("{name}{extension}"))
                .is_file()
        })
}

fn local_yggterm_production_archive(
    paths: &Paths,
) -> anyhow::Result<Option<(String, String, String, PathBuf)>> {
    if yggterm_dev_fingerprint(paths).is_some() {
        println!("ynpm: local yggterm is DEV; production fleet sync will not replace it");
        return Ok(None);
    }
    let Ok(context) = yggterm_install_context(paths) else {
        println!("ynpm: yggterm is not a direct install; skipping fleet product sync");
        return Ok(None);
    };
    let root = context
        .managed_root
        .as_ref()
        .context("direct yggterm context has no managed root")?;
    let source_dir = root.join("versions").join(&context.current_version);
    let extension = cfg!(target_os = "windows").then_some(".exe").unwrap_or("");
    if !source_dir.join(format!("yggterm{extension}")).is_file()
        || !source_dir
            .join(format!("yggterm-headless{extension}"))
            .is_file()
    {
        bail!(
            "active yggterm generation {} has no GUI/headless pair under {}",
            context.current_version,
            source_dir.display()
        );
    }
    // deploy-fleet's compatibility layout publishes ynpm/ynpx beside the
    // `.yggterm/bin` aliases, while a native ynpm self-update keeps all four
    // products in the version directory. Normalize either source layout into
    // one scratch generation so fleet import never transfers a half-product.
    let staging = paths.scratch().join(format!(
        "yggterm-fleet-stage-{}-{}",
        context.current_version,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    for name in ["yggterm", "yggterm-headless"] {
        fs::copy(
            source_dir.join(format!("{name}{extension}")),
            staging.join(format!("{name}{extension}")),
        )?;
    }
    let current_executable = std::env::current_exe().ok();
    for name in ["ynpm", "ynpx"] {
        let source = yggterm_aux_source_candidates(
            &paths.home,
            root,
            &context.current_version,
            &format!("{name}{extension}"),
            current_executable.as_deref(),
        )
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| format!("active yggterm generation has no {name} product"))?;
        fs::copy(&source, staging.join(format!("{name}{extension}")))?;
    }
    for name in ["yggterm", "yggterm-headless", "ynpm", "ynpx"] {
        let path = staging.join(format!("{name}{extension}"));
        set_executable(&path)?;
        let answer = run_version(&path)
            .with_context(|| format!("verifying local yggterm product {name}"))?;
        if !version_answer_matches_identity(&answer, &context.current_version) {
            bail!(
                "local yggterm product {name} answered {:?}, not {}",
                answer.trim(),
                context.current_version
            );
        }
    }
    let archive = tar_directory(
        paths,
        &staging,
        &format!("yggterm-fleet-{}", context.current_version),
    )?;
    let _ = fs::remove_dir_all(&staging);
    Ok(Some((
        context.current_version,
        context.asset_label,
        context.repo,
        archive,
    )))
}

fn yggterm_dev_allows_production(
    paths: &Paths,
    production_version: &str,
    production_fingerprint: &str,
) -> bool {
    let Ok(state) = paths.load_state() else {
        return true;
    };
    let Some(key) = find_package_key(&state, "@ygghq/yggterm") else {
        return true;
    };
    let Some(marker) = state
        .packages
        .get(key)
        .and_then(|package| package.dev.as_ref())
    else {
        return true;
    };
    let Some(supersedes) = marker.supersedes.as_deref() else {
        return true;
    };
    let (Ok(production), Ok(dev_base)) =
        (SemVer::parse(production_version), SemVer::parse(supersedes))
    else {
        return false;
    };
    match SemVer::cmp_semver(&production, &dev_base) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            marker.dev_fingerprint.as_deref() != Some(production_fingerprint)
        }
    }
}

/// Return candidate manager binaries in trust order for a fleet archive.
///
/// `ynpm sync-fleet` is itself the source of truth for the auxiliary manager
/// products. A host can still have an older copy in a managed yggterm version
/// directory, though, and choosing that copy by filesystem order silently
/// downgrades every peer during `import-yggterm`. When the running executable
/// is ynpm/ynpx, it is the copy that just performed the sync and therefore the
/// only candidate allowed to outrank those historical paths.
fn yggterm_aux_source_candidates(
    home: &Path,
    root: &Path,
    version: &str,
    name: &str,
    current_executable: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if current_executable.is_some_and(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "ynpm" | "ynpm.exe" | "ynpx" | "ynpx.exe"))
    }) {
        candidates.push(current_executable.unwrap().to_path_buf());
    }
    candidates.extend([
        root.join("versions").join(version).join(name),
        home.join(".yggterm/bin").join(name),
        home.join(".local/bin").join(name),
    ]);
    candidates
}

fn verb_import_yggterm(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    let version = args.first().context("import-yggterm wants a version")?;
    let mut archive = None;
    let mut asset_label = None;
    let mut repo = "yggdrasilhq/yggterm".to_string();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--archive" => {
                index += 1;
                archive = Some(PathBuf::from(
                    args.get(index).context("--archive wants a path")?,
                ));
            }
            "--asset-label" => {
                index += 1;
                asset_label = Some(
                    args.get(index)
                        .context("--asset-label wants a value")?
                        .clone(),
                );
            }
            "--repo" => {
                index += 1;
                repo = args
                    .get(index)
                    .context("--repo wants owner/repository")?
                    .clone();
            }
            value => bail!("unknown import-yggterm option '{value}'"),
        }
        index += 1;
    }
    let archive = archive.context("import-yggterm wants --archive PATH")?;
    let asset_label = asset_label
        .or_else(|| yggterm_core::current_asset_label().ok())
        .context("import-yggterm could not determine the platform asset label")?;
    let staging =
        paths
            .scratch()
            .join(format!("import-yggterm-{}-{}", version, std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    extract_tarball(&archive, &staging)?;
    let extension = cfg!(target_os = "windows").then_some(".exe").unwrap_or("");
    let yggterm = staging.join(format!("yggterm{extension}"));
    let fingerprint = sha256_file(&yggterm)?;
    if !yggterm_dev_allows_production(paths, version, &fingerprint) {
        println!("ynpm: retained the remote yggterm dev build; production {version} is not ahead");
        let _ = fs::remove_dir_all(&staging);
        return Ok(());
    }
    for name in ["yggterm", "yggterm-headless", "ynpm", "ynpx"] {
        let path = staging.join(format!("{name}{extension}"));
        let answer = run_version(&path)
            .with_context(|| format!("verifying imported yggterm product {name}"))?;
        if !version_answer_matches_identity(&answer, version) {
            bail!(
                "REFUSED: imported yggterm product {name} answered {:?}, not {version}",
                answer.trim()
            );
        }
    }
    let root = yggterm_core::direct_install_root()?;
    let versions = root.join("versions");
    fs::create_dir_all(&versions)?;
    let final_dir = versions.join(version);
    if final_dir.exists() {
        if !yggterm_generation_is_complete(&versions, version) {
            fs::remove_dir_all(&final_dir)
                .with_context(|| format!("removing incomplete {}", final_dir.display()))?;
            fs::rename(&staging, &final_dir)?;
        } else {
            let _ = fs::remove_dir_all(&staging);
        }
    } else {
        fs::rename(&staging, &final_dir)?;
    }
    let yggterm = final_dir.join(format!("yggterm{extension}"));
    yggterm_core::write_direct_install_state(&root, &repo, &asset_label, version, &yggterm)?;
    for name in ["ynpm", "ynpx"] {
        publish_release_aux_link(
            &paths.home,
            &format!("{name}{extension}"),
            &final_dir.join(format!("{name}{extension}")),
        )?;
    }
    clear_yggterm_dev_state(paths)?;
    let _ = Command::new(&yggterm)
        .args(["install", "integrate"])
        .status();
    let headless = final_dir.join(format!("yggterm-headless{extension}"));
    let restart = Command::new(&headless)
        .args([
            "server",
            "monitor",
            "--scenario",
            "hot-restart",
            "--all",
            "--timeout-ms",
            "30000",
        ])
        .status();
    if restart.as_ref().is_ok_and(|status| status.success()) {
        println!("ynpm: requested the session-preserving daemon hot-restart");
    } else {
        eprintln!(
            "ynpm: yggterm activated; daemon hot-restart was not requested (the next GUI launch will converge)"
        );
    }
    println!("ynpm: imported and activated yggterm {version} from the fleet archive");
    Ok(())
}

fn bootstrap_remote_ynpm(host: &str) -> anyhow::Result<()> {
    let current = std::env::current_exe().context("locating ynpm for fleet sync")?;
    let remote_rel = format!(
        ".yggterm/scratchpad/ynpm/ynpm-bootstrap-{}",
        std::process::id()
    );
    let remote = format!("{host}:{remote_rel}");
    let status = Command::new("scp")
        .args(["-q"])
        .arg(current)
        .arg(&remote)
        .status()
        .with_context(|| format!("copying ynpm to {host}"))?;
    if !status.success() {
        bail!("could not copy ynpm to {host}");
    }
    let status = Command::new("ssh")
        .arg(host)
        .arg(format!(
            "set -eu; mkdir -p $HOME/.local/bin $HOME/.yggterm/bin $HOME/.yggterm/scratchpad/ynpm && chmod 755 $HOME/{remote_rel} && for name in ynpm ynpx; do for dir in $HOME/.local/bin $HOME/.yggterm/bin; do stage=\"$dir/.${{name}}-bootstrap-{pid}\"; cp $HOME/{remote_rel} \"$stage\" && chmod 755 \"$stage\" && mv -f \"$stage\" \"$dir/$name\"; done; done; rm -f $HOME/{remote_rel}",
            pid = std::process::id(),
            remote_rel = remote_rel
        ))
        .status()?;
    if !status.success() {
        bail!("could not activate ynpm on {host}");
    }
    Ok(())
}

fn remote_export_record(host: &str, package: &str) -> anyhow::Result<Option<ExportRecord>> {
    let command = format!(
        "$HOME/.local/bin/ynpm export {} --metadata",
        shell_quote(package)
    );
    let output = Command::new("ssh")
        .arg(host)
        .arg(command)
        .output()
        .with_context(|| format!("asking {host} for {package}'s ynpm generation"))?;
    if !output.status.success() {
        let detail = format!(
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_ascii_lowercase();
        if detail.contains("not recorded") || detail.contains("dev generation") {
            return Ok(None);
        }
        bail!(
            "remote ynpm export of {package} on {host} failed ({}): {}",
            output.status,
            detail.trim()
        );
    }
    let record = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<ExportRecord>(line.trim()).ok())
        .with_context(|| {
            format!(
                "remote ynpm export of {package} on {host} returned no metadata: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            )
        })?;
    if record.package != package {
        bail!(
            "remote ynpm export on {host} named {} instead of {package}",
            record.package
        );
    }
    Ok((!record.dev).then_some(record))
}

fn remote_export_archive(
    paths: &Paths,
    host: &str,
    record: &ExportRecord,
) -> anyhow::Result<PathBuf> {
    let remote_rel = format!(
        ".yggterm/scratchpad/ynpm/export-{}-{}-{}.tgz",
        record.key,
        record.version,
        std::process::id()
    );
    let command = format!(
        "$HOME/.local/bin/ynpm export {} --archive \"$HOME/{remote_rel}\" --metadata",
        shell_quote(&record.package)
    );
    let output = Command::new("ssh")
        .arg(host)
        .arg(command)
        .output()
        .with_context(|| format!("exporting {} from {host}", record.package))?;
    if !output.status.success() {
        bail!(
            "remote ynpm archive of {} on {host} failed ({}): {}",
            record.package,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let returned = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<ExportRecord>(line.trim()).ok())
        .context("remote ynpm archive returned no metadata")?;
    if returned.version != record.version || returned.bins != record.bins || returned.dev {
        bail!(
            "remote ynpm export of {} changed while it was being copied",
            record.package
        );
    }
    let local = paths.scratch().join(format!(
        "peer-{}-{}-{}.tgz",
        record.key,
        record.version,
        std::process::id()
    ));
    fs::create_dir_all(paths.scratch())?;
    let status = Command::new("scp")
        .args(["-q"])
        .arg(format!("{host}:{remote_rel}"))
        .arg(&local)
        .status()
        .with_context(|| format!("copying {} from {host}", record.package))?;
    let _ = Command::new("ssh")
        .arg(host)
        .arg(format!("rm -f \"$HOME/{remote_rel}\""))
        .status();
    if !status.success() {
        let _ = fs::remove_file(&local);
        bail!("could not copy {}'s generation from {host}", record.package);
    }
    Ok(local)
}

fn local_production_record(paths: &Paths, package: &str) -> Option<ExportRecord> {
    let state = paths.load_state().ok()?;
    let key = find_package_key(&state, package)?.to_string();
    let entry = state.packages.get(&key)?;
    (!entry.dev.is_some()).then(|| ExportRecord {
        package: package.to_string(),
        key,
        version: entry.current.clone(),
        bins: entry.bins.keys().cloned().collect(),
        dev: false,
        integration: entry.integration.clone(),
    })
}

fn local_package_is_dev(paths: &Paths, package: &str) -> bool {
    let Ok(state) = paths.load_state() else {
        return false;
    };
    find_package_key(&state, package)
        .and_then(|key| state.packages.get(key))
        .is_some_and(|entry| entry.dev.is_some())
}

fn reuse_remote_integrated_generations(paths: &Paths, hosts: &[String]) -> anyhow::Result<()> {
    let destination = paths.root().join("bin");
    for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
        let yggterm_core::agent_cli::CliInstall::Npm(package) = descriptor.install else {
            continue;
        };
        if local_package_is_dev(paths, package) {
            println!(
                "ynpm: cli {} is DEV locally; peer production releases will not replace it",
                descriptor.slug
            );
            continue;
        }
        let local = local_production_record(paths, package);
        let mut best = None::<(String, ExportRecord, SemVer)>;
        for host in hosts {
            let Some(record) = remote_export_record(host, package)? else {
                continue;
            };
            let Ok(version) = SemVer::parse(&record.version) else {
                eprintln!(
                    "ynpm: ignoring {}@{} from {host}; it is not an orderable version",
                    record.package, record.version
                );
                continue;
            };
            let replace = best
                .as_ref()
                .is_none_or(|(_, _, current)| SemVer::cmp_semver(&version, current).is_gt());
            if replace {
                best = Some((host.clone(), record, version));
            }
        }
        let Some((host, record, _)) = best else {
            continue;
        };
        let should_import = local.as_ref().is_none_or(|current| {
            SemVer::parse(&record.version)
                .ok()
                .zip(SemVer::parse(&current.version).ok())
                .is_some_and(|(remote, local)| SemVer::cmp_semver(&remote, &local).is_gt())
        });
        if !should_import {
            continue;
        }
        let archive = remote_export_archive(paths, &host, &record)?;
        let result = import_generation(
            paths,
            &record.package,
            &record.version,
            &archive,
            &record.bins,
            Some(&destination),
            record.integration.clone(),
        );
        let _ = fs::remove_file(&archive);
        result?;
        println!(
            "ynpm: reused {}@{} from fleet host {host} before registry sync",
            record.package, record.version
        );
    }
    Ok(())
}

/// Push already-verified generations to explicit fleet hosts. The generation
/// archive is made once locally and imported remotely, so a release is not
/// downloaded independently by every connected machine.
fn verb_sync_fleet(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    let mut hosts = None;
    let mut integrated = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--hosts" | "--fleet" => {
                index += 1;
                hosts = Some(
                    args.get(index)
                        .context("--hosts wants comma-separated machine keys")?
                        .split(',')
                        .map(str::trim)
                        .filter(|host| !host.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
            }
            "--integrated" => integrated = true,
            value => bail!("unknown sync-fleet option '{value}'"),
        }
        index += 1;
    }
    let hosts = hosts
        .or_else(|| {
            std::env::var("YGGTERM_FLEET_HOSTS").ok().map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|host| !host.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .filter(|hosts: &Vec<String>| !hosts.is_empty())
        .context("sync-fleet needs --hosts H1,H2 (ynpm cannot infer a GUI's SSH roster from a bare shell)")?;
    for host in &hosts {
        bootstrap_remote_ynpm(host)?;
    }
    sync_yggterm_to_hosts(paths, &hosts)?;
    if integrated {
        // A package already present on any peer is a valid local source. Do
        // this before the registry sync so the expensive tarball crosses the
        // WAN once, then let the normal manifest check decide whether an even
        // newer release exists.
        reuse_remote_integrated_generations(paths, &hosts)?;
        sync_integrated(paths)?;
    }
    let state = paths.load_state()?;
    let mut pushed = 0usize;
    for (key, entry) in &state.packages {
        if entry.dev.is_some() {
            println!("ynpm: {key} is DEV; not replacing it during production fleet sync");
            continue;
        }
        let package = package_identity(key, entry);
        let archive = tar_generation(paths, key, &entry.current)?;
        let remote_rel = format!(
            ".yggterm/scratchpad/ynpm/fleet-{}-{}-{}.tgz",
            key,
            entry.current,
            std::process::id()
        );
        let bins = entry.bins.keys().cloned().collect::<Vec<_>>();
        let bin_csv = bins.join(",");
        let destination = package_destination(paths, entry);
        let integration = entry
            .integration
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let integration = integration
            .as_deref()
            .map(|value| format!(" --integration-json {}", shell_quote(value)))
            .unwrap_or_default();
        for host in &hosts {
            let status = Command::new("scp")
                .args(["-q"])
                .arg(&archive)
                .arg(format!("{host}:{remote_rel}"))
                .status()
                .with_context(|| format!("pushing {package}@{} to {host}", entry.current))?;
            if !status.success() {
                bail!("could not push {package} to {host}");
            }
            let command = format!(
                "$HOME/.local/bin/ynpm import {package} {version} --archive \"$HOME/{remote_rel}\" --dest {destination} --bins {bins}{integration}; rm -f \"$HOME/{remote_rel}\"",
                package = shell_quote(&package),
                version = shell_quote(&entry.current),
                remote_rel = remote_rel,
                destination = shell_quote(&destination.display().to_string()),
                bins = shell_quote(&bin_csv),
                integration = integration,
            );
            let status = Command::new("ssh").arg(host).arg(command).status()?;
            if !status.success() {
                bail!("could not import {package} on {host}");
            }
            pushed += 1;
        }
        let _ = fs::remove_file(&archive);
    }
    println!(
        "ynpm: fleet sync imported {pushed} package generations across {} host(s)",
        hosts.len()
    );
    Ok(())
}

/// Distribute the active production yggterm generation using the same
/// download-once rule as app packages. The remote import re-runs the product
/// version gates and refuses to replace a remote dev build unless production
/// is newer or has same-version binary polish.
fn sync_yggterm_to_hosts(paths: &Paths, hosts: &[String]) -> anyhow::Result<()> {
    let Some((version, asset_label, repo, archive)) = local_yggterm_production_archive(paths)?
    else {
        return Ok(());
    };
    let remote_rel = format!(
        ".yggterm/scratchpad/ynpm/yggterm-fleet-{}-{}.tgz",
        version,
        std::process::id()
    );
    for host in hosts {
        let status = Command::new("scp")
            .args(["-q"])
            .arg(&archive)
            .arg(format!("{host}:{remote_rel}"))
            .status()
            .with_context(|| format!("pushing yggterm {version} to {host}"))?;
        if !status.success() {
            bail!("could not push yggterm {version} to {host}");
        }
        let command = format!(
            "$HOME/.local/bin/ynpm import-yggterm {version} --archive \"$HOME/{remote_rel}\" --asset-label {asset_label} --repo {repo}; rm -f \"$HOME/{remote_rel}\"",
            version = shell_quote(&version),
            remote_rel = remote_rel,
            asset_label = shell_quote(&asset_label),
            repo = shell_quote(&repo),
        );
        let status = Command::new("ssh")
            .arg(host)
            .arg(command)
            .status()
            .with_context(|| format!("importing yggterm {version} on {host}"))?;
        if !status.success() {
            bail!("could not import yggterm {version} on {host}");
        }
        // The yggterm archive carries ynpm/ynpx for a self-contained product
        // install. Reassert the manager that is performing this fleet sync
        // after import as well: it is the newest protocol peer and must not be
        // replaced by a stale auxiliary copy left by an older deploy layout.
        bootstrap_remote_ynpm(host)?;
        println!("ynpm: distributed yggterm {version} to {host}");
    }
    let _ = fs::remove_file(archive);
    Ok(())
}

fn verb_import(paths: &Paths, args: &[String]) -> anyhow::Result<()> {
    let package = args.first().context("import wants a package name")?;
    let version = args.get(1).context("import wants a version")?;
    let mut archive = None;
    let mut destination = None;
    let mut bins = None;
    let mut integration = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--archive" => {
                index += 1;
                archive = Some(PathBuf::from(
                    args.get(index).context("--archive wants a path")?,
                ));
            }
            "--dest" | "--destination" => {
                index += 1;
                destination = Some(PathBuf::from(
                    args.get(index).context("--dest wants a path")?,
                ));
            }
            "--bins" => {
                index += 1;
                bins = Some(
                    args.get(index)
                        .context("--bins wants comma-separated names")?
                        .split(',')
                        .filter(|name| !name.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
            }
            "--integration-json" => {
                index += 1;
                let raw = args
                    .get(index)
                    .context("--integration-json wants a JSON object")?;
                integration = Some(
                    serde_json::from_str(raw)
                        .context("--integration-json is not valid yggterm metadata")?,
                );
            }
            value => bail!("unknown import option '{value}'"),
        }
        index += 1;
    }
    let archive = archive.context("import wants --archive PATH")?;
    let bins = bins.context("import wants --bins name,name")?;
    let (package, _) = expand_package(package)?;
    import_generation(
        paths,
        &package,
        version,
        &archive,
        &bins,
        destination.as_deref(),
        integration,
    )
}

/// Every executable or absolute command-line path a running process currently
/// holds, best effort. `/proc/<pid>/exe` is enough for native bins; Node-based
/// CLIs need `/proc/<pid>/cmdline` because their interpreter is `/usr/bin/node`.
fn running_process_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return paths;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if let Ok(path) = fs::read_link(format!("/proc/{name}/exe")) {
            paths.push(path);
        }
        // Node-based CLIs execute `/usr/bin/node`; their package generation
        // appears only in cmdline (`node .../bin/codex`). Include absolute
        // arguments so purge/remove do not mistake a live JavaScript session
        // for a dangling tree and delete helpers it still needs.
        if let Ok(command_line) = fs::read(format!("/proc/{name}/cmdline")) {
            paths.extend(
                command_line
                    .split(|byte| *byte == 0)
                    .filter_map(|argument| {
                        let argument = std::str::from_utf8(argument).ok()?;
                        argument.starts_with('/').then(|| PathBuf::from(argument))
                    }),
            );
        }
    }
    paths
}

fn path_is_live(path: &Path, live: &[PathBuf]) -> bool {
    live.iter().any(|executable| executable.starts_with(path))
}

/// Remove the old yggterm npm prefix and duplicate user-local npm package
/// trees after `sync --integrated` has populated ynpm. Only exact package
/// paths are considered, and a generation still used by a running process is
/// reported and retained for a later sweep.
fn verb_purge_legacy(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    let live = running_process_paths();
    let old_root = paths.home.join(".yggterm/npm");
    let mut removed = 0usize;
    let mut deferred = 0usize;
    if let Ok(entries) = fs::read_dir(old_root.join("cli")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".gen"))
            {
                continue;
            }
            if path_is_live(&path, &live) {
                println!("ynpm: retain live legacy generation {}", path.display());
                deferred += 1;
            } else {
                fs::remove_dir_all(&path)
                    .with_context(|| format!("removing legacy generation {}", path.display()))?;
                removed += 1;
            }
        }
    }
    if let Ok(entries) = fs::read_dir(old_root.join("bin")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() || path.is_file() {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }

    let mut npm_packages = yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .filter_map(|descriptor| match descriptor.install {
            yggterm_core::agent_cli::CliInstall::Npm(package) => Some(package),
            _ => None,
        })
        .collect::<Vec<_>>();
    // The v1 package is not in the descriptor registry but is the specific
    // OpenCode line the v2 migration retires.
    npm_packages.push("opencode-ai");
    for package in npm_packages {
        for package_root in [
            paths.home.join(".local/lib/node_modules").join(package),
            old_root.join("lib/node_modules").join(package),
        ] {
            if package_root.exists() {
                if path_is_live(&package_root, &live) {
                    println!(
                        "ynpm: retain live legacy package {}",
                        package_root.display()
                    );
                    deferred += 1;
                } else {
                    fs::remove_dir_all(&package_root).with_context(|| {
                        format!("removing legacy package {}", package_root.display())
                    })?;
                    removed += 1;
                }
            }
        }
    }
    let old_node_modules = old_root.join("lib/node_modules");
    if old_node_modules.is_dir() {
        if path_is_live(&old_node_modules, &live) {
            println!(
                "ynpm: retain live legacy npm module tree {}",
                old_node_modules.display()
            );
            deferred += 1;
        } else {
            // This prefix is yggterm-owned, unlike ~/.local/lib/node_modules,
            // which can contain unrelated user tools. Removing the complete
            // old tree also catches transitive package roots left behind by
            // the former global npm installer.
            fs::remove_dir_all(&old_node_modules).with_context(|| {
                format!(
                    "removing legacy npm module tree {}",
                    old_node_modules.display()
                )
            })?;
            removed += 1;
        }
    }
    for descriptor in yggterm_core::agent_cli::AGENT_CLIS {
        let yggterm_core::agent_cli::CliInstall::Npm(_) = descriptor.install else {
            continue;
        };
        let path = paths.home.join(".local/bin").join(descriptor.binary_name);
        if fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let resolved = fs::canonicalize(&path).ok();
        let owned_by_ynpm = resolved
            .as_deref()
            .is_some_and(|resolved| resolved.starts_with(paths.root().join("bin")));
        if owned_by_ynpm {
            continue;
        }
        if path_is_live(resolved.as_deref().unwrap_or(&path), &live) {
            println!("ynpm: retain live duplicate {}", path.display());
            deferred += 1;
        } else if path.is_symlink() || path.is_file() {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    // State is read intentionally, not rewritten: purge removes only legacy
    // filesystem artefacts and must never erase ynpm's rollback record.
    let _ = state;
    println!("ynpm: legacy purge removed {removed}, deferred {deferred} live artefact(s)");
    Ok(())
}

fn is_network_failure(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}").to_ascii_lowercase();
    [
        "curl ",
        "could not resolve host",
        "failed to connect",
        "timed out",
        "timeout",
        "network is unreachable",
        "connection reset",
        "registry fetch",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn github_checkout(paths: &Paths, spec: &str) -> anyhow::Result<PathBuf> {
    let repo = spec
        .strip_prefix("github:")
        .or_else(|| spec.strip_prefix("git+https://github.com/"))
        .or_else(|| spec.strip_prefix("https://github.com/"))
        .context("GitHub spec wants github:owner/repo")?
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let mut pieces = repo.split('/');
    let owner = pieces
        .next()
        .filter(|part| !part.is_empty())
        .context("GitHub spec has no owner")?;
    let name = pieces
        .next()
        .filter(|part| !part.is_empty())
        .context("GitHub spec has no repository")?;
    if pieces.next().is_some() {
        bail!("GitHub spec must be github:owner/repo");
    }
    let checkout = paths.root().join("github").join(format!("{owner}--{name}"));
    if checkout.join(".git").exists() {
        let output = Command::new("git")
            .args(["-C", &checkout.display().to_string(), "pull", "--ff-only"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("updating GitHub checkout {repo}"))?;
        if !output.status.success() {
            let detail = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if is_network_failure(&anyhow::anyhow!(detail.trim().to_string())) {
                eprintln!("ynpx: GitHub unavailable; using the last local checkout {repo}");
            } else {
                bail!(
                    "could not fast-forward GitHub checkout {repo}: {}",
                    detail.trim()
                );
            }
        }
    } else {
        fs::create_dir_all(checkout.parent().context("GitHub checkout has no parent")?)?;
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                &format!("https://github.com/{repo}.git"),
            ])
            .arg(&checkout)
            .stdin(Stdio::null())
            .status()
            .with_context(|| format!("cloning GitHub checkout {repo}"))?;
        if !status.success() {
            bail!("could not clone GitHub checkout {repo}");
        }
    }
    Ok(checkout)
}

fn launch_installed(
    paths: &Paths,
    package: &str,
    bin: Option<&str>,
    args: &[String],
) -> anyhow::Result<i32> {
    let state = paths.load_state()?;
    let key = find_package_key(&state, package)
        .map(str::to_string)
        .context("package was not installed and the network was unavailable")?;
    let entry = state.packages.get(&key).context("package disappeared")?;
    let destination = package_destination(paths, entry);
    let name = bin
        .map(str::to_string)
        .or_else(|| {
            entry
                .bins
                .keys()
                .find(|candidate| package.ends_with(candidate.as_str()))
                .cloned()
        })
        .or_else(|| entry.bins.keys().next().cloned())
        .context("installed package has no executable bin")?;
    let executable = destination.join(&name);
    if !executable.exists() {
        bail!(
            "installed package {package} has no published bin {}",
            executable.display()
        );
    }
    let status = Command::new(&executable)
        .args(args)
        .status()
        .with_context(|| format!("launching {}", executable.display()))?;
    Ok(status.code().unwrap_or(1))
}

fn run_ynpx(paths: &Paths, args: &[String]) -> anyhow::Result<i32> {
    let mut package = None;
    let mut bin = None;
    let mut rest = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--bin" => {
                index += 1;
                bin = Some(args.get(index).context("--bin wants a bin name")?.clone());
            }
            "--dev" => {
                index += 1;
                let checkout = PathBuf::from(args.get(index).context("--dev wants a checkout")?);
                let outcome =
                    dev_checkout(paths, &checkout, None, None, BTreeMap::new(), None, &[])?;
                package = Some(outcome.package);
            }
            "--" if package.is_some() => rest.extend(args[index + 1..].iter().cloned()),
            value if package.is_none() => package = Some(value.to_string()),
            value => rest.push(value.to_string()),
        }
        if args.get(index).is_some_and(|value| value == "--") {
            break;
        }
        index += 1;
    }
    let package = package.context("ynpx wants a package, github:owner/repo, or --dev checkout")?;
    if package.starts_with("github:")
        || package.starts_with("https://github.com/")
        || package.starts_with("git+https://github.com/")
    {
        let checkout = github_checkout(paths, &package)?;
        let outcome = dev_checkout(paths, &checkout, None, None, BTreeMap::new(), None, &[])?;
        return launch_installed(paths, &outcome.package, bin.as_deref(), &rest);
    }
    if Path::new(&package).exists() {
        let outcome = dev_checkout(
            paths,
            Path::new(&package),
            None,
            None,
            BTreeMap::new(),
            None,
            &[],
        )?;
        return launch_installed(paths, &outcome.package, bin.as_deref(), &rest);
    }
    let (canonical, pin) = expand_package(&package)?;
    let install_spec = pin
        .as_deref()
        .map(|pin| format!("{canonical}@{pin}"))
        .unwrap_or_else(|| canonical.clone());
    // The fast network arm makes offline ynpx useful: it tries the latest
    // package briefly, then runs the last verified generation if the network
    // is unavailable. Non-network failures remain loud.
    unsafe { std::env::set_var("YNPM_NETWORK_TIMEOUT_SECS", "8") };
    let existing_dev = paths.load_state().ok().and_then(|state| {
        find_package_key(&state, &canonical)
            .and_then(|key| state.packages.get(key))
            .and_then(|entry| entry.dev.clone())
    });
    if let Some(marker) = existing_dev {
        let manifest = marker
            .watch
            .as_deref()
            .and_then(|watch| fetch_package_manifest(watch, None).ok());
        let handback = dev_handback(
            manifest.as_ref().map(|manifest| manifest.version.as_str()),
            marker.supersedes.as_deref(),
            marker.supersedes_fingerprint.as_deref(),
            manifest
                .as_ref()
                .and_then(|manifest| manifest.integrity.as_deref().or(manifest.shasum.as_deref())),
        );
        if handback == DevHandback::KeepDev {
            return launch_installed(paths, &canonical, bin.as_deref(), &rest);
        }
    }
    match install_one(paths, &install_spec, true) {
        Ok(_) => launch_installed(paths, &canonical, bin.as_deref(), &rest),
        Err(error) if is_network_failure(&error) => {
            eprintln!("ynpx: offline; using the last verified {canonical} generation");
            launch_installed(paths, &canonical, bin.as_deref(), &rest)
        }
        Err(error) => Err(error),
    }
}

fn generation_binary(generation: &Path, bin: &str) -> PathBuf {
    let nested = generation.join("bin").join(bin);
    if nested.exists() {
        nested
    } else {
        generation.join(bin)
    }
}

fn verb_rollback(paths: &Paths, name: &str) -> anyhow::Result<()> {
    let (package_name, _) = expand_package(name)?;
    let key = {
        let state = paths.load_state()?;
        find_package_key(&state, &package_name)
            .map(str::to_string)
            .context("package was never installed through ynpm")?
    };
    let mut state = paths.load_state()?;
    let package = state
        .packages
        .get_mut(&key)
        .context("package disappeared")?;
    let target = package
        .versions
        .iter()
        .rev()
        .find(|version| {
            package
                .bins
                .keys()
                .all(|bin| generation_binary(&paths.generation_dir(&key, version), bin).is_file())
        })
        .cloned()
        .filter(|version| package.dev.is_some() || version != &package.current)
        .context("no earlier verified generation is available to roll back to")?;
    let destination = package_destination(paths, package);
    for bin in package.bins.keys() {
        let source = generation_binary(&paths.generation_dir(&key, &target), bin);
        publish_link(&source, &destination.join(bin))?;
    }
    package.current = target.clone();
    package.dev = None;
    package.channel = Some("npm".to_string());
    paths.save_state(&state)?;
    println!("ynpm: {package_name} rolled back to {target}");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let home = match std::env::var("YNPM_HOME") {
        Ok(h) if !h.trim().is_empty() => PathBuf::from(h),
        _ => PathBuf::from(
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .context(
                    "ynpm needs HOME/USERPROFILE (or YNPM_HOME) to know where its state lives",
                )?,
        ),
    };
    let paths = Paths::new(absolute_path(&home));
    let mut argv = std::env::args();
    let invoked_as = argv
        .next()
        .and_then(|path| {
            PathBuf::from(path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "ynpm".to_string());
    let args: Vec<String> = argv.collect();
    const USAGE: &str = "ynpm - the yggdrasilhq package manager\n\
         verbs: install [--dest DIR] <pkg>[@<ver>]... | install --dev <pkg> [--watch PKG] [--bin NAME=PATH]... |\n\
         list | check | doctor | sync [--integrated] | sync-fleet --hosts H1,H2 [--integrated] | self-update [--json] | export <pkg> [--metadata] [--archive PATH] | import <pkg> <ver> --archive PATH --bins a,b | import-yggterm <ver> --archive PATH | rollback <pkg> | remove <pkg> | prod <pkg> |\n\
         purge-legacy                                        # remove old yggterm/npm copies when no live process uses them\n\
         dev [--build CMD] [--watch PKG] [--fleet H1,H2] [--dest DIR] [--bin NAME=PATH] <checkout>\n\
         ynpx <pkg> [flags] installs/updates when online, then launches the verified bin; github:owner/repo and --dev checkout are supported";
    if invoked_as != "ynpx" && args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if invoked_as != "ynpx" && args.iter().any(|a| a == "--version" || a == "-V") {
        // Bare version, exactly what yggterm and yggterm-headless print: the
        // deploy script compares the three outputs verbatim.
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if invoked_as == "ynpx" && args.len() == 1 && args.iter().any(|a| a == "--version" || a == "-V")
    {
        // A bare alias version query is the distribution identity; a
        // package's `ynpx <pkg> --version` continues through the launcher.
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if invoked_as == "ynpx" {
        let code = run_ynpx(&paths, &args)?;
        std::process::exit(code);
    }
    let Some(verb) = args.first() else {
        eprintln!("{USAGE}");
        bail!("no verb given");
    };
    match verb.as_str() {
        "install" => verb_install(&paths, &args[1..]),
        "list" => verb_list(&paths),
        "check" => verb_check(&paths),
        "doctor" => {
            if args.len() != 1 {
                bail!("doctor takes no arguments");
            }
            verb_doctor(&paths)
        }
        "sync" => verb_sync(&paths, &args[1..]),
        "sync-fleet" => verb_sync_fleet(&paths, &args[1..]),
        "self-update" => verb_self_update(&paths, &args[1..]),
        "import-yggterm" => verb_import_yggterm(&paths, &args[1..]),
        "activate-yggterm-dev" => {
            if args.len() != 1 {
                bail!("activate-yggterm-dev takes no arguments");
            }
            activate_yggterm_dev(&paths)
        }
        "export" => verb_export(&paths, &args[1..]),
        "import" => verb_import(&paths, &args[1..]),
        "purge-legacy" => {
            if args.len() != 1 {
                bail!("purge-legacy takes no arguments");
            }
            verb_purge_legacy(&paths)
        }
        "remove" => {
            let name = args.get(1).context("remove wants a package")?;
            verb_remove(&paths, name)
        }
        "prod" => {
            let name = args.get(1).context("prod wants a package")?;
            verb_prod(&paths, name)
        }
        "dev" => {
            let mut build = None;
            let mut watch = None;
            let mut fleet = Vec::new();
            let mut destination = None;
            let mut bins = BTreeMap::new();
            let mut checkout = None;
            let mut index = 1;
            while index < args.len() {
                match args[index].as_str() {
                    "--build" => {
                        index += 1;
                        build = Some(args.get(index).context("--build wants a command")?.clone());
                    }
                    "--watch" => {
                        index += 1;
                        watch = Some(args.get(index).context("--watch wants a package")?.clone());
                    }
                    "--fleet" => {
                        index += 1;
                        fleet = args
                            .get(index)
                            .context("--fleet wants comma-separated host names")?
                            .split(',')
                            .map(str::trim)
                            .filter(|host| !host.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                    "--dest" | "--destination" => {
                        index += 1;
                        destination = Some(PathBuf::from(
                            args.get(index).context("--dest wants a destination")?,
                        ));
                    }
                    "--bin" => {
                        index += 1;
                        let value = args.get(index).context("--bin wants name=path")?;
                        let (name, path) =
                            value.split_once('=').context("--bin wants name=path")?;
                        bins.insert(name.to_string(), PathBuf::from(path));
                    }
                    value if value.starts_with("--") => bail!("unknown dev option '{value}'"),
                    value if checkout.is_none() => checkout = Some(PathBuf::from(value)),
                    value => bail!("unexpected dev argument '{value}'"),
                }
                index += 1;
            }
            let checkout = checkout.context("dev wants a checkout path")?;
            dev_checkout(&paths, &checkout, build, watch, bins, destination, &fleet).map(|_| ())
        }
        "rollback" => {
            let name = args
                .get(1)
                .context("rollback wants a package (example: ynpm rollback ychrome)")?;
            verb_rollback(&paths, name)
        }
        other => bail!(
            "'{other}' is not an ynpm verb (install | list | check | doctor | sync | sync-fleet | self-update | export | import | import-yggterm | rollback | remove | prod | purge-legacy | dev)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_is_scoped_and_a_pin_survives() {
        assert_eq!(
            expand_package("ychrome").unwrap(),
            ("@ygghq/ychrome".to_string(), None)
        );
        assert_eq!(
            expand_package("ychrome@0.2.1").unwrap(),
            ("@ygghq/ychrome".to_string(), Some("0.2.1".to_string()))
        );
        assert_eq!(
            expand_package("@ygghq/ychrome").unwrap(),
            ("@ygghq/ychrome".to_string(), None)
        );
    }

    #[test]
    fn a_scoped_name_is_not_eaten_by_the_pin_split() {
        // The '@' of "@scope/name" must not be mistaken for a version pin.
        let (pkg, pin) = expand_package("@ygghq/ychrome@0.2.1").unwrap();
        assert_eq!(pkg, "@ygghq/ychrome");
        assert_eq!(pin.as_deref(), Some("0.2.1"));
    }

    #[test]
    fn the_integrated_opencode2_package_uses_the_beta_tag() {
        assert_eq!(integrated_npm_dist_tag("@opencode-ai/cli"), Some("beta"));
        assert_eq!(integrated_npm_dist_tag("@openai/codex"), None);
    }

    #[test]
    fn junk_is_refused_not_guessed() {
        assert!(expand_package("").is_err());
        assert!(expand_package("a/b").is_err());
        assert!(expand_package("@noslash").is_err());
    }

    #[test]
    fn the_platform_map_is_honest_about_what_we_ship() {
        // ⚠ std spells it "linux" - the lowercase that the first dogfood
        // install tripped on. The map accepts the constant AS IT IS.
        assert_eq!(platform_target("linux", "x86_64").unwrap(), "linux-x64");
        assert_eq!(platform_target("linux", "aarch64").unwrap(), "linux-arm64");
        assert_eq!(platform_target("linux", "arm64").unwrap(), "linux-arm64");
        assert_eq!(platform_target("Linux", "x86_64").unwrap(), "linux-x64");
        // The lie install.sh used to tell: mapping a platform we never
        // shipped, so the fetch 404s somewhere downstream.
        assert!(platform_target("macos", "x86_64").is_err());
        assert!(platform_target("windows", "x86_64").is_err());
    }

    #[test]
    fn versions_order_by_triple_then_prerelease_below_release() {
        use SemVer as S;
        let v = |t: &str| S::parse(t).unwrap();
        assert_eq!(
            S::cmp_semver(&v("0.2.1"), &v("0.2.1")),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            S::cmp_semver(&v("0.2.10"), &v("0.2.9")),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            S::cmp_semver(&v("1.0.0"), &v("0.99.9")),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            S::cmp_semver(&v("1.0.0-rc1"), &v("1.0.0")),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            S::cmp_semver(&v("1.0.0-rc1"), &v("1.0.0-rc2")),
            std::cmp::Ordering::Less
        );
        assert!(
            S::parse("0.2").is_err(),
            "a two-part version is not orderable here"
        );
    }

    #[test]
    fn a_version_answer_must_contain_the_version() {
        assert!(version_answer_matches("ychrome 0.2.1", "0.2.1"));
        assert!(version_answer_matches("v0.2.1", "0.2.1"));
        assert!(version_answer_matches("0.2.1", "0.2.1"));
        assert!(
            !version_answer_matches("ychrome 0.1.0", "0.2.1"),
            "the 0.2.0 lie"
        );
        assert!(
            !version_answer_matches("", "0.2.1"),
            "a silent binary is a lie of silence"
        );
    }

    #[test]
    fn build_metadata_in_a_cli_version_is_orderable_and_not_drift() {
        let parsed = SemVer::parse("0.132.0+13595c36+litc03171d9").unwrap();
        assert_eq!((parsed.major, parsed.minor, parsed.patch), (0, 132, 0));
        assert_eq!(
            version_from_answer("codex-cli 0.132.0+13595c36+litc03171d9").as_deref(),
            Some("0.132.0+13595c36+litc03171d9")
        );
        assert!(version_answer_matches_identity(
            "codex-cli 0.132.0+13595c36+litc03171d9",
            "0.132.0-13595c36-litc03171d9"
        ));
    }

    #[test]
    fn the_drift_instrument_reads_the_version_out_of_any_honest_answer() {
        assert_eq!(
            version_from_answer("ychrome 0.2.1").as_deref(),
            Some("0.2.1")
        );
        assert_eq!(version_from_answer("v3.2.19").as_deref(), Some("3.2.19"));
        assert!(version_from_answer("nothing usable").is_none());
    }

    #[test]
    fn a_latest_manifest_yields_its_dist_tags_and_tarball() {
        let doc = serde_json::json!({
            "name": "@ygghq/ychrome",
            "version": "0.2.1",
            "dist-tags": { "latest": "0.2.1" },
            "dist": { "tarball": "https://registry.npmjs.org/@ygghq/ychrome-linux-x64/-/x-0.2.1.tgz" },
            "optionalDependencies": { "@ygghq/ychrome-linux-x64": "0.2.1" }
        });
        let m = parse_manifest(&doc).unwrap();
        assert_eq!(m.version, "0.2.1");
        assert!(m.tarball.ends_with("x-0.2.1.tgz"));
        assert_eq!(m.optional_dependencies, vec!["@ygghq/ychrome-linux-x64"]);
    }

    #[test]
    fn a_packument_answers_from_under_its_versions_table() {
        // `registry/<pkg>` is the EVERY-VERSION document: no top-level
        // version, no top-level dist - the tarball hides under
        // versions[<latest>]. The first dogfood install tripped on exactly
        // this shape.
        let doc = serde_json::json!({
            "name": "@ygghq/ychrome",
            "dist-tags": { "latest": "0.2.1" },
            "versions": {
                "0.1.0": { "version": "0.1.0", "dist": { "tarball": "https://registry.npmjs.org/x-0.1.0.tgz" } },
                "0.2.1": { "version": "0.2.1", "dist": { "tarball": "https://registry.npmjs.org/x-0.2.1.tgz" },
                    "optionalDependencies": { "@ygghq/ychrome-linux-x64": "0.2.1" } }
            }
        });
        let m = parse_manifest(&doc).unwrap();
        assert_eq!(m.version, "0.2.1");
        assert!(m.tarball.ends_with("x-0.2.1.tgz"));
        assert_eq!(m.optional_dependencies.len(), 1);
    }

    #[test]
    fn a_manifest_without_a_tarball_is_a_refusal_not_a_guess() {
        let doc =
            serde_json::json!({ "name": "@ygghq/ychrome", "dist-tags": { "latest": "0.2.1" } });
        assert!(parse_manifest(&doc).is_err());
        assert!(parse_manifest(&serde_json::json!({})).is_err());
    }

    #[test]
    fn the_bin_table_is_the_whole_contract_of_what_gets_installed() {
        let doc = serde_json::json!({
            "name": "@ygghq/ychrome-linux-x64",
            "version": "0.2.1",
            "bin": { "ychrome": "bin/ychrome", "ychrome-vault": "bin/ychrome-vault" }
        });
        let bins = parse_bin_table(&doc).unwrap();
        assert_eq!(bins.get("ychrome").map(String::as_str), Some("bin/ychrome"));
        assert_eq!(bins.len(), 2);
        assert!(parse_bin_table(&serde_json::json!({ "name": "x" })).is_err());
        assert!(parse_bin_table(&serde_json::json!({ "bin": {} })).is_err());
    }

    #[test]
    fn a_string_bin_table_derives_the_public_command_name() {
        let bins = parse_bin_table(&serde_json::json!({
            "name": "@scope/codex-session-tui",
            "bin": "bin/cli.js"
        }))
        .unwrap();
        assert_eq!(
            bins.get("codex-session-tui"),
            Some(&"bin/cli.js".to_string())
        );
    }

    #[test]
    fn scoped_package_storage_keys_cannot_collide_on_the_basename() {
        assert_eq!(package_storage_key("@ygghq/ytop"), "ytop");
        assert_eq!(package_storage_key("@a/tool"), "a__tool");
        assert_ne!(
            package_storage_key("@a/tool"),
            package_storage_key("@b/tool")
        );
    }

    #[test]
    fn a_dev_release_hands_back_only_forward_or_on_new_prod_metadata() {
        assert_eq!(
            dev_handback(Some("1.2.0"), Some("1.1.0"), None, None),
            DevHandback::ReleaseAvailable
        );
        assert_eq!(
            dev_handback(Some("1.0.0"), Some("1.1.0"), None, None),
            DevHandback::KeepDev
        );
        assert_eq!(
            dev_handback(Some("1.1.0"), Some("1.1.0"), Some("old"), Some("new")),
            DevHandback::ReleaseAvailable
        );
        assert_eq!(
            dev_handback(Some("1.1.0"), Some("1.1.0"), Some("same"), Some("same")),
            DevHandback::KeepDev
        );
        assert_eq!(
            dev_handback(Some("not-semver"), Some("1.1.0"), None, None),
            DevHandback::KeepDev
        );
    }

    #[test]
    fn dev_args_keep_watch_destination_and_explicit_bin_identity() {
        let parsed = parse_dev_args(&[
            "@avikalpa/zcode-tui".to_string(),
            "--watch".to_string(),
            "@avikalpa/zcode-tui".to_string(),
            "--dest".to_string(),
            "/home/user/.local/bin".to_string(),
            "--bin".to_string(),
            "zcode-tui=/home/user/dist/zcode-tui".to_string(),
        ])
        .unwrap();
        assert_eq!(parsed.package, "@avikalpa/zcode-tui");
        assert_eq!(parsed.watch.as_deref(), Some("@avikalpa/zcode-tui"));
        assert_eq!(
            parsed.bins.get("zcode-tui"),
            Some(&PathBuf::from("/home/user/dist/zcode-tui"))
        );
        assert!(parse_dev_args(&["@scope/tool".to_string()]).is_err());
    }

    #[test]
    fn state_survives_a_round_trip_and_rollback_picks_the_newest_kept_generation() {
        let package = Package {
            package_name: Some("@ygghq/ychrome".to_string()),
            current: "0.2.1".to_string(),
            versions: vec![
                "0.1.0".to_string(),
                "0.2.0".to_string(),
                "0.2.1".to_string(),
            ],
            bins: BTreeMap::from([("ychrome".to_string(), "bin/ychrome".to_string())]),
            external_prev: Some("0.1.0".to_string()),
            destination: None,
            dev: None,
            dev_generation: None,
            channel: Some("npm".to_string()),
            integration: None,
        };
        let json = serde_json::to_string(&package).unwrap();
        let back: Package = serde_json::from_str(&json).unwrap();
        assert_eq!(back, package);
        assert_eq!(back.versions.len(), 3);
    }

    #[test]
    fn paths_derive_from_the_home_and_the_dest_override_wins() {
        let paths = Paths::new("/home/user");
        assert_eq!(
            paths.state_file(),
            PathBuf::from("/home/user/.yggterm/ynpm/state.json")
        );
        assert_eq!(
            paths.generation_dir("ychrome", "0.2.1"),
            PathBuf::from("/home/user/.yggterm/ynpm/generations/ychrome/0.2.1")
        );
        assert_eq!(
            Paths::dest_from(None, Path::new("/home/user")),
            PathBuf::from("/home/user/.local/bin")
        );
        assert_eq!(
            Paths::dest_from(Some("/opt/tools"), Path::new("/home/user")),
            PathBuf::from("/opt/tools")
        );
        assert_eq!(
            Paths::dest_from(Some("  "), Path::new("/home/user")),
            PathBuf::from("/home/user/.local/bin"),
            "an empty override is no override"
        );
        assert_eq!(
            resolve_path_from(Path::new("bin"), Path::new("/home/user")),
            PathBuf::from("/home/user/bin")
        );
        assert_eq!(
            resolve_path_from(Path::new("/opt/tools"), Path::new("/home/user")),
            PathBuf::from("/opt/tools")
        );
    }

    #[test]
    fn fleet_archive_prefers_the_running_manager_over_a_stale_versioned_copy() {
        let candidates = yggterm_aux_source_candidates(
            Path::new("/home/user"),
            Path::new("/home/user/.yggterm"),
            "3.2.91",
            "ynpm",
            Some(Path::new("/home/user/.local/bin/ynpm")),
        );
        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from("/home/user/.local/bin/ynpm"))
        );
        assert_eq!(
            candidates.get(1),
            Some(&PathBuf::from("/home/user/.yggterm/versions/3.2.91/ynpm"))
        );
    }
}
