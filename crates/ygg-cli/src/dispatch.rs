//! Owned component dispatch: resolve the declared runner for a command,
//! verify it, then hand the process over while preserving argv, signals and
//! exit status. Resolution never searches PATH for the porcelain itself and
//! never executes a runner that resolves back to this binary.

use crate::grammar::Args;
use crate::ir::{Component, Loaded};
use std::path::{Path, PathBuf};
use std::process::Command;

pub enum Dispatch {
    /// Execute this runner with these fixed prefix + trailing arguments.
    Exec {
        program: PathBuf,
        interpreter: Option<String>,
        argv: Vec<String>,
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
        .map(|h| Path::new(&h).join(".local/bin").join(name))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn exe_sibling(name: &str) -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
}

/// Evaluate a component's declared resolution strategies in order. An
/// explicitly set env var wins even when the file is missing: an override
/// that silently falls through to a payload copy would hide a broken
/// installation from the operator.
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
        let candidate = if let Some(name) = strategy.strip_prefix("home_bin:") {
            home_bin(name)
        } else if let Some(name) = strategy.strip_prefix("path:") {
            find_on_path(name)
        } else if let Some(name) = strategy.strip_prefix("exe_sibling:") {
            exe_sibling(name).filter(|p| p.is_file())
        } else if strategy == "payload" || strategy.starts_with("payload:") {
            let name = strategy.strip_prefix("payload:").unwrap_or(tool);
            let candidate = payload_root().join(name);
            candidate.is_file().then_some(candidate)
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
        component: component_id.clone(),
    }
}

/// Wire the inspection families through the contract's `wired_adapters`.
/// This generation ships none: a wired entry names an adapter this frontend
/// does not implement, which is an explicit refusal, never a fake empty
/// ledger. Until an adapter lands, queries stage honest unavailable
/// coverage with exit 3.
pub fn plan_query(loaded: &Loaded, path: &[String], args: &Args) -> Dispatch {
    let family = path.first().map(|s| s.as_str()).unwrap_or("");
    if loaded.document.wired_adapters.contains_key(family) {
        return Dispatch::Refused(format!(
            "ygg: adapter for family {family} is declared but this frontend generation does not implement it; install a matching generation"
        ));
    }
    let request = format!(
        "{{\"arguments\": {}, \"command\": {}}}",
        python_style_object(args),
        python_style_list(path)
    );
    Dispatch::StagedUnavailable {
        path: path.to_vec(),
        request_json: request,
    }
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
    let findings_command = request_json
        .split("\"command\": ")
        .nth(1)
        .and_then(|rest| rest.strip_suffix("}"))
        .unwrap_or("[]");
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
/// its status is propagated.
pub fn exec(program: &Path, interpreter: Option<&str>, argv: &[String]) -> ! {
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
        match command.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(error) => {
                eprintln!("ygg: owned component unavailable ({error})");
                std::process::exit(3);
            }
        }
    }
}
