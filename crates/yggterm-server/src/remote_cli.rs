use crate::{
    ManagedCliTool, run_remote_ensure_managed_cli, run_remote_generation_context,
    run_remote_cc_rename, run_remote_local_codex_identities, run_remote_preview,
    run_remote_preview_head,
    run_remote_preview_tail, run_remote_protocol_version, run_remote_refresh_managed_cli,
    run_remote_resume_codex, run_remote_saved_codex_session_exists, run_remote_scan,
    run_remote_stage_clipboard_png, run_remote_start_codex, run_remote_terminate_codex,
    run_remote_upsert_generated_copy,
};
use anyhow::{Result, bail};

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
    RefreshManagedCli {
        background: bool,
    },
    EnsureManagedCli {
        tool: ManagedCliTool,
    },
    Scan {
        codex_home: Option<String>,
    },
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
    UpsertGeneratedCopy {
        session_id: String,
    },
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
        "refresh-managed-cli" if args.len() >= 4 => RemoteServerCommand::RefreshManagedCli {
            background: args[3] == "background",
        },
        "ensure-managed-cli" if args.len() >= 4 => RemoteServerCommand::EnsureManagedCli {
            tool: parse_managed_cli_tool(&args[3])?,
        },
        "scan" => RemoteServerCommand::Scan {
            codex_home: args.get(3).cloned(),
        },
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
        "upsert-generated-copy" if args.len() == 4 => RemoteServerCommand::UpsertGeneratedCopy {
            session_id: args[3].clone(),
        },
        _ => return Ok(None),
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
        RemoteServerCommand::RefreshManagedCli { background } => {
            run_remote_refresh_managed_cli(background)
        }
        RemoteServerCommand::EnsureManagedCli { tool } => run_remote_ensure_managed_cli(tool),
        RemoteServerCommand::Scan { codex_home } => run_remote_scan(codex_home.as_deref()),
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
        RemoteServerCommand::UpsertGeneratedCopy { session_id } => {
            run_remote_upsert_generated_copy(&session_id)
        }
    }
}

fn parse_managed_cli_tool(value: &str) -> Result<ManagedCliTool> {
    match value {
        "codex" => Ok(ManagedCliTool::Codex),
        "codex-litellm" => Ok(ManagedCliTool::CodexLiteLlm),
        other => bail!("unknown managed cli tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resume_codex_supports_require_existing_and_cwd() {
        let args = vec![
            "server".to_string(),
            "remote".to_string(),
            "resume-codex".to_string(),
            "019ad8".to_string(),
            "/home/pi".to_string(),
            "--require-existing".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::ResumeCodex {
                session_id: "019ad8".to_string(),
                cwd: Some("/home/pi".to_string()),
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
            "/home/pi/.codex/sessions/example.jsonl".to_string(),
        ];
        let command = parse_remote_server_command(&args)
            .expect("parse command")
            .expect("remote command");
        assert_eq!(
            command,
            RemoteServerCommand::PreviewTail {
                session_id: "/home/pi/.codex/sessions/example.jsonl".to_string(),
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
