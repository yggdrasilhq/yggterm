//! The seed front launcher — a narrow bootstrap mode of the native `ygg`
//! front that forwards to the installed seed generation WITHOUT loading any
//! command IR. A bootstrap generation resolves its target before the
//! installed contract is ever consulted, so an older launcher never rejects
//! a newer generation's IR.
//!
//! Pointer contract (schema 1, written by native ynpm `seed-activate` alone):
//! `<seed-root>/active.json` = { "schema": 1, "generation": "<name>",
//! "bins": { "<program>": "<sha256>" } }. Every path the launcher derives is
//! relative and stays inside `<seed-root>/generations/`; the exact target
//! bytes are gated by their recorded sha256; a target that is this very
//! process's executable is refused instead of recursing. Unix additionally
//! keeps the `<seed-root>/current` symlink for compatibility; the launcher
//! reader contract is the active.json pointer on every platform.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const ACTIVE_SCHEMA: u64 = 1;

fn is_windows() -> bool {
    cfg!(windows)
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// One generation-name component of the pointer is attacker-influenced
/// state; a name that could leave the owned generations tree is refused
/// before any path is built from it.
fn valid_generation_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn active_pointer(seed_root: &Path) -> Result<(String, std::collections::BTreeMap<String, String>), String> {
    let path = seed_root.join("active.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("seed pointer {} unreadable: {e}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("seed pointer is not valid JSON: {e}"))?;
    let schema = doc
        .get("schema")
        .and_then(|v| v.as_u64())
        .ok_or("seed pointer declares no integer schema")?;
    if schema != ACTIVE_SCHEMA {
        return Err(format!(
            "seed pointer schema {schema} unsupported; this launcher speaks schema {ACTIVE_SCHEMA} only"
        ));
    }
    let generation = doc
        .get("generation")
        .and_then(|v| v.as_str())
        .ok_or("seed pointer declares no generation")?
        .to_string();
    let bins: std::collections::BTreeMap<String, String> = doc
        .get("bins")
        .and_then(|v| v.as_object())
        .ok_or("seed pointer declares no bins table")?
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();
    if bins.is_empty() {
        return Err("seed pointer bins table is empty".into());
    }
    Ok((generation, bins))
}

/// Refuse a resolved target that is the running launcher itself: forwarding
/// to self would recurse forever.
fn refuses_self(target: &Path) -> bool {
    match std::env::current_exe() {
        Ok(self_exe) => match (self_exe.canonicalize(), target.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => self_exe == target,
        },
        Err(_) => false,
    }
}

/// Resolve the exact program binary inside the owned generation named by
/// the seed pointer, enforcing schema, containment, the hash gate and the
/// self-recursion refusal. No other policy engine runs here.
pub fn resolve_program(seed_root: &Path, program: &str) -> Result<PathBuf, String> {
    if program.contains('/') || program.contains('\\') || program != program.trim() || program.is_empty()
    {
        return Err(format!("launcher program name {program:?} is not a bare program name"));
    }
    let (generation, bins) = active_pointer(seed_root)?;
    if !valid_generation_name(&generation) {
        return Err(format!(
            "seed pointer generation {generation:?} is not a plain generation name"
        ));
    }
    let expected = bins.get(program).ok_or_else(|| {
        format!(
            "seed pointer names bins {}, not {program:?}; launch through a name the seed installed",
            bins.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("seed pointer bin digest is not a sha256 hex token".into());
    }
    let generations = seed_root.join("generations");
    let target = generations
        .join(&generation)
        .join("bin")
        .join(if is_windows() {
            format!("{program}.exe")
        } else {
            program.to_string()
        });
    // Containment: the derived path must stay inside the owned tree even if
    // a future pointer shape grows a separator through validation.
    let canonical_root = generations
        .canonicalize()
        .map_err(|e| format!("generations tree {}: {e}", generations.display()))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("seed target {}: {e}", target.display()))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err("resolved seed target escaped the owned generations tree; refusing".into());
    }
    let actual = sha256_hex(&canonical_target)?;
    if actual != *expected {
        return Err(format!(
            "seed target {} sha256 mismatch: pointer expects {expected}, found {actual}; \
             refusing a generation whose bytes are not the activated ones",
            canonical_target.display()
        ));
    }
    if refuses_self(&canonical_target) {
        return Err("seed target resolves to this launcher itself; refusing to recurse".into());
    }
    Ok(canonical_target)
}

/// The `ygg --seed-launch` surface. Explicit `--root`/`--program` exist for
/// qualification harnesses; absent them, the seed root is this launcher's
/// parent directory and the program is the launcher's own file stem.
pub fn launch(argv: &[String]) -> Result<u32, String> {
    let mut root: Option<PathBuf> = None;
    let mut program: Option<String> = None;
    let mut forward: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--root" => {
                i += 1;
                root = Some(PathBuf::from(
                    argv.get(i).ok_or("--seed-launch --root needs a value")?,
                ));
            }
            "--program" => {
                i += 1;
                program = Some(argv.get(i).ok_or("--seed-launch --program needs a value")?.clone());
            }
            "--" => {
                forward.extend(argv[i + 1..].iter().cloned());
                break;
            }
            other => return Err(format!("--seed-launch does not take {other:?}; use --root/--program/--")),
        }
        i += 1;
    }
    let seed_root = match root {
        Some(root) => root,
        None => std::env::current_exe()
            .map_err(|e| format!("resolving launcher location: {e}"))?
            .parent()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .ok_or("launcher lives at <seed-root>/launchers/<name>; no seed root found")?,
    };
    let program = match program {
        Some(program) => program,
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().to_string()))
            .ok_or("launcher program name could not be derived from argv0")?,
    };
    let target = resolve_program(&seed_root, &program)?;
    forward_program(&target, &forward)
}

/// Exit status and signal discipline travel to the forwarded program: unix
/// replaces this process; windows spawns and mirrors the exit code.
fn forward_program(target: &Path, forward: &[String]) -> Result<u32, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(target).args(forward).exec();
        return Err(format!("exec {} failed: {error}", target.display()));
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(target)
            .args(forward)
            .status()
            .map_err(|e| format!("spawning {} failed: {e}", target.display()))?;
        Ok(status.code().unwrap_or(1) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox {
        home: PathBuf,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .expect("test home")
                .join(".yggterm/scratchpad/ygg-cli")
                .join(format!("seed-launch-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).unwrap();
            Self { home }
        }
        fn root(&self) -> PathBuf {
            self.home.join("seed")
        }
        /// Install a fake generation whose bin contents hash to themselves.
        fn install(&self, generation: &str, bins: &[(&str, &str)]) {
            let bin = self.root().join("generations").join(generation).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let mut table = serde_json::Map::new();
            for (program, body) in bins {
                let path = bin.join(program);
                std::fs::write(&path, body).unwrap();
                table.insert(
                    (*program).to_string(),
                    serde_json::Value::String(sha256_hex(&path).unwrap()),
                );
            }
            let pointer = serde_json::json!({
                "schema": ACTIVE_SCHEMA,
                "generation": generation,
                "bins": serde_json::Value::Object(table),
            });
            std::fs::write(
                self.root().join("active.json"),
                serde_json::to_string(&pointer).unwrap(),
            )
            .unwrap();
        }
        fn write_pointer(&self, value: serde_json::Value) {
            std::fs::write(
                self.root().join("active.json"),
                serde_json::to_string(&value).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    #[test]
    fn resolves_the_activated_generation_bin_through_the_hash_gate() {
        let sandbox = Sandbox::new("resolve");
        sandbox.install("4.0.0-abc123def456", &[("ygg", "#!/bin/sh\necho ygg\n")]);
        let target = resolve_program(&sandbox.root(), "ygg").unwrap();
        assert!(target.starts_with(sandbox.root().join("generations/4.0.0-abc123def456/bin")));
    }

    #[test]
    fn a_mutated_target_fails_the_exact_hash_gate() {
        let sandbox = Sandbox::new("mutation");
        sandbox.install("4.0.0-abc123def456", &[("ygg", "original bytes")]);
        let bin = sandbox
            .root()
            .join("generations/4.0.0-abc123def456/bin/ygg");
        std::fs::write(&bin, "tampered bytes").unwrap();
        let error = resolve_program(&sandbox.root(), "ygg").unwrap_err();
        assert!(error.contains("sha256 mismatch"), "{error}");
    }

    #[test]
    fn unknown_programs_and_bad_generations_are_refused_before_any_path_is_built() {
        let sandbox = Sandbox::new("refuse");
        sandbox.install("4.0.0-abc123def456", &[("ygg", "bytes")]);
        let error = resolve_program(&sandbox.root(), "ynpx").unwrap_err();
        assert!(error.contains("launch through a name"), "{error}");
        for evil in ["..", "a/b", "a\\b", "", ".hidden/.."] {
            sandbox.write_pointer(serde_json::json!({
                "schema": 1, "generation": evil, "bins": {"ygg": "0".repeat(64)}
            }));
            let error = resolve_program(&sandbox.root(), "ygg").unwrap_err();
            assert!(
                error.contains("generation") || error.contains("generations tree"),
                "{evil:?} was not refused as a generation: {error}"
            );
        }
        sandbox.write_pointer(serde_json::json!({
            "schema": 2, "generation": "4.0.0-abc123def456", "bins": {"ygg": "0".repeat(64)}
        }));
        assert!(resolve_program(&sandbox.root(), "ygg")
            .unwrap_err()
            .contains("schema 2 unsupported"));
    }

    #[test]
    fn a_dangling_pointer_fails_closed() {
        let sandbox = Sandbox::new("dangling");
        sandbox.install("9.9.9-deadbeefcafe", &[("ygg", "bytes")]);
        std::fs::remove_dir_all(sandbox.root().join("generations/9.9.9-deadbeefcafe")).unwrap();
        let error = resolve_program(&sandbox.root(), "ygg").unwrap_err();
        assert!(error.contains("No such file") || error.contains("seed target"), "{error}");
    }

    #[test]
    fn the_running_launcher_is_refused_as_its_own_target() {
        let self_exe = std::env::current_exe().unwrap();
        assert!(refuses_self(&self_exe));
        assert!(!refuses_self(Path::new("/definitely/not/the/launcher")));
    }
}
