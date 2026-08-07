use crate::{
    ManagedCliTool, run_remote_ensure_managed_cli, run_remote_generation_context,
    run_remote_cc_rename, run_remote_local_codex_identities, run_remote_preview,
    run_remote_preview_head,
    run_remote_preview_tail, run_remote_protocol_version, run_remote_refresh_managed_cli,
    run_remote_resume_agent, run_remote_resume_cc, run_remote_resume_codex,
    run_remote_saved_agent_session_exists, run_remote_saved_codex_session_exists,
    run_remote_scan, run_remote_stage_clipboard_png, run_remote_start_agent, run_remote_start_cc,
    run_remote_start_codex,
    run_remote_agent_runtime_alive, run_remote_apps, run_remote_terminate_agent,
    run_remote_terminate_cc,
    run_remote_terminate_codex, run_remote_upsert_generated_copy,
};
use anyhow::{Result, bail};
use yggterm_core::SessionKind;
use yggterm_core::agent_cli::AGENT_CLIS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteServerCommand {
    StageClipboardPng,
    ProtocolVersion,
    SavedCodexSessionExists {
        session_id: String,
    },
    ResumeCodex {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
    },
    StartCodex {
        session_id: String,
        cwd: Option<String>,
    },
    /// Claude Code twins of ResumeCodex/StartCodex — the same host-daemon
    /// runtime lane, so the claude PTY survives client restarts.
    ResumeCc {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
    },
    StartCc {
        session_id: String,
        cwd: Option<String>,
    },
    RefreshManagedCli {
        background: bool,
    },
    EnsureManagedCli {
        tool: ManagedCliTool,
    },
    Scan {
        codex_home: Option<String>,
    },
    /// Emit THIS host's libyggterm app registry (`~/.yggterm/apps/*.json`), one
    /// manifest per line.
    ///
    /// The registry describes what is installed *here*, with absolute binary
    /// paths that only mean anything *here*. The GUI host's daemon therefore
    /// cannot answer "which apps does machine M have" from its own directory,
    /// which is exactly what it used to do: a right-click on a `remote-cc://dev`
    /// row drew the GUI host's app list, so an app installed only on `dev` was
    /// invisible and an app installed only on the GUI host was offered — with
    /// the GUI host's path — to run on `dev`.
    Apps,
    /// Enumerate Codex/Claude Code processes running on the remote machine and
    /// emit their real CLI session ids. Used by the local daemon to rebind
    /// live remote-Codex rows that still carry a synthesized UUIDv4 id
    /// (`[[finding-uuidv4-codex-session-drift]]` Stage 2).
    LocalCodexIdentities,
    /// Append a Claude Code `custom-title` (user rename) to a session's JSONL
    /// on this (remote) machine — the SSH-invoked half of yggterm's CC rename
    /// write-back. See memory finding-cc-title-storage-custom-title.
    CcRename {
        session_id: String,
        title: String,
    },
    PreviewHead {
        session_id: String,
        blocks: usize,
    },
    PreviewTail {
        session_id: String,
        blocks: usize,
    },
    Preview {
        session_id: String,
    },
    GenerationContext {
        session_id: String,
    },
    TerminateCodex {
        session_id: String,
    },
    /// Claude Code twin of `TerminateCodex`. Its absence meant a `remote-cc://`
    /// row's close had nothing to call across the ssh hop, so the remote claude
    /// outlived the row that owned it while the teardown reported `verified:true`.
    TerminateCc {
        session_id: String,
    },
    /// Does a daemon on THIS host still own `runtime_key`? The fact a remote
    /// teardown is verified against — the local side can only prove it reaped
    /// its own ssh client. Sweeps every coexisting daemon, so an older daemon
    /// still holding the PTY answers `alive: true` rather than going unseen.
    AgentRuntimeAlive {
        runtime_key: String,
    },
    UpsertGeneratedCopy {
        session_id: String,
    },
    /// The four wrapper verbs of every CLI registered AFTER the codex/cc pair
    /// above, carried as ONE kind-bearing variant each.
    ///
    /// The pair above keeps its own variants so `resume-codex` / `resume-cc`
    /// parse and dispatch byte-for-byte — those strings are on the wire between
    /// machines that may be running different builds. Everything after them
    /// derives its verbs from `wrapper_slug`, so registering a CLI is what makes
    /// `resume-<slug>` work; there is no per-CLI literal to remember.
    ResumeAgent {
        kind: SessionKind,
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
    },
    StartAgent {
        kind: SessionKind,
        session_id: String,
        cwd: Option<String>,
    },
    TerminateAgent {
        kind: SessionKind,
        session_id: String,
    },
    SavedAgentSessionExists {
        kind: SessionKind,
        session_id: String,
    },
}

/// Which registered CLI a wrapper verb belongs to, and which of the four verbs
/// it is.
///
/// Derived from `wrapper_slug` — `resume-<slug>`, `start-<slug>`,
/// `terminate-<slug>`, `<slug>-session-exists` — rather than 24 more literal
/// match arms. The three shipped CLIs are matched here too and then routed to
/// their own dedicated variants below, so this parser and the literals stay
/// provably in agreement instead of being two spellings of one verb table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperVerb {
    Resume,
    Start,
    Terminate,
    SessionExists,
}

fn parse_wrapper_verb(verb: &str) -> Option<(SessionKind, WrapperVerb)> {
    AGENT_CLIS.iter().find_map(|descriptor| {
        let which = if descriptor.resume_subcommand().as_deref() == Some(verb) {
            WrapperVerb::Resume
        } else if descriptor.start_subcommand().as_deref() == Some(verb) {
            WrapperVerb::Start
        } else if descriptor.terminate_subcommand().as_deref() == Some(verb) {
            WrapperVerb::Terminate
        } else if descriptor.session_exists_subcommand().as_deref() == Some(verb) {
            WrapperVerb::SessionExists
        } else {
            return None;
        };
        Some((descriptor.kind, which))
    })
}

/// The optional positional cwd that follows a session id: the first argument
/// after it that is not a flag. Spelled once because all four verbs read it the
/// same way, and the four hand-copied closures it replaced are where a fifth
/// would have been copied slightly differently.
fn positional_cwd(args: &[String]) -> Option<String> {
    args.iter()
        .skip(4)
        .find(|value| !value.starts_with("--"))
        .cloned()
        .filter(|value| !value.is_empty())
}

pub fn try_run_remote_server_command(args: &[String]) -> Result<bool> {
    let Some(command) = parse_remote_server_command(args)? else {
        return Ok(false);
    };
    run_remote_server_command(command)?;
    Ok(true)
}

fn parse_remote_server_command(args: &[String]) -> Result<Option<RemoteServerCommand>> {
    if args.len() < 3 || args[0] != "server" || args[1] != "remote" {
        return Ok(None);
    }
    let command = match args[2].as_str() {
        "stage-clipboard-png" if args.len() == 3 => RemoteServerCommand::StageClipboardPng,
        "protocol-version" if args.len() == 3 => RemoteServerCommand::ProtocolVersion,
        "codex-session-exists" if args.len() == 4 => RemoteServerCommand::SavedCodexSessionExists {
            session_id: args[3].clone(),
        },
        "resume-codex" if args.len() >= 4 => RemoteServerCommand::ResumeCodex {
            session_id: args[3].clone(),
            cwd: args
                .iter()
                .skip(4)
                .find(|value| !value.starts_with("--"))
                .cloned()
                .filter(|value| !value.is_empty()),
            require_existing: args.iter().any(|value| value == "--require-existing"),
        },
        "start-codex" if args.len() >= 4 => RemoteServerCommand::StartCodex {
            session_id: args[3].clone(),
            cwd: args
                .iter()
                .skip(4)
                .find(|value| !value.starts_with("--"))
                .cloned()
                .filter(|value| !value.is_empty()),
        },
        "resume-cc" if args.len() >= 4 => RemoteServerCommand::ResumeCc {
            session_id: args[3].clone(),
            cwd: args
                .iter()
                .skip(4)
                .find(|value| !value.starts_with("--"))
                .cloned()
                .filter(|value| !value.is_empty()),
            require_existing: args.iter().any(|value| value == "--require-existing"),
        },
        "start-cc" if args.len() >= 4 => RemoteServerCommand::StartCc {
            session_id: args[3].clone(),
            cwd: args
                .iter()
                .skip(4)
                .find(|value| !value.starts_with("--"))
                .cloned()
                .filter(|value| !value.is_empty()),
        },
        "refresh-managed-cli" if args.len() >= 4 => RemoteServerCommand::RefreshManagedCli {
            background: args[3] == "background",
        },
        "ensure-managed-cli" if args.len() >= 4 => RemoteServerCommand::EnsureManagedCli {
            tool: parse_managed_cli_tool(&args[3])?,
        },
        "scan" => RemoteServerCommand::Scan {
            codex_home: args.get(3).cloned(),
        },
        "apps" if args.len() == 3 => RemoteServerCommand::Apps,
        "local-codex-identities" if args.len() == 3 => RemoteServerCommand::LocalCodexIdentities,
        "cc-rename" if args.len() == 5 => RemoteServerCommand::CcRename {
            session_id: args[3].clone(),
            title: args[4].clone(),
        },
        "preview-head" if args.len() >= 4 => RemoteServerCommand::PreviewHead {
            session_id: args[3].clone(),
            blocks: args
                .get(4)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(8),
        },
        "preview-tail" if args.len() >= 4 => RemoteServerCommand::PreviewTail {
            session_id: args[3].clone(),
            blocks: args
                .get(4)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(48),
        },
        "preview" if args.len() == 4 => RemoteServerCommand::Preview {
            session_id: args[3].clone(),
        },
        "generation-context" if args.len() == 4 => RemoteServerCommand::GenerationContext {
            session_id: args[3].clone(),
        },
        "terminate-codex" if args.len() == 4 => RemoteServerCommand::TerminateCodex {
            session_id: args[3].clone(),
        },
        "terminate-cc" if args.len() == 4 => RemoteServerCommand::TerminateCc {
            session_id: args[3].clone(),
        },
        "agent-runtime-alive" if args.len() == 4 => RemoteServerCommand::AgentRuntimeAlive {
            runtime_key: args[3].clone(),
        },
        "upsert-generated-copy" if args.len() == 4 => RemoteServerCommand::UpsertGeneratedCopy {
            session_id: args[3].clone(),
        },
        // Every OTHER registered CLI's four verbs, resolved against the wrapper
        // registry. Reached only when none of the literals above matched, so
        // the shipped spellings keep their exact arms and their exact arity
        // rules; this arm is what makes `resume-pi` / `start-agy` /
        // `qwen-session-exists` exist at all, and it cannot fall behind the
        // registry because it IS the registry.
        other => match parse_wrapper_verb(other) {
            Some((kind, WrapperVerb::Resume)) if args.len() >= 4 => {
                RemoteServerCommand::ResumeAgent {
                    kind,
                    session_id: args[3].clone(),
                    cwd: positional_cwd(args),
                    require_existing: args.iter().any(|value| value == "--require-existing"),
                }
            }
            Some((kind, WrapperVerb::Start)) if args.len() >= 4 => {
                RemoteServerCommand::StartAgent {
                    kind,
                    session_id: args[3].clone(),
                    cwd: positional_cwd(args),
                }
            }
            Some((kind, WrapperVerb::Terminate)) if args.len() == 4 => {
                RemoteServerCommand::TerminateAgent {
                    kind,
                    session_id: args[3].clone(),
                }
            }
            Some((kind, WrapperVerb::SessionExists)) if args.len() == 4 => {
                RemoteServerCommand::SavedAgentSessionExists {
                    kind,
                    session_id: args[3].clone(),
                }
            }
            _ => return Ok(None),
        },
    };
    Ok(Some(command))
}

fn run_remote_server_command(command: RemoteServerCommand) -> Result<()> {
    match command {
        RemoteServerCommand::StageClipboardPng => run_remote_stage_clipboard_png(),
        RemoteServerCommand::ProtocolVersion => run_remote_protocol_version(),
        RemoteServerCommand::SavedCodexSessionExists { session_id } => {
            run_remote_saved_codex_session_exists(&session_id)
        }
        RemoteServerCommand::ResumeCodex {
            session_id,
            cwd,
            require_existing,
        } => run_remote_resume_codex(&session_id, cwd.as_deref(), require_existing),
        RemoteServerCommand::StartCodex { session_id, cwd } => {
            run_remote_start_codex(&session_id, cwd.as_deref())
        }
        RemoteServerCommand::ResumeCc {
            session_id,
            cwd,
            require_existing,
        } => run_remote_resume_cc(&session_id, cwd.as_deref(), require_existing),
        RemoteServerCommand::StartCc { session_id, cwd } => {
            run_remote_start_cc(&session_id, cwd.as_deref())
        }
        RemoteServerCommand::RefreshManagedCli { background } => {
            run_remote_refresh_managed_cli(background)
        }
        RemoteServerCommand::EnsureManagedCli { tool } => run_remote_ensure_managed_cli(tool),
        RemoteServerCommand::Scan { codex_home } => run_remote_scan(codex_home.as_deref()),
        RemoteServerCommand::Apps => run_remote_apps(),
        RemoteServerCommand::LocalCodexIdentities => run_remote_local_codex_identities(),
        RemoteServerCommand::CcRename { session_id, title } => {
            run_remote_cc_rename(&session_id, &title)
        }
        RemoteServerCommand::PreviewHead { session_id, blocks } => {
            run_remote_preview_head(&session_id, blocks)
        }
        RemoteServerCommand::PreviewTail { session_id, blocks } => {
            run_remote_preview_tail(&session_id, blocks)
        }
        RemoteServerCommand::Preview { session_id } => run_remote_preview(&session_id),
        RemoteServerCommand::GenerationContext { session_id } => {
            run_remote_generation_context(&session_id)
        }
        RemoteServerCommand::TerminateCodex { session_id } => {
            run_remote_terminate_codex(&session_id)
        }
        RemoteServerCommand::TerminateCc { session_id } => run_remote_terminate_cc(&session_id),
        RemoteServerCommand::AgentRuntimeAlive { runtime_key } => {
            run_remote_agent_runtime_alive(&runtime_key)
        }
        RemoteServerCommand::UpsertGeneratedCopy { session_id } => {
            run_remote_upsert_generated_copy(&session_id)
        }
        RemoteServerCommand::ResumeAgent {
            kind,
            session_id,
            cwd,
            require_existing,
        } => run_remote_resume_agent(kind, &session_id, cwd.as_deref(), require_existing),
        RemoteServerCommand::StartAgent {
            kind,
            session_id,
            cwd,
        } => run_remote_start_agent(kind, &session_id, cwd.as_deref()),
        RemoteServerCommand::TerminateAgent { kind, session_id } => {
            run_remote_terminate_agent(&session_id, kind)
        }
        RemoteServerCommand::SavedAgentSessionExists { kind, session_id } => {
            run_remote_saved_agent_session_exists(kind, &session_id)
        }
    }
}

/// Resolve `ensure-managed-cli <name>` to a provisioning key.
///
/// Registry-first, so every CLI's slug works the moment it is registered; the
/// three literals below are legacy spellings already on the wire between
/// machines (`claude` is the BINARY name, not the slug).
fn parse_managed_cli_tool(value: &str) -> Result<ManagedCliTool> {
    if let Some(descriptor) = AGENT_CLIS
        .iter()
        .find(|descriptor| descriptor.slug == value || descriptor.binary_name == value)
        && let Some(tool) = ManagedCliTool::from_session_kind(descriptor.kind)
    {
        return Ok(tool);
    }
    match value {
        "codex" => Ok(ManagedCliTool::Codex),
        "codex-litellm" => Ok(ManagedCliTool::CodexLiteLlm),
        "claude" | "claude-code" => Ok(ManagedCliTool::ClaudeCode),
        other => bail!("unknown managed cli tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resume_cc_and_start_cc_mirror_codex() {
        let resume = parse_remote_server_command(&[
            "server".to_string(),
            "remote".to_string(),
            "resume-cc".to_string(),
            "abc-123".to_string(),
            "/srv/ws".to_string(),
            "--require-existing".to_string(),
        ])
        .expect("parse")
        .expect("command");
        assert_eq!(
            resume,
            RemoteServerCommand::ResumeCc {
                session_id: "abc-123".to_string(),
                cwd: Some("/srv/ws".to_string()),
                require_existing: true,
            }
        );
        let start = parse_remote_server_command(&[
            "server".to_string(),
            "remote".to_string(),
            "start-cc".to_string(),
            "abc-123".to_string(),
            "/srv/ws".to_string(),
        ])
        .expect("parse")
        .expect("command");
        assert_eq!(
            start,
            RemoteServerCommand::StartCc {
                session_id: "abc-123".to_string(),
                cwd: Some("/srv/ws".to_string()),
            }
        );
    }

    #[test]
    fn parse_resume_codex_supports_require_existing_and_cwd() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "resume-codex".to_string(),
            "019ad8".to_string(),
            "/home/user".to_string(),
            "--require-existing".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::ResumeCodex {
                session_id: "019ad8".to_string(),
                cwd: Some("/home/user".to_string()),
                require_existing: true,
            }
        );
    }

    #[test]
    fn parse_cc_rename_command() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "cc-rename".to_string(),
            "654669a2-f2d4-4d40-a19c-ad1d4ba3d833".to_string(),
            "My Renamed Session".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::CcRename {
                session_id: "654669a2-f2d4-4d40-a19c-ad1d4ba3d833".to_string(),
                title: "My Renamed Session".to_string(),
            }
        );
    }

    #[test]
    fn parse_saved_codex_session_exists_command() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "codex-session-exists".to_string(),
            "019ad8".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::SavedCodexSessionExists {
                session_id: "019ad8".to_string(),
            }
        );
    }

    #[test]
    fn parse_preview_tail_command_defaults_to_recent_window() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "preview-tail".to_string(),
            "/home/user/.codex/sessions/example.jsonl".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::PreviewTail {
                session_id: "/home/user/.codex/sessions/example.jsonl".to_string(),
                blocks: 48,
            }
        );
    }

    #[test]
    fn parse_ensure_managed_cli_recognizes_tools() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "ensure-managed-cli".to_string(),
            "codex-litellm".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::EnsureManagedCli {
                tool: ManagedCliTool::CodexLiteLlm,
            }
        );
    }

    #[test]
    fn parse_local_codex_identities_command() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "local-codex-identities".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(command, RemoteServerCommand::LocalCodexIdentities);
    }

    #[test]
    fn parse_local_codex_identities_rejects_extra_args() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "local-codex-identities".to_string(),
            "unexpected".to_string(),
        ];
        assert!(
            parse_remote_server_command(&args)
                .expect("parse command")
                .is_none()
        );
    }

    /// Every registered CLI's four wrapper verbs must PARSE. The bug this locks
    /// out is not a wrong answer but a missing one: an unparsed verb falls
    /// through to `Ok(None)`, the wrapper reports "unknown command" on the far
    /// side of an ssh hop, and the row it was launching never appears.
    ///
    /// Byte-for-byte on the two shipped spellings is asserted separately above
    /// (`parse_resume_cc_and_start_cc_mirror_codex`,
    /// `parse_resume_codex_supports_require_existing_and_cwd`); this one proves
    /// the SET is complete.
    #[test]
    fn every_registered_cli_has_all_four_wrapper_verbs_parsing() {
        for descriptor in AGENT_CLIS {
            let Some(wrapper) = descriptor.wrapper_slug else {
                // ⛔ LOCAL-ONLY (codex-litellm). It has no remote arm at all, so
                // it must contribute NO verbs — asserted, because a local-only
                // CLI that quietly grew wrapper verbs would be reachable over
                // ssh with no row scheme to name the result.
                assert!(
                    descriptor.resume_subcommand().is_none()
                        && descriptor.start_subcommand().is_none()
                        && descriptor.terminate_subcommand().is_none()
                        && descriptor.session_exists_subcommand().is_none(),
                    "{:?} is local-only yet names a wrapper verb",
                    descriptor.kind
                );
                continue;
            };
            let verbs = [
                (format!("resume-{wrapper}"), true),
                (format!("start-{wrapper}"), true),
                (format!("terminate-{wrapper}"), false),
                (format!("{wrapper}-session-exists"), false),
            ];
            for (verb, takes_cwd) in verbs {
                let mut args = vec![
                    "server".to_string(),
                    "remote".to_string(),
                    verb.clone(),
                    "00000000-0000-4000-8000-000000000001".to_string(),
                ];
                if takes_cwd {
                    args.push("/home/user/gh/yggterm".to_string());
                }
                let parsed = parse_remote_server_command(&args)
                    .unwrap_or_else(|error| panic!("{verb}: parse failed: {error}"));
                assert!(
                    parsed.is_some(),
                    "{verb} does not parse — the wrapper would answer 'unknown command' \
                     across the ssh hop and the session would never appear",
                );
            }
        }
    }

    /// The generic parser must resolve a NEW CLI's verb to that CLI, not to
    /// codex. `resume-pi` reaching `ResumeCodex` is the exact silent failure the
    /// `_ =>` catch-alls produced everywhere else.
    #[test]
    fn a_new_clis_verb_resolves_to_that_cli() {
        let command = parse_remote_server_command(&[
            "server".to_string(),
            "remote".to_string(),
            "resume-pi".to_string(),
            "abc-123".to_string(),
            "/srv/ws".to_string(),
            "--require-existing".to_string(),
        ])
        .expect("parse")
        .expect("command");
        assert_eq!(
            command,
            RemoteServerCommand::ResumeAgent {
                kind: SessionKind::Pi,
                session_id: "abc-123".to_string(),
                cwd: Some("/srv/ws".to_string()),
                require_existing: true,
            }
        );
        let exists = parse_remote_server_command(&[
            "server".to_string(),
            "remote".to_string(),
            "agy-session-exists".to_string(),
            "abc-123".to_string(),
        ])
        .expect("parse")
        .expect("command");
        assert_eq!(
            exists,
            RemoteServerCommand::SavedAgentSessionExists {
                kind: SessionKind::Antigravity,
                session_id: "abc-123".to_string(),
            }
        );
    }

    #[test]
    fn parse_non_remote_command_returns_none() {
        let args = vec!["server".to_string(), "status".to_string()];
        assert!(
            parse_remote_server_command(&args)
                .expect("parse command")
                .is_none()
        );
    }
}
