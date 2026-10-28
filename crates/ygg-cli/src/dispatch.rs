//! Owned component dispatch: resolve the declared runner for a command,
//! verify it, then hand the process over while preserving argv, signals and
//! exit status. Resolution never searches PATH for the porcelain itself and
//! never executes a runner that resolves back to this binary.

use crate::grammar::Args;
use crate::ir::{Component, Loaded};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum Dispatch {
    /// Execute this runner with these fixed prefix + trailing arguments.
    Exec {
        program: PathBuf,
        interpreter: Option<String>,
        argv: Vec<String>,
        env: Vec<(String, String)>,
        component: String,
    },
    /// The command's source adapter is not wired in this generation.
    StagedUnavailable {
        path: Vec<String>,
        request_json: String,
    },
    /// An explicit refusal (recursive shim, unprovisioned interpreter,
    /// missing compatibility tool, wired adapter without a native
    /// implementation). Exit code 3.
    Refused(String),
}

fn payload_root() -> PathBuf {
    crate::ir::payload_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn home_bin(name: &str) -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .and_then(|h| first_existing(&Path::new(&h).join(".local/bin"), name))
}

/// Candidate names for an executable on this platform: the exact declared
/// name plus the platform executable suffix when missing, so a payload that
/// ships `ynpm` without an explicit `.exe` still resolves on Windows.
fn exe_candidates(name: &str) -> Vec<String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut out = vec![name.to_string()];
    if !suffix.is_empty() && !name.ends_with(suffix) {
        out.push(format!("{name}{suffix}"));
    }
    out
}

fn first_existing(dir: &Path, name: &str) -> Option<PathBuf> {
    exe_candidates(name)
        .into_iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if let Some(found) = first_existing(&dir, name) {
            return Some(found);
        }
    }
    None
}

fn exe_sibling(name: &str) -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.to_path_buf()))
        .and_then(|dir| first_existing(&dir, name))
}

/// Evaluate a component's declared resolution strategies in order. An
/// explicitly set env var wins even when the target is missing: an override
/// that silently falls through to a payload copy would hide a broken
/// installation from the operator. `env:` names an exact file override;
/// `env_dir:` names a directory the runner is joined into.
fn resolve_runner(component: &Component, tool: &str) -> Result<PathBuf, String> {
    for strategy in &component.resolution {
        if let Some(var) = strategy.strip_prefix("env:") {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    return Ok(PathBuf::from(value));
                }
            }
            continue;
        }
        if let Some(var) = strategy.strip_prefix("env_dir:") {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    let dir = PathBuf::from(value);
                    // The var names the payload root: runners sit flat in
                    // the dispersed bin tree or under components/ in the
                    // packaged layout. Both are the same installation.
                    let found = first_existing(&dir, tool)
                        .or_else(|| first_existing(&dir.join("components"), tool));
                    return match found {
                        Some(path) => Ok(path),
                        None => Err(format!(
                            "ygg: {var} is set to {} but no {tool} was found under it; repair that installation or unset {var}",
                            dir.display()
                        )),
                    };
                }
            }
            continue;
        }
        let candidate = if let Some(name) = strategy.strip_prefix("home_bin:") {
            home_bin(name)
        } else if let Some(name) = strategy.strip_prefix("path:") {
            find_on_path(name)
        } else if let Some(name) = strategy.strip_prefix("exe_sibling:") {
            exe_sibling(name)
        } else if strategy == "payload" || strategy.starts_with("payload:") {
            let name = strategy.strip_prefix("payload:").unwrap_or(tool);
            first_existing(&payload_root(), name)
        } else if strategy == "internal" {
            return Err(format!(
                "internal component reached without a wired adapter: {tool}"
            ));
        } else {
            return Err(format!(
                "component {} declares an unsupported resolution strategy: {strategy}",
                component.runner
            ));
        };
        if let Some(path) = candidate {
            return Ok(path);
        }
    }
    Err(format!("ygg: compatibility tool unavailable: {tool}"))
}

/// The canonical self check: a runner that resolves to this binary would
/// recurse through the porcelain instead of doing the work.
fn is_self(program: &Path) -> bool {
    let Ok(self_exe) = std::env::current_exe() else {
        return false;
    };
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(program) == canon(&self_exe)
}

pub fn plan_legacy(
    loaded: &Loaded,
    path: &[String],
    rest: &[String],
) -> Dispatch {
    let node = &loaded.tree[&path.to_vec()];
    let record = &node.record;
    let component_id = &record.component;
    let Some(component) = loaded.document.components.get(component_id) else {
        return Dispatch::Refused(format!(
            "ygg: command names an undeclared component: {}",
            path.join("/")
        ));
    };
    let tool = record
        .legacy
        .first()
        .map(|s| s.trim_start_matches('@').to_string())
        .unwrap_or_else(|| component.runner.clone());
    let fixed: Vec<String> = record.legacy.iter().skip(1).cloned().collect();

    let program = match resolve_runner(component, &tool) {
        Ok(p) => p,
        Err(message) => return Dispatch::Refused(message),
    };
    if is_self(&program) {
        return Dispatch::Refused(format!(
            "ygg: component resolution is recursive ({}); install the real runner for component {component_id}",
            program.display()
        ));
    }
    if let Some(interpreter) = &component.interpreter {
        let available = if interpreter.contains('/') {
            Path::new(interpreter).is_file()
        } else {
            find_on_path(interpreter).is_some()
        };
        if !available {
            return Dispatch::Refused(format!(
                "ygg: interpreter not provisioned for component {component_id}: {interpreter} is required and was not found"
            ));
        }
    }
    let mut argv = fixed;
    argv.extend(rest.iter().cloned());
    Dispatch::Exec {
        program,
        interpreter: component.interpreter.clone(),
        argv,
        env: Vec::new(),
        component: component_id.clone(),
    }
}

/// Wire the inspection families through the contract's `wired_adapters`.
/// A family with no wired entry stages honest unavailable coverage with
/// exit 3 — never a fake empty ledger. A wired entry forwards to the named
/// adapter file resolved from the installed payload (`YGG_INSPECT_BIN`
/// exact-file override, then `<payload>/components`, then a flattened
/// payload root) under `python3`, passing the native frontend's generation
/// identity (`YGG_COMMAND_CONTRACT_HASH`, `YGG_KERNEL_DIR`, and
/// `YGG_COMMAND_IR` when the contract came from a file) so both sides load
/// one contract. The adapter's own verdict and exit status pass through:
/// health is never self-certified here.
pub fn plan_query(loaded: &Loaded, path: &[String], args: &Args, rest: &[String]) -> Dispatch {
    let family = path.first().map(|s| s.as_str()).unwrap_or("");
    let Some(spec) = loaded.document.wired_adapters.get(family) else {
        let request = format!(
            "{{\"arguments\": {}, \"command\": {}}}",
            python_style_object(args),
            python_style_list(path)
        );
        return Dispatch::StagedUnavailable {
            path: path.to_vec(),
            request_json: request,
        };
    };
    let adapter = spec.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
    if adapter.is_empty() {
        return Dispatch::Refused(format!(
            "ygg: wired adapter for family {family} is malformed; the contract must name an adapter filename"
        ));
    }
    let root = payload_root();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(value) = std::env::var("YGG_INSPECT_BIN") {
        if !value.trim().is_empty() {
            let selected = PathBuf::from(value);
            if !selected.is_file() {
                return Dispatch::Refused(format!(
                    "ygg: explicitly selected inspection adapter {} is unavailable; repair that generation or unset YGG_INSPECT_BIN",
                    selected.display()
                ));
            }
            candidates.push(selected);
        }
    }
    for dir in [root.join("components"), root.clone()] {
        candidates.extend(exe_candidates(adapter).into_iter().map(|n| dir.join(n)));
    }
    let Some(program) = candidates.into_iter().find(|p| p.is_file()) else {
        return Dispatch::Refused(format!(
            "ygg: adapter for family {family} is declared but {adapter} was not found in the installed payload; install a matching generation"
        ));
    };
    let interpreter = "python3".to_string();
    if find_on_path(&interpreter).is_none() && !Path::new(&interpreter).is_file() {
        return Dispatch::Refused(format!(
            "ygg: interpreter not provisioned for the {family} adapter: python3 is required and was not found; a fresh host without Python should use the remote-operator door (ygg remote <host>)"
        ));
    }
    let mut env: Vec<(String, String)> = vec![
        (
            "YGG_KERNEL_DIR".into(),
            crate::ir::kernel_dir().display().to_string(),
        ),
        (
            "YGG_COMMAND_CONTRACT_HASH".into(),
            loaded.contract_hash.clone(),
        ),
    ];
    if let crate::ir::ContractSource::File(contract_path) = &loaded.source {
        env.push((
            "YGG_COMMAND_IR".into(),
            contract_path.display().to_string(),
        ));
    }
    // The adapter receives the full command path followed by the exact
    // post-path tokens, so its own CLI mirrors the porcelain invocation.
    let mut argv = path.to_vec();
    argv.extend_from_slice(rest);
    Dispatch::Exec {
        program,
        interpreter: Some(interpreter),
        argv,
        env,
        component: "inspection".into(),
    }
}

/// Validate a remote-operator host token. The exec call passes argv
/// verbatim (no shell, so quoting survives byte-exact); these checks close
/// the remaining confusion vector — a host that ssh would read as an option
/// is rejected outright, and the argv still carries an explicit `--`.
pub fn validate_remote_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("remote host is required".into());
    }
    if host.starts_with('-') {
        return Err(format!(
            "remote host {host:?} looks like an option; use: ygg remote <host> -- <arguments>"
        ));
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "remote host {host:?} contains whitespace or control characters"
        ));
    }
    Ok(())
}

/// OpenSSH joins command arguments into a remote shell command. Quote each
/// operand for that POSIX shell, even though the local process uses argv.
fn remote_command(args: &[String]) -> String {
    let mut command = String::from("exec ygg");
    for arg in args {
        command.push_str(" '");
        command.push_str(&arg.replace('\'', "'\\''"));
        command.push('\'');
    }
    command
}

/// The remote-operator door: run the remote node's installed `ygg` over
/// ssh. Local help stays local; the remote side resolves its own
/// generation. Requires only an ssh client — no local Python, GUI or
/// source checkout. The remote endpoint must use a POSIX command shell;
/// local Windows clients can inspect Linux worker nodes through this door.
pub fn plan_remote(rest: &[String]) -> Result<Dispatch, String> {
    let Some(host) = rest.first() else {
        return Err(
            "remote <host> -- <arguments> requires a host; try ygg help remote".into(),
        );
    };
    validate_remote_host(host)?;
    let mut remote_argv: Vec<String> = rest[1..].to_vec();
    if remote_argv.first().map(|s| s.as_str()) == Some("--") {
        remote_argv.remove(0);
    }
    let Some(ssh) = find_on_path("ssh") else {
        return Ok(Dispatch::Refused(
            "ygg: ssh client unavailable on PATH; the remote-operator door needs an ssh client"
                .into(),
        ));
    };
    let argv: Vec<String> = vec!["--".into(), host.clone(), remote_command(&remote_argv)];
    Ok(Dispatch::Exec {
        program: ssh,
        interpreter: None,
        argv,
        env: Vec::new(),
        component: "remote-operator".into(),
    })
}

/// Render the staged snapshot the way the Python router's
/// `json.dumps(..., sort_keys=True)` does, so identical requests produce
/// identical snapshot ids across front ends.
pub fn staged_snapshot_json(path: &[String], request_json: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(request_json.as_bytes());
    let digest = hasher.finalize();
    let short: String = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .chars()
        .take(16)
        .collect();
    let source = path.join(".");
    let findings_command = python_style_list(path);
    format!(
        "{{\"schema_version\": 1, \"snapshot_id\": \"unavailable-{short}\", \"as_of\": null, \"config_hash\": null, \"coverage\": [{{\"source\": \"{source}\", \"quality\": \"unavailable\", \"reason\": \"adapter_not_implemented\"}}], \"items\": [], \"next_cursor\": null, \"findings\": [{{\"code\": \"ADAPTER_NOT_IMPLEMENTED\", \"command\": {findings_command}}}]}}"
    )
}

fn python_style_list(items: &[String]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()))
        .collect();
    format!("[{}]", parts.join(", "))
}

fn python_style_object(args: &Args) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|(k, v)| format!("{}: {}", serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into()), v.python_json()))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// Hand the process to the resolved runner. On Unix this is a true exec:
/// the kernel replaces this image, so signals and exit status are the
/// component's own. Elsewhere the child is spawned with inherited stdio and
/// its status is propagated. `env` additions ride the child only.
pub fn exec(program: &Path, interpreter: Option<&str>, argv: &[String], env: &[(String, String)]) -> ! {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = match interpreter {
            Some(interpreter) => {
                let mut c = Command::new(interpreter);
                c.arg(program);
                c
            }
            None => Command::new(program),
        };
        command.args(argv);
        command.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let error = command.exec();
        eprintln!("ygg: owned component unavailable ({})", error);
        std::process::exit(3);
    }
    #[cfg(not(unix))]
    {
        let mut command = match interpreter {
            Some(interpreter) => {
                let mut c = Command::new(interpreter);
                c.arg(program);
                c
            }
            None => Command::new(program),
        };
        command.args(argv);
        command.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => {
                eprintln!("ygg: owned component unavailable ({error})");
                std::process::exit(3);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir;
    use std::sync::Mutex;

    /// Env mutation is process-global; cargo runs tests on parallel
    /// threads. Every test below that writes an env var holds this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn component(resolution: &[&str]) -> Component {
        Component {
            runner: "tool".into(),
            aliases: vec![],
            resolution: resolution.iter().map(|s| s.to_string()).collect(),
            interpreter: None,
            owner_managed: false,
            owns: vec![],
            reads: vec![],
        }
    }

    #[test]
    fn exe_candidates_add_the_platform_suffix_once() {
        let candidates = exe_candidates("ynpm");
        assert_eq!(candidates[0], "ynpm");
        let suffix = std::env::consts::EXE_SUFFIX;
        if suffix.is_empty() {
            assert_eq!(candidates.len(), 1);
        } else {
            assert_eq!(candidates, vec!["ynpm".to_string(), format!("ynpm{suffix}")]);
        }
    }

    #[test]
    fn env_dir_resolves_the_runner_inside_the_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let var = "YGG_TEST_COMPONENT_DIR";
        let dir = std::env::temp_dir().join(format!("ygg-cli-env-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("components")).expect("components dir");
        std::fs::write(dir.join("components").join("tool"), "#!/bin/sh\n").expect("tool file");
        // SAFETY: test-only var no other code in this target reads; restored
        // before the test returns.
        unsafe { std::env::set_var(var, dir.join("components").display().to_string()) };
        let resolved = resolve_runner(
            &component(&["env_dir:YGG_TEST_COMPONENT_DIR", "payload:tool"]),
            "tool",
        );
        unsafe { std::env::remove_var(var) };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(resolved.expect("resolved"), dir.join("components").join("tool"));
    }

    #[test]
    fn env_dir_set_but_broken_is_an_explicit_refusal_not_a_fallthrough() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let var = "YGG_TEST_COMPONENT_DIR";
        let dir = std::env::temp_dir().join(format!("ygg-cli-env-dir-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("empty dir");
        unsafe { std::env::set_var(var, dir.display().to_string()) };
        let resolved = resolve_runner(
            &component(&["env_dir:YGG_TEST_COMPONENT_DIR", "payload:tool"]),
            "tool",
        );
        unsafe { std::env::remove_var(var) };
        let _ = std::fs::remove_dir_all(&dir);
        let message = resolved.expect_err("must refuse");
        assert!(message.contains(var) && message.contains("tool"), "{message}");
    }

    #[test]
    fn wired_adapter_forwards_with_generation_identity() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let loaded = ir::finish(ir::EMBEDDED_IR.to_string(), ir::ContractSource::Embedded).expect("loads");
        let bin_dir =
            std::env::temp_dir().join(format!("ygg-cli-adapter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bin_dir);
        std::fs::create_dir_all(bin_dir.join("components")).expect("payload root");
        let rest = vec!["ls".to_string(), "--json".to_string()];
        let path = vec!["dream".to_string(), "ls".to_string()];
        // SAFETY: YGG_INSPECT_BIN/YGG_BIN_DIR are read only by resolution;
        // both are restored before the test returns.
        unsafe {
            std::env::set_var("YGG_INSPECT_BIN", "/nonexistent/ygg-inspect-override");
            std::env::set_var("YGG_BIN_DIR", bin_dir.display().to_string());
        }
        let absent = plan_query(&loaded, &path, &Args::new(), &rest);
        std::fs::write(bin_dir.join("components").join("ygg-inspect.py"), "# adapter\n")
            .expect("adapter file");
        let broken_override = plan_query(&loaded, &path, &Args::new(), &rest);
        unsafe { std::env::remove_var("YGG_INSPECT_BIN") };
        let forwarded = plan_query(&loaded, &path, &Args::new(), &rest);
        unsafe {
            std::env::remove_var("YGG_INSPECT_BIN");
            std::env::remove_var("YGG_BIN_DIR");
        }
        let _ = std::fs::remove_dir_all(&bin_dir);
        // Declared but absent is an explicit refusal, never a staged empty.
        assert!(matches!(absent, Dispatch::Refused(_)), "{absent:?}");
        assert!(matches!(broken_override, Dispatch::Refused(_)), "{broken_override:?}");
        match forwarded {
            Dispatch::Exec {
                program,
                interpreter,
                argv,
                env,
                component,
            } => {
                assert!(program.ends_with("ygg-inspect.py"), "{program:?}");
                assert_eq!(interpreter.as_deref(), Some("python3"));
                assert_eq!(argv, [path.as_slice(), rest.as_slice()].concat());
                assert_eq!(component, "inspection");
                let env: std::collections::BTreeMap<String, String> = env.into_iter().collect();
                assert_eq!(
                    env.get("YGG_COMMAND_CONTRACT_HASH").map(String::as_str),
                    Some(loaded.contract_hash.as_str())
                );
                assert_eq!(
                    env.get("YGG_KERNEL_DIR").map(String::as_str),
                    Some(bin_dir.join("kernel").display().to_string()).as_deref()
                );
            }
            other => panic!("expected exec, got {other:?}"),
        }
    }

    #[test]
    fn remote_host_validation_rejects_option_like_hosts() {
        assert!(validate_remote_host("build-host").is_ok());
        assert!(validate_remote_host("user@lab-node.example.test").is_ok());
        assert!(validate_remote_host("-oProxyCommand=evil").is_err());
        assert!(validate_remote_host("host with space").is_err());
        assert!(validate_remote_host("").is_err());
    }

    #[test]
    fn remote_rejects_unknown_options_before_the_delimiter() {
        // Local validation stays local: a typo like --json before `--` is
        // a usage error (exit 2), never forwarded to the remote ygg.
        let outcome = crate::grammar::parse(
            &crate::ir::finish(ir::EMBEDDED_IR.to_string(), ir::ContractSource::Embedded)
                .expect("loads"),
            &["remote".to_string(), "build-host".to_string(), "--json".to_string()],
        );
        assert!(matches!(outcome, crate::grammar::ParseOutcome::Invalid(_)));
    }

    #[test]
    fn remote_plan_execs_ssh_with_explicit_end_of_options() {
        let plan = plan_remote(&[
            "build-host".into(),
            "--".into(),
            "dream".into(),
            "ls".into(),
        ])
        .expect("ssh exists on the build host");
        match plan {
            Dispatch::Exec {
                program,
                interpreter,
                argv,
                env,
                component,
            } => {
                assert!(program.ends_with("ssh"), "{program:?}");
                assert_eq!(interpreter, None);
                assert_eq!(argv, vec!["--", "build-host", "exec ygg 'dream' 'ls'"]);
                assert!(env.is_empty());
                assert_eq!(component, "remote-operator");
            }
            other => panic!("expected exec, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn remote_shell_receives_literal_operands() {
        // A local argv boundary does not survive OpenSSH's command join.
        // Exercise the actual POSIX shell parse against metacharacters.
        let values: Vec<String> = ["", "two words", "it's literal", "$(exit 29)",
                                  "; exit 31", "*", "line\nbreak"]
            .into_iter().map(String::from).collect();
        let command = remote_command(&values);
        let probe = format!(
            "ygg() {{ for arg do printf '%s\\000' \"$arg\"; done; }}; {}",
            command.replacen("exec ygg", "ygg", 1)
        );
        let output = Command::new("sh").args(["-c", &probe]).output().expect("POSIX test shell");
        assert!(output.status.success());
        let expected: Vec<u8> = values.iter()
            .flat_map(|v| v.as_bytes().iter().copied().chain(std::iter::once(0)))
            .collect();
        assert_eq!(output.stdout, expected);
    }
}
