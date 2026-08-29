//! `ynpm` - the yggdrasilhq package manager. Ships with yggterm.
//!
//! # Why this exists
//!
//! The fleet's binaries are delivered as npm packages under the `@ygghq`
//! scope (the registry is public; the tarballs are plain HTTPS). Before this
//! CLI existed, every fleet update was that delivery assembled BY HAND on
//! each host: curl the tarball, extract, keep a `.prev` copy, swap the
//! binary, restart daemons, hope. An agent's discipline resets every
//! session; a verb's does not. This is the verb.
//!
//! # The contract
//!
//! | verb | does |
//! |---|---|
//! | `ynpm install <pkg>[@<ver>]...` | resolve, download, VERIFY, swap into DEST, keep a generation |
//! | `ynpm list` | what is installed, and what each binary self-reports |
//! | `ynpm check` | the drift instrument: disk vs state vs registry latest |
//! | `ynpm sync` | install the registry latest of every installed package |
//! | `ynpm rollback <pkg>` | the previous generation back into DEST |
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
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (text, None),
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
        Self { home: home.into() }
    }
    pub fn state_file(&self) -> PathBuf {
        self.home.join(".yggterm/ynpm/state.json")
    }
    pub fn generations(&self) -> PathBuf {
        self.home.join(".yggterm/ynpm/generations")
    }
    pub fn generation_dir(&self, name: &str, version: &str) -> PathBuf {
        self.generations().join(name).join(version)
    }
    pub fn dest(&self) -> PathBuf {
        Self::dest_from(std::env::var("YNPM_DEST").ok().as_deref(), &self.home)
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
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        let path = self.state_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(state)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

// ===== registry + process substrate (curl / tar / the binary itself) =====

/// The registry manifest of ONE version, reduced to what the flow needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub tarball: String,
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
    Ok(Manifest { version, tarball })
}

/// The platform package's bin table, verbatim: bin name -> path inside the
/// package. The finalize contract: EVERY entry is a binary this tool copies
/// and verifies.
pub fn parse_bin_table(doc: &serde_json::Value) -> anyhow::Result<BTreeMap<String, String>> {
    let bins = doc
        .get("bin")
        .and_then(|v| v.as_object())
        .context("platform package carries no bin table - nothing to install")?;
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

fn curl(url: &str) -> anyhow::Result<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "120", url])
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
    let url = match pin {
        Some(v) => format!("https://registry.npmjs.org/{pkg}/{v}"),
        None => format!("https://registry.npmjs.org/{pkg}"),
    };
    let body = curl(&url).with_context(|| format!("fetching the manifest of {pkg}"))?;
    let doc: serde_json::Value = serde_json::from_slice(&body)
        .with_context(|| format!("parsing the registry manifest of {pkg}"))?;
    parse_manifest(&doc).with_context(|| format!("resolving {pkg}"))
}

fn latest_version(pkg: &str) -> anyhow::Result<String> {
    let body = curl(&format!("https://registry.npmjs.org/{pkg}"))
        .with_context(|| format!("fetching the manifest of {pkg}"))?;
    let doc: serde_json::Value = serde_json::from_slice(&body)?;
    parse_manifest(&doc).map(|m| m.version)
}

/// Run a binary with `--version` and give it a moment: a freshly copied ELF
/// answering nothing is not a version, it is a hang we are about to own.
fn run_version(bin: &Path) -> anyhow::Result<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("spawning {} --version", bin.display()))?;
    let started = Instant::now();
    let deadline = Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            let out = child.wait_with_output()?;
            let answer = String::from_utf8_lossy(&out.stdout).to_string();
            if !status.success() && answer.trim().is_empty() {
                bail!(
                    "{} --version exited {} with no answer",
                    bin.display(),
                    status
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

// ===== the install pipeline =====

pub struct InstallOutcome {
    pub name: String,
    pub version: String,
    pub previous: Option<String>,
    pub bins: Vec<String>,
}

/// One package, resolved to its feet: fetch, verify the binaries tell the
/// truth, keep the generation, THEN swap into DEST by rename. Nothing is
/// written to DEST before every bin has answered `--version` correctly.
fn install_one(paths: &Paths, spec: &str, quiet: bool) -> anyhow::Result<InstallOutcome> {
    let (pkg, pin) = expand_package(spec)?;
    let name = pkg
        .strip_prefix("@ygghq/")
        .with_context(|| format!("'{pkg}' is outside the @ygghq scope this tool manages"))?
        .to_string();
    let platform = platform_target(std::env::consts::OS, run_simple("uname", &["-m"])?.trim())?;
    let platform_pkg = format!("{pkg}-{platform}");

    let manifest = fetch_manifest(&pkg, pin.as_deref())?;
    let version = manifest.version.clone();
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
        if !version_answer_matches(&answer, &platform_version) {
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
    let dest = paths.dest();
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut state = paths.load_state()?;
    // The state mutation is a closed scope: the entry borrow must end before
    // the state is saved, and what the render below needs out of it is two
    // plain values.
    let (previous, external_prev) = {
        match state.packages.entry(name.clone()) {
            std::collections::btree_map::Entry::Occupied(mut occupied) => {
                let package = occupied.get_mut();
                let previous = Some(package.current.clone());
                package.versions.push(version.clone());
                package.current = version.clone();
                package.bins = bins.clone();
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
                    current: version.clone(),
                    versions: vec![version.clone()],
                    bins: bins.clone(),
                    external_prev: external_prev.clone(),
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

fn set_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("chmod 755 {}", path.display()))
}

fn run_simple(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{program} {} failed: {}", args.join(" "), out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ===== verbs =====

fn verb_install(paths: &Paths, specs: &[String]) -> anyhow::Result<()> {
    if specs.is_empty() {
        bail!("install wants at least one package (example: ynpm install ychrome)");
    }
    let mut failed = 0usize;
    for spec in specs {
        if let Err(err) = install_one(paths, spec, false) {
            eprintln!("ynpm: ⛔ {spec}: {err:#}");
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} package(s) failed", specs.len());
    }
    Ok(())
}

fn verb_list(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    if state.packages.is_empty() {
        println!("ynpm: nothing installed through ynpm yet (try: ynpm install ychrome)");
        return Ok(());
    }
    for (name, package) in &state.packages {
        let bins: Vec<String> = package
            .bins
            .keys()
            .map(|bin| {
                let disk = run_version(&paths.dest().join(bin))
                    .ok()
                    .as_deref()
                    .and_then(version_from_answer)
                    .unwrap_or_else(|| "?".to_string());
                format!("{bin}={disk}")
            })
            .collect();
        println!("{name} {} [{}]", package.current, bins.join(", "));
    }
    Ok(())
}

fn verb_check(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    if state.packages.is_empty() {
        println!("ynpm: nothing installed through ynpm yet");
        return Ok(());
    }
    let mut drifted = 0usize;
    for (name, package) in &state.packages {
        let disk = package
            .bins
            .keys()
            .next()
            .and_then(|bin| run_version(&paths.dest().join(bin)).ok())
            .as_deref()
            .and_then(version_from_answer);
        let latest = latest_version(&format!("@ygghq/{name}")).ok();
        let mut flags = Vec::new();
        match &disk {
            Some(d) if d != &package.current => {
                flags.push(format!(
                    "DRIFT: disk answers {d}, state says {}",
                    package.current
                ));
            }
            None => flags.push("DRIFT: the installed binary answers no version".to_string()),
            _ => {}
        }
        match &latest {
            Some(l) if Some(l) != Some(&package.current) => {
                flags.push(format!("behind the registry: latest is {l} (ynpm sync)"));
            }
            _ => {}
        }
        if flags.is_empty() {
            println!(
                "{name} {}: current (registry {})",
                package.current,
                latest.unwrap_or_else(|| "?".to_string())
            );
        } else {
            drifted += 1;
            println!("{name} {}: {}", package.current, flags.join("; "));
        }
    }
    if drifted > 0 {
        bail!("{drifted} package(s) drifted");
    }
    Ok(())
}

fn verb_sync(paths: &Paths) -> anyhow::Result<()> {
    let state = paths.load_state()?;
    if state.packages.is_empty() {
        println!("ynpm: nothing installed through ynpm yet");
        return Ok(());
    }
    let names: Vec<String> = state.packages.keys().cloned().collect();
    let mut updated = 0usize;
    let mut failed = 0usize;
    for name in &names {
        let current = state.packages[name].current.clone();
        match latest_version(&format!("@ygghq/{name}")) {
            Ok(latest) if latest != current => {
                println!("ynpm: {name} {current} -> {latest}");
                match install_one(paths, &format!("@ygghq/{name}"), true) {
                    Ok(_) => updated += 1,
                    Err(err) => {
                        eprintln!("ynpm: ⛔ {name}: {err:#}");
                        failed += 1;
                    }
                }
            }
            Ok(_) => println!("ynpm: {name} {current}: current"),
            Err(err) => {
                eprintln!("ynpm: ⛔ {name}: {err:#}");
                failed += 1;
            }
        }
    }
    println!(
        "ynpm: {updated} updated, {} current, {failed} failed",
        names.len() - updated
    );
    if failed > 0 {
        bail!("{failed} package(s) failed to sync");
    }
    Ok(())
}

fn verb_rollback(paths: &Paths, name: &str) -> anyhow::Result<()> {
    let (pkg, _) = expand_package(name)?;
    let name = pkg
        .strip_prefix("@ygghq/")
        .with_context(|| format!("'{pkg}' is outside the @ygghq scope this tool manages"))?
        .to_string();
    let mut state = paths.load_state()?;
    let Some(package) = state.packages.get_mut(&name) else {
        bail!("{name} was never installed through ynpm");
    };
    // The newest entry below current that still has its generation bytes.
    let target = package
        .versions
        .iter()
        .rev()
        .skip(1)
        .find(|version| {
            package
                .bins
                .keys()
                .all(|bin| paths.generation_dir(&name, version).join(bin).exists())
        })
        .cloned();
    let Some(target) = target else {
        bail!(
            "{name} has no earlier generation on disk to roll back to \
             (versions recorded: {:?})",
            package.versions
        );
    };
    let dest = paths.dest();
    for bin in package.bins.keys() {
        let src = paths.generation_dir(&name, &target).join(bin);
        let staged = dest.join(format!(".ynpm-new-{bin}"));
        std::fs::copy(&src, &staged).with_context(|| format!("staging {bin}"))?;
        set_executable(&staged)?;
        std::fs::rename(&staged, dest.join(bin))
            .with_context(|| format!("swapping {bin} back to {target}"))?;
    }
    package.current = target.clone();
    paths.save_state(&state)?;
    println!("ynpm: {name} rolled back to {target}");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let home = match std::env::var("YNPM_HOME") {
        Ok(h) if !h.trim().is_empty() => PathBuf::from(h),
        _ => PathBuf::from(
            std::env::var("HOME")
                .context("ynpm needs HOME (or YNPM_HOME) to know where its state lives")?,
        ),
    };
    let paths = Paths::new(home);
    let args: Vec<String> = std::env::args().skip(1).collect();
    const USAGE: &str = "ynpm - the yggdrasilhq package manager\n\
         verbs: install <pkg>[@<ver>]... | list | check | sync | rollback <pkg>\n\
         every @ygghq binary lands in ~/.local/bin (YNPM_DEST overrides) with a generation kept for rollback";
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        // Bare version, exactly what yggterm and yggterm-headless print: the
        // deploy script compares the three outputs verbatim.
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let Some(verb) = args.first() else {
        eprintln!("{USAGE}");
        bail!("no verb given");
    };
    match verb.as_str() {
        "install" => verb_install(&paths, &args[1..]),
        "list" => verb_list(&paths),
        "check" => verb_check(&paths),
        "sync" => verb_sync(&paths),
        "rollback" => {
            let name = args
                .get(1)
                .context("rollback wants a package (example: ynpm rollback ychrome)")?;
            verb_rollback(&paths, name)
        }
        other => bail!("'{other}' is not an ynpm verb (install | list | check | sync | rollback)"),
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
            "dist": { "tarball": "https://registry.npmjs.org/@ygghq/ychrome-linux-x64/-/x-0.2.1.tgz" }
        });
        let m = parse_manifest(&doc).unwrap();
        assert_eq!(m.version, "0.2.1");
        assert!(m.tarball.ends_with("x-0.2.1.tgz"));
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
                "0.2.1": { "version": "0.2.1", "dist": { "tarball": "https://registry.npmjs.org/x-0.2.1.tgz" } }
            }
        });
        let m = parse_manifest(&doc).unwrap();
        assert_eq!(m.version, "0.2.1");
        assert!(m.tarball.ends_with("x-0.2.1.tgz"));
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
    fn state_survives_a_round_trip_and_rollback_picks_the_newest_kept_generation() {
        let package = Package {
            current: "0.2.1".to_string(),
            versions: vec![
                "0.1.0".to_string(),
                "0.2.0".to_string(),
                "0.2.1".to_string(),
            ],
            bins: BTreeMap::from([("ychrome".to_string(), "bin/ychrome".to_string())]),
            external_prev: Some("0.1.0".to_string()),
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
    }
}
