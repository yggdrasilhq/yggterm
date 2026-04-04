use crate::{
    ManagedCliTool, run_remote_ensure_managed_cli, run_remote_generation_context,
    run_remote_preview, run_remote_preview_head, run_remote_protocol_version,
    run_remote_refresh_managed_cli, run_remote_resume_codex, run_remote_scan,
    run_remote_stage_clipboard_png, run_remote_terminate_codex, run_remote_upsert_generated_copy,
};
use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteServerCommand {
    StageClipboardPng,
    ProtocolVersion,
    ResumeCodex {
        session_id: String,
        cwd: Option<String>,
        require_existing: bool,
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
    PreviewHead {
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
        "refresh-managed-cli" if args.len() >= 4 => RemoteServerCommand::RefreshManagedCli {
            background: args[3] == "background",
        },
        "ensure-managed-cli" if args.len() >= 4 => RemoteServerCommand::EnsureManagedCli {
            tool: parse_managed_cli_tool(&args[3])?,
        },
        "scan" => RemoteServerCommand::Scan {
            codex_home: args.get(3).cloned(),
        },
        "preview-head" if args.len() >= 4 => RemoteServerCommand::PreviewHead {
            session_id: args[3].clone(),
            blocks: args
                .get(4)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(8),
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
        RemoteServerCommand::ResumeCodex {
            session_id,
            cwd,
            require_existing,
        } => run_remote_resume_codex(&session_id, cwd.as_deref(), require_existing),
        RemoteServerCommand::RefreshManagedCli { background } => {
            run_remote_refresh_managed_cli(background)
        }
        RemoteServerCommand::EnsureManagedCli { tool } => run_remote_ensure_managed_cli(tool),
        RemoteServerCommand::Scan { codex_home } => run_remote_scan(codex_home.as_deref()),
        RemoteServerCommand::PreviewHead { session_id, blocks } => {
            run_remote_preview_head(&session_id, blocks)
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
    fn parse_non_remote_command_returns_none() {
        let args = vec!["server".to_string(), "status".to_string()];
        assert!(
            parse_remote_server_command(&args)
                .expect("parse command")
                .is_none()
        );
    }
}
