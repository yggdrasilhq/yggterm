//! `ygg` — the native porcelain entry. Order of routes mirrors the
//! compatibility router exactly: local help/completion/version first, then
//! the native/component generation gate, then parse, then effects.

mod dispatch;
mod grammar;
mod ir;
mod payload;
mod seed_launcher;

use grammar::{ParseOutcome, ArgValue};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

fn out(text: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{text}");
}

fn out_print(text: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = write!(lock, "{text}");
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(argv) {
        Ok(code) => ExitCode::from(code as u8),
        Err(message) => {
            eprintln!("ygg: {message}");
            ExitCode::from(3)
        }
    }
}

fn run(mut argv: Vec<String>) -> Result<u32, String> {
    // The seed front launcher is a bootstrap surface that resolves its
    // target BEFORE any installed IR loads: an older launcher generation
    // must forward to a newer seed generation whose command contract it
    // has never seen, not reject it through the IR gate below.
    if argv.first().map(|s| s.as_str()) == Some("--seed-launch") {
        return seed_launcher::launch(&argv[1..]);
    }

    // A stable Windows alias frontend — a launcher copy with an immutable
    // `.seed-alias` metadata file beside it — forwards to its recorded seed
    // program under the SAME bootstrap guarantee: pointer, root marker and
    // hash gate first, command IR never. Windows-only; unix installs carry
    // no seed-alias files, so unix behavior is untouched.
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        match seed_launcher::resolve_alias(&exe) {
            Some(Ok(target)) => return seed_launcher::forward_program(&target, &argv),
            Some(Err(message)) => return Err(message),
            None => {}
        }
    }

    let loaded = ir::load().map_err(|e| format!("{e}"))?;

    if argv.first().map(|s| s.as_str()) == Some("--version") {
        argv[0] = "version".into();
    }
    // Respect the literal-arguments delimiter. No component, adapter or
    // network is touched on ANY help route, including legacy leaves.
    let delimiter = argv.iter().position(|a| a == "--");
    let before_delimiter = match delimiter {
        Some(i) => argv[..i].to_vec(),
        None => argv.clone(),
    };
    if argv.is_empty() || before_delimiter.iter().any(|x| x == "-h" || x == "--help") {
        let path = grammar::context(&loaded, &before_delimiter);
        out_print(&grammar::help_text(&loaded, &path));
        return Ok(0);
    }
    if argv[0] == "help" && (argv.len() == 1 || argv[1] != "examples") {
        let path = grammar::context(&loaded, &argv[1..]);
        if !argv[1..].is_empty() && path.is_empty() {
            eprintln!("ygg: unknown help topic");
            return Ok(2);
        }
        out_print(&grammar::help_text(&loaded, &path));
        return Ok(0);
    }
    if loaded.tree.contains_key(&argv) && loaded.tree[&argv].record.handler == "group" {
        out_print(&grammar::help_text(&loaded, &argv));
        return Ok(0);
    }
    if argv[0] == "complete" {
        let mut words = argv[1..].to_vec();
        if words.first().map(|s| s.as_str()) == Some("--") {
            words.remove(0);
        }
        out(&grammar::complete(&loaded, &words).join("\n"));
        return Ok(0);
    }
    // Preserve the old row-board flag form while noun-only board teaches
    // the topic-board verbs.
    if argv[0] == "board" && argv.len() > 1 && argv[1].starts_with('-') {
        argv.insert(1, "rows".into());
    }
    let parsed = match grammar::parse(&loaded, &argv) {
        ParseOutcome::Parsed { path, args, .. } => (path, args),
        ParseOutcome::PlannedFamily { message, .. } => {
            eprintln!("ygg: {message}");
            return Ok(3);
        }
        ParseOutcome::Invalid(message) => {
            eprintln!("ygg: {message}");
            return Ok(2);
        }
    };
    let (path, args) = parsed;
    let node = &loaded.tree[&path];

    // Local diagnostic reads remain available to repair a mismatched
    // installation. Queries, dispatch and payload installation fail closed.
    let diagnostic = matches!(node.record.handler.as_str(), "version" | "examples")
        || (node.record.handler == "native"
            && matches!(node.record.native.as_str(), "payload_manifest" | "payload_verify" | "docs"));
    if !diagnostic {
        if let Ok(expected) = std::env::var("YGG_COMMAND_CONTRACT_HASH") {
            if !expected.is_empty() && expected != loaded.contract_hash {
                eprintln!("ygg: native/component command contracts differ; install a matching generation");
                return Ok(3);
            }
        }
    }

    match node.record.handler.as_str() {
        "version" => {
            let wants_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
            if wants_json {
                out(&serde_json::to_string_pretty(&serde_json::json!({
                    "schema_version": 1,
                    "frontend": {
                        "name": "ygg",
                        "version": env!("CARGO_PKG_VERSION"),
                        "os": std::env::consts::OS,
                    },
                    "command_contract": {
                        "name": loaded.document.contract,
                        "schema_version": ir::SUPPORTED_SCHEMA,
                        "sha256": loaded.contract_hash,
                        "source": contract_source_label(&loaded.source),
                    },
                }))
                .unwrap_or_default());
            } else {
                // Frontend version and command-contract identity are separate
                // facts; neither may masquerade as the other in the human
                // form either.
                out(&format!(
                    "ygg frontend {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS
                ));
                out(&format!(
                    "command contract {} sha256 {} source {}",
                    loaded.document.contract,
                    &loaded.contract_hash[..12.min(loaded.contract_hash.len())],
                    contract_source_label(&loaded.source),
                ));
            }
            Ok(0)
        }
        "examples" => {
            let phrase = args
                .get("search")
                .map(|v| match v {
                    ArgValue::Str(s) => s.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            let found = grammar::search_examples(&loaded, &phrase);
            let wants_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
            if wants_json {
                out(&serde_json::to_string_pretty(&serde_json::json!({
                    "schema_version": 1,
                    "examples": found,
                }))
                .unwrap_or_default());
            } else if found.is_empty() {
                out("No matching local example; try fewer words.");
            } else {
                for e in found {
                    out(&format!(
                        "{} [{}]\n  {}",
                        e["title"].as_str().unwrap_or_default(),
                        e["effect"].as_str().unwrap_or_default(),
                        e["command"].as_str().unwrap_or_default()
                    ));
                }
            }
            Ok(0)
        }
        "completion" => {
            out(
                "_ygg_complete() {\n  mapfile -t COMPREPLY < <(ygg complete -- \"${COMP_WORDS[@]:1:$COMP_CWORD}\")\n}\ncomplete -F _ygg_complete ygg",
            );
            Ok(0)
        }
        "native" => run_native(&loaded, &path, &args),
        "legacy" => match dispatch::plan_legacy(&loaded, &path, &rest_of(&argv, &path)) {
            dispatch::Dispatch::Exec {
                program,
                interpreter,
                argv,
                ..
            } => dispatch::exec(&program, interpreter.as_deref(), &argv, &[]),
            dispatch::Dispatch::Refused(message) => {
                eprintln!("{message}");
                Ok(3)
            }
            _ => unreachable!("legacy dispatch never stages"),
        },
        "remote" => match dispatch::plan_remote(&rest_of(&argv, &path)) {
            Ok(dispatch::Dispatch::Exec {
                program,
                interpreter,
                argv,
                ..
            }) => dispatch::exec(&program, interpreter.as_deref(), &argv, &[]),
            Ok(_) => unreachable!("remote dispatch never stages"),
            Err(message) => {
                eprintln!("ygg: {message}");
                Ok(2)
            }
        },
        handler if handler == "query" || handler == "open" => {
            match dispatch::plan_query(&loaded, &path, &args, &rest_of(&argv, &path)) {
                dispatch::Dispatch::Exec {
                    program,
                    interpreter,
                    argv,
                    env,
                    ..
                } => dispatch::exec(&program, interpreter.as_deref(), &argv, &env),
                dispatch::Dispatch::StagedUnavailable { path, request_json } => {
                    let snapshot = dispatch::staged_snapshot_json(&path, &request_json);
                    let wants_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
                    if wants_json {
                        out(&serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&snapshot).unwrap_or_default()).unwrap_or_default());
                    } else {
                        out(&format!(
                            "ygg {}: source adapter not implemented; coverage UNKNOWN.",
                            path.join(" ")
                        ));
                        out("Help and examples are available; an empty result would not prove zero work.");
                    }
                    Ok(3)
                }
                dispatch::Dispatch::Refused(message) => {
                    eprintln!("{message}");
                    Ok(3)
                }
            }
        }
        other => {
            eprintln!(
                "ygg: unsupported handler role for ygg {}: {other}; install a matching generation",
                path.join(" ")
            );
            Ok(3)
        }
    }
}

fn rest_of(argv: &[String], path: &[String]) -> Vec<String> {
    argv[path.len()..].to_vec()
}

fn contract_source_label(source: &ir::ContractSource) -> String {
    match source {
        ir::ContractSource::Embedded => "embedded".into(),
        ir::ContractSource::File(p) => p.display().to_string(),
    }
}

fn run_native(loaded: &ir::Loaded, path: &[String], args: &grammar::Args) -> Result<u32, String> {
    let native = loaded.tree[&path.to_vec()].record.native.clone();
    let wants_json = matches!(args.get("json"), Some(ArgValue::Bool(true)));
    match native.as_str() {
        "payload_manifest" => {
            let manifest = payload::manifest();
            if wants_json {
                out(&serde_json::to_string_pretty(&manifest).unwrap_or_default());
            } else {
                out(&format!(
                    "payload contract {} schema {} (verify with: ygg payload verify)",
                    manifest.contract, manifest.schema_version
                ));
                for e in &manifest.payload {
                    out(&format!(
                        "  {:<36} {:>9} bytes  sha256:{}",
                        e.path,
                        e.bytes,
                        &e.sha256[..12.min(e.sha256.len())]
                    ));
                }
            }
            Ok(0)
        }
        "payload_verify" | "payload_install" => {
            let dir: Option<PathBuf> = args.get("dir").and_then(|v| match v {
                ArgValue::Str(s) if !s.is_empty() => Some(PathBuf::from(s)),
                _ => None,
            });
            let outcome = if native == "payload_verify" {
                let dir = dir.or_else(ir::payload_dir).unwrap_or_else(|| PathBuf::from("."));
                payload::verify(&dir)
            } else {
                match dir {
                    Some(dir) => payload::install(&dir),
                    None => {
                        eprintln!("ygg: --dir is required\n{}", grammar::parser_usage(loaded, path));
                        return Ok(2);
                    }
                }
            };
            match outcome {
                payload::PayloadOutcome::Done(report) => {
                    if wants_json {
                        out(&serde_json::to_string_pretty(&report).unwrap_or_default());
                    } else {
                        for row in &report.rows {
                            out(&format!("{:<8} {}", row.status, row.path));
                        }
                        out(&format!(
                            "{} verified, {} failed in {}",
                            report.verified, report.failed, report.dir
                        ));
                    }
                    Ok(if report.failed == 0 { 0 } else { 1 })
                }
                payload::PayloadOutcome::Unavailable(message)
                | payload::PayloadOutcome::Refused(message) => {
                    eprintln!("{message}");
                    Ok(3)
                }
                payload::PayloadOutcome::IoError(message) => {
                    eprintln!("{message}");
                    Ok(3)
                }
            }
        }
        other => {
            eprintln!(
                "ygg: unsupported native handler for ygg {}: {other}; install a matching generation",
                path.join(" ")
            );
            Ok(3)
        }
    }
}
