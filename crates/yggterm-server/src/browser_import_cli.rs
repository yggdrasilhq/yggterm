//! THE `web-import …` verb plane — one owner, both binaries.
//!
//! A thin shell over [`yggterm_core::browser_import`]: it finds browsers,
//! lists their profiles, and runs an import. Every decision — the epoch, the
//! copy-then-open, the dedupe — belongs to the library, so `ychrome`, the GUI,
//! and a future `collection import` verb can all reach the same behaviour
//! without going through this parser.
//!
//! ⚠ A GAP, stated rather than hidden. The spec (`ychrome/docs/collections.md`
//! §Verbs) writes the import as `collection import --from <chromium|firefox>
//! --path <dir>`, and I3's `collection` plane landed without that arm. So the
//! import answers to `web-import run` and NOT to `collection import` today.
//! Closing it is one delegation in `web_collection_cli.rs` —
//! `"import" => …run_browser_import_cli(…)` — deliberately left to that file's
//! owner rather than reached into from here while their lane is in flight.
//! `browsers` and `profiles` stay here either way: enumerating what is
//! installed on this machine is not a collection concept.
//!
//! No daemon, no GUI, no app-control handshake: an import is local file work
//! and must not need a running desktop.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde_json::json;

use yggterm_core::browser_import::{
    BROWSER_SOURCES, BrowserProfile, DiscoveredBrowser, ImportRequest, browser_source,
    discover_browsers, discover_profiles, import_browser_profile,
};
use yggterm_core::cli_args::{cli_flag_value, cli_positional_args};
use yggterm_core::web_profile::WEB_PROFILE_DEFAULT;

pub fn browser_import_usage_block(binary: &str) -> String {
    format!(
        "browser import (history and bookmarks out of other browsers — see ychrome/docs/collections.md):
  {binary} web-import browsers [--json]
  {binary} web-import profiles --browser <id> [--path <user-data-dir>] [--json]
  {binary} web-import run --browser <id> [--source-profile <dir-name>]
      [--path <user-data-dir>] [--profile <yggterm-profile>]
      [--no-history] [--no-bookmarks] [--collection-id <id>] [--dry-run] [--json]

  Browsers: {browsers}
  History imports as VISITS into the profile's history.jsonl; bookmarks import
  as ONE collection with their folder tree. Both halves are idempotent — run it
  again to pick up what the browser has done since. --dry-run reads everything
  and writes nothing.",
        browsers = BROWSER_SOURCES
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The verbs this plane answers to, for the usage-drift lock.
pub const BROWSER_IMPORT_ACTIONS: [&str; 3] = ["browsers", "profiles", "run"];

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("no home directory to look for browsers in"))
}

fn profiles_root() -> Result<PathBuf> {
    Ok(yggterm_core::resolve_yggterm_home()?.join("web-profiles"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// The local UTC offset, so an imported collection's `created_at` reads in the
/// timezone the user lives in — through the one reader of this machine's
/// offset, shared with the automation and collection planes.
fn utc_offset_secs(now: u64) -> i32 {
    crate::automation_cli::local_utc_offset_secs(now).unwrap_or(0)
}

fn wants_json(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--json")
}

/// Resolve `--browser` plus an optional `--path` into a user-data directory.
/// An explicit `--path` wins, so a profile copied off another machine (or a
/// backup) can be imported without pretending to be installed here.
fn resolve_user_data_dir(args: &[String], browser_id: &str) -> Result<PathBuf> {
    let source = browser_source(browser_id)
        .ok_or_else(|| anyhow!("unknown browser {browser_id:?}; try `web-import browsers`"))?;
    if let Some(path) = cli_flag_value(args, "--path") {
        return Ok(PathBuf::from(path));
    }
    let home = home_dir()?;
    source.installed_user_data_dir(&home).ok_or_else(|| {
        anyhow!(
            "{} is not installed under {} — pass --path <user-data-dir> if it lives elsewhere",
            source.display_name,
            home.display()
        )
    })
}

fn print_browsers(found: &[DiscoveredBrowser], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "browser_count": found.len(),
                "browsers": found,
            }))
            .unwrap_or_else(|_| "{}".to_string())
        );
        return;
    }
    if found.is_empty() {
        println!("no importable browsers found");
        return;
    }
    for browser in found {
        println!(
            "{:<10} {:<16} {} ({} profile{})",
            browser.browser_id,
            browser.display_name,
            browser.user_data_dir.display(),
            browser.profiles.len(),
            if browser.profiles.len() == 1 { "" } else { "s" }
        );
    }
}

fn print_profiles(profiles: &[BrowserProfile], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile_count": profiles.len(),
                "profiles": profiles,
            }))
            .unwrap_or_else(|_| "{}".to_string())
        );
        return;
    }
    if profiles.is_empty() {
        println!("no profiles found");
        return;
    }
    for profile in profiles {
        println!(
            "{}{:<28} {:<24} history={} bookmarks={}",
            if profile.is_default { "* " } else { "  " },
            profile.dir_name,
            profile.display_name,
            profile.history_db().is_some(),
            profile.bookmarks_source().is_some(),
        );
    }
}

/// `web-import …`. Accepts the plane with or without a leading `server`, like
/// the automation plane does, so an agent typing either shape reaches it.
pub fn run_browser_import_cli(args: &[String]) -> Result<()> {
    // args[0] is `web-import`; the action is the next positional, so a flag
    // that takes a value can never be mistaken for one.
    let positional = cli_positional_args(args, 1);
    let action = positional.first().copied().unwrap_or_default().to_string();
    let json = wants_json(args);

    match action.as_str() {
        "browsers" => {
            let found = discover_browsers(&home_dir()?);
            print_browsers(&found, json);
            Ok(())
        }
        "profiles" => {
            let browser_id = cli_flag_value(args, "--browser")
                .context("missing --browser for web-import profiles")?;
            let source = browser_source(browser_id)
                .ok_or_else(|| anyhow!("unknown browser {browser_id:?}"))?;
            let user_data_dir = resolve_user_data_dir(args, browser_id)?;
            let profiles = discover_profiles(source, &user_data_dir);
            print_profiles(&profiles, json);
            Ok(())
        }
        "run" => {
            let browser_id = cli_flag_value(args, "--browser")
                .context("missing --browser for web-import run")?;
            let source = browser_source(browser_id)
                .ok_or_else(|| anyhow!("unknown browser {browser_id:?}"))?;
            let user_data_dir = resolve_user_data_dir(args, browser_id)?;
            let profiles = discover_profiles(source, &user_data_dir);
            if profiles.is_empty() {
                return Err(anyhow!(
                    "no profiles under {} — is that the user-data directory?",
                    user_data_dir.display()
                ));
            }
            // A named profile, or the one the browser itself opens by default.
            // Never "the first one we happened to read" — that would import a
            // different profile depending on directory order.
            let chosen = match cli_flag_value(args, "--source-profile") {
                Some(name) => profiles
                    .iter()
                    .find(|profile| profile.dir_name == name)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "{browser_id} has no profile {name:?}; try `web-import profiles --browser {browser_id}`"
                        )
                    })?,
                None => profiles
                    .iter()
                    .find(|profile| profile.is_default)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "{browser_id} names no default profile — pass --source-profile"
                        )
                    })?,
            };

            let now = now_ms();
            let request = ImportRequest {
                source: chosen,
                profiles_root: profiles_root()?,
                target_profile: cli_flag_value(args, "--profile")
                    .unwrap_or(WEB_PROFILE_DEFAULT)
                    .to_string(),
                history: !args.iter().any(|arg| arg == "--no-history"),
                bookmarks: !args.iter().any(|arg| arg == "--no-bookmarks"),
                collection_id: cli_flag_value(args, "--collection-id").map(str::to_string),
                now_ms: now,
                utc_offset_secs: utc_offset_secs(now),
                dry_run: args.iter().any(|arg| arg == "--dry-run"),
            };
            let report = import_browser_profile(&request)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{} / {} -> profile {}{}",
                    report.browser,
                    report.source_profile,
                    report.target_profile,
                    if report.dry_run { " (dry run)" } else { "" }
                );
                println!(
                    "  history:   {} read, {} written, {} already there{}",
                    report.history.visits_offered,
                    report.history.visits_written,
                    report.history.duplicates,
                    match (
                        &report.history.oldest_utc_day,
                        &report.history.newest_utc_day
                    ) {
                        (Some(oldest), Some(newest)) => format!("  [{oldest} .. {newest}]"),
                        _ => String::new(),
                    }
                );
                println!(
                    "  bookmarks: {} read, {} added, {} already there, {} folders",
                    report.bookmarks.read,
                    report.bookmarks.added,
                    report.bookmarks.duplicates,
                    report.bookmarks.folders
                );
                if let Some(path) = &report.bookmarks.collection {
                    println!("  collection: {path}");
                }
            }
            Ok(())
        }
        other => Err(anyhow!(
            "unknown web-import action {other:?}\n\n{}",
            browser_import_usage_block("yggterm-headless")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ THE USAGE-DRIFT LOCK, the same one the `web` plane carries: an action
    /// this dispatcher answers and the usage block never names reads as "this
    /// build does not have it" to an agent following `--help`.
    #[test]
    fn every_action_appears_in_the_usage_block() {
        let usage = browser_import_usage_block("yggterm-headless");
        for action in BROWSER_IMPORT_ACTIONS {
            assert!(
                usage.contains(&format!("web-import {action}")),
                "`{action}` is dispatched and named nowhere in the usage block"
            );
        }
        let source = include_str!("browser_import_cli.rs");
        for action in BROWSER_IMPORT_ACTIONS {
            assert!(
                source.contains(&format!("\"{action}\" => ")),
                "`{action}` is in the usage block and dispatched nowhere"
            );
        }
    }

    /// Every browser in the table is offerable by name in the help text — the
    /// list is DERIVED, so adding a row to the library adds it here.
    #[test]
    fn the_usage_block_names_every_browser_the_library_knows() {
        let usage = browser_import_usage_block("yggterm");
        for source in BROWSER_SOURCES {
            assert!(usage.contains(source.id), "{} is unnamed", source.id);
        }
    }
}
