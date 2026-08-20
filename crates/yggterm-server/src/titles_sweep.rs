//! `server titles sweep` — resolve the titles `server titles ls` reports as bad.
//!
//! ⛔ THIS EXISTS BECAUSE THE SWEEP WAS A CHORE EVERY SESSION HAND-ASSEMBLED.
//! "Find the rows wearing a short hash, resolve them store-first, pace the
//! endpoint, and forget the entries whose sessions are gone" was a paragraph of
//! instructions, re-derived from primitives each time by an agent whose
//! discipline resets every session. A verb's does not.
//!
//! It reads the SAME durable scan `server titles ls` and the startpage read
//! (`scan_all_durable_sessions`) and the SAME recognizer the chore uses
//! (`looks_like_generated_fallback_title`), so "which titles are bad" cannot
//! have two answers — one to report and one to act on.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use yggterm_core::startpage::{scan_all_durable_sessions, StartpageDurableRow};
use yggterm_core::{
    best_effort_context_from_session_path, copy_generation_pause_remaining_ms,
    looks_like_generated_fallback_title, SessionStore, SessionTitleStore,
};

use crate::snapshot;

/// How a row's title reads today. The three states are what the owner asked to
/// see counted per host and per CLI, before and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum TitleState {
    /// A real title — a name a person could have written.
    Ok,
    /// A short hash, a placeholder, a raw path: the recognizer's business.
    Fallback,
    /// Nothing at all, which renders as the short id.
    Missing,
}

fn title_state(row: &StartpageDurableRow) -> TitleState {
    match row.effective_title.as_deref().map(str::trim) {
        None | Some("") => TitleState::Missing,
        Some(title) if looks_like_generated_fallback_title(title) => TitleState::Fallback,
        Some(_) => TitleState::Ok,
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
struct StateCounts {
    ok: usize,
    fallback: usize,
    missing: usize,
}

impl StateCounts {
    fn add(&mut self, state: TitleState) {
        match state {
            TitleState::Ok => self.ok += 1,
            TitleState::Fallback => self.fallback += 1,
            TitleState::Missing => self.missing += 1,
        }
    }

    fn bad(&self) -> usize {
        self.fallback + self.missing
    }
}

fn counts_by_kind(rows: &[StartpageDurableRow]) -> HashMap<String, StateCounts> {
    let mut out: HashMap<String, StateCounts> = HashMap::new();
    for row in rows {
        out.entry(kind_slug(row).to_string())
            .or_default()
            .add(title_state(row));
    }
    out
}

fn kind_slug(row: &StartpageDurableRow) -> &'static str {
    yggterm_core::agent_cli::agent_cli_descriptor(row.kind)
        .map(|descriptor| descriptor.slug)
        .unwrap_or("other")
}

#[derive(Debug, Clone, serde::Serialize)]
struct ResolvedRow {
    session_id: String,
    kind: &'static str,
    was: TitleState,
    title: Option<String>,
    source: &'static str,
    error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct SweepReport {
    host: String,
    home: String,
    model: String,
    dry_run: bool,
    durable_count: usize,
    /// Per CLI, before this sweep ran.
    before: HashMap<String, StateCounts>,
    /// Per CLI, after — a RE-SCAN, not the before-counts with arithmetic
    /// applied. A count derived from what we believe we did cannot report that
    /// a write did not land.
    after: HashMap<String, StateCounts>,
    attempted: usize,
    resolved: Vec<ResolvedRow>,
    /// Entries in the copy store whose session no longer exists anywhere.
    pruned: Vec<String>,
    prune_skipped_reason: Option<String>,
    /// Set when the endpoint refused mid-sweep: everything after that point was
    /// NOT attempted, and a report that did not say so would read as "these
    /// rows cannot be titled".
    stopped_on_endpoint_pause_ms: Option<u64>,
    elapsed_ms: u128,
}

pub fn run_server_titles_sweep(store: &SessionStore, args: &[String]) -> Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let prune = args.iter().any(|arg| arg == "--prune");
    let limit = flag_value(args, "--limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(25);
    let max_seconds = flag_value(args, "--max-seconds")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(900);
    let only_kind = flag_value(args, "--kind").map(str::to_string);

    let started_at = Instant::now();
    let home = store.home_dir().to_path_buf();
    let system_home = dirs::home_dir().unwrap_or_else(|| home.clone());
    let settings = store.load_settings().unwrap_or_default();

    let rows = scan_all_durable_sessions(&system_home);
    let before = counts_by_kind(&rows);

    let mut targets = rows
        .iter()
        .filter(|row| title_state(row) != TitleState::Ok)
        .filter(|row| {
            only_kind
                .as_deref()
                .is_none_or(|slug| kind_slug(row) == slug)
        })
        .cloned()
        .collect::<Vec<_>>();
    // Newest first: the rows a person is most likely to look at today are the
    // ones a bounded sweep should spend its budget on.
    targets.sort_by(|a, b| b.modified_epoch_ms.cmp(&a.modified_epoch_ms));

    let mut resolved = Vec::new();
    let mut attempted = 0usize;
    let mut stopped_on_endpoint_pause_ms = None;
    for row in targets.iter() {
        // ⛔ `--limit` bounds GENERATIONS, not rows looked at. It used to bound
        // the loop, and the first budget went entirely to four rows whose store
        // is a database the transcript reader cannot open — a sweep that spent
        // its whole allowance on rows it could not have titled, and reported
        // that as its work.
        if attempted >= limit {
            break;
        }
        if started_at.elapsed().as_secs() >= max_seconds {
            break;
        }
        if let Some(remaining_ms) = copy_generation_pause_remaining_ms() {
            // ⛔ Stop, do not skip: with the endpoint refusing, every further
            // row would report the same failure and the report would blame the
            // sessions for something the endpoint did.
            stopped_on_endpoint_pause_ms = Some(remaining_ms);
            break;
        }
        let was = title_state(row);
        if dry_run {
            // ⛔ A DRY RUN THAT ONLY COUNTS ROWS ANSWERS NOTHING. The question a
            // person asks before spending an endpoint budget is "how many of
            // these could actually be titled" — and the answer turned out to be
            // most of them cannot, because they are one-word sessions with no
            // content to name. So the plan is computed for real; only the
            // request is withheld.
            let (source, error) = match plan_one(row) {
                Ok(plan) => (plan.source(), None),
                Err(error) => ("error", Some(format!("{error:#}"))),
            };
            if source == "generated" {
                attempted += 1;
            }
            resolved.push(ResolvedRow {
                session_id: row.session_id.clone(),
                kind: kind_slug(row),
                was,
                title: None,
                source,
                error,
            });
            continue;
        }
        let outcome = resolve_one(store, &settings, row);
        match outcome {
            Ok((title, source)) => {
                if source == "generated" {
                    attempted += 1;
                }
                resolved.push(ResolvedRow {
                    session_id: row.session_id.clone(),
                    kind: kind_slug(row),
                    was,
                    title,
                    source,
                    error: None,
                })
            }
            Err(error) => {
                attempted += 1;
                let rendered = format!("{error:#}");
                let endpoint_refusal = yggterm_core::error_is_endpoint_refusal(&error);
                resolved.push(ResolvedRow {
                    session_id: row.session_id.clone(),
                    kind: kind_slug(row),
                    was,
                    title: None,
                    source: "error",
                    error: Some(rendered),
                });
                if endpoint_refusal {
                    stopped_on_endpoint_pause_ms =
                        Some(copy_generation_pause_remaining_ms().unwrap_or(0));
                    break;
                }
            }
        }
    }

    let (pruned, prune_skipped_reason) = if prune && !dry_run {
        prune_orphaned_copy(&home, &system_home, &rows)
    } else if prune {
        (Vec::new(), Some("dry run".to_string()))
    } else {
        (Vec::new(), None)
    };

    let after_rows = scan_all_durable_sessions(&system_home);
    let report = SweepReport {
        host: hostname(),
        home: system_home.display().to_string(),
        model: settings.interface_llm_model.clone(),
        dry_run,
        durable_count: rows.len(),
        before,
        after: counts_by_kind(&after_rows),
        attempted,
        resolved,
        pruned,
        prune_skipped_reason,
        stopped_on_endpoint_pause_ms,
        elapsed_ms: started_at.elapsed().as_millis(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report, targets.len());
    }
    Ok(())
}

/// What this row NEEDS, decided without spending anything.
///
/// Split out from the doing so a dry run can answer the only question worth
/// asking before a sweep: how many of these can be titled at all.
enum Plan {
    /// The CLI already wrote a usable title — nothing to generate.
    FromStore(String),
    /// The store file is gone; the row is a tombstone the scan still sees.
    NoTranscript,
    /// The transcript exists and says nothing worth naming — a one-word
    /// session, or boilerplate the context builder correctly strips.
    NoContext,
    /// Ready to generate, with the context to generate from.
    Generate { context: String, force: bool },
}

impl Plan {
    fn source(&self) -> &'static str {
        match self {
            Plan::FromStore(_) => "store",
            Plan::NoTranscript => "no-transcript",
            Plan::NoContext => "no-context",
            Plan::Generate { .. } => "generated",
        }
    }
}

/// Decide what a row needs. STORE FIRST, and that is not a nicety: a CLI that
/// writes its own title is the authority for it, and asking a model to invent a
/// second one is how a row ends up with two names that disagree forever.
fn plan_one(row: &StartpageDurableRow) -> Result<Plan> {
    if let Some(title) = row.title.as_deref().map(str::trim) {
        if !title.is_empty() && !looks_like_generated_fallback_title(title) {
            return Ok(Plan::FromStore(title.to_string()));
        }
    }
    let path = PathBuf::from(&row.storage_path);
    if !path.exists() {
        return Ok(Plan::NoTranscript);
    }
    let context = best_effort_context_from_session_path(&path)?;
    if context.trim().is_empty() {
        return Ok(Plan::NoContext);
    }
    Ok(Plan::Generate {
        context,
        // `force` for a row that HAS generated copy: it is a fallback, and
        // without force the resolver hands the cached fallback straight back.
        force: row.generated_title.is_some(),
    })
}

fn resolve_one(
    store: &SessionStore,
    settings: &yggterm_core::AppSettings,
    row: &StartpageDurableRow,
) -> Result<(Option<String>, &'static str)> {
    match plan_one(row)? {
        Plan::FromStore(title) => Ok((Some(title), "store")),
        Plan::NoTranscript => Ok((None, "no-transcript")),
        Plan::NoContext => Ok((None, "no-context")),
        Plan::Generate { context, force } => {
            let title = store.generate_title_for_context(
                settings,
                &row.session_id,
                &row.cwd,
                &context,
                force,
            )?;
            Ok((title, "generated"))
        }
    }
}

/// Forget copy for sessions that no longer exist anywhere.
///
/// ⛔ THE DANGEROUS DIRECTION IS DELETING SOMETHING LIVE, so this refuses on any
/// doubt: an empty scan, an unreachable daemon (its live rows are keyed by
/// runtime uuid and would otherwise all read as orphans) or copy younger than a
/// week is left alone. A negative from one store is not a negative from
/// persistence.
fn prune_orphaned_copy(
    home: &std::path::Path,
    system_home: &std::path::Path,
    rows: &[StartpageDurableRow],
) -> (Vec<String>, Option<String>) {
    if rows.is_empty() {
        return (
            Vec::new(),
            Some("the durable scan found nothing — refusing to call every entry an orphan".into()),
        );
    }
    let live = match snapshot(&crate::server_cli::cli_server_endpoint(home)) {
        Ok((snap, _)) => snap
            .live_sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<HashSet<_>>(),
        Err(error) => {
            return (
                Vec::new(),
                Some(format!(
                    "no daemon answered ({error}) — a live row's copy is keyed by its runtime id \
                     and would read as an orphan"
                )),
            );
        }
    };
    let Ok(title_store) = SessionTitleStore::open(system_home) else {
        return (Vec::new(), Some("could not open the copy store".into()));
    };
    let Ok(entries) = title_store.generated_copy_ages_in_days() else {
        return (Vec::new(), Some("could not read the copy store".into()));
    };
    let durable = rows
        .iter()
        .map(|row| row.session_id.clone())
        .collect::<HashSet<_>>();
    let mut pruned = Vec::new();
    for (session_id, age_days) in entries {
        if age_days < PRUNE_MIN_AGE_DAYS {
            continue;
        }
        if durable.contains(&session_id) || live.contains(&session_id) {
            continue;
        }
        if title_store.delete_generated_copy(&session_id).is_ok() {
            pruned.push(session_id);
        }
    }
    (pruned, None)
}

/// Copy younger than this is never pruned, however orphaned it looks.
const PRUNE_MIN_AGE_DAYS: i64 = 7;

fn print_human(report: &SweepReport, bad_total: usize) {
    println!(
        "titles sweep — host {}  model {}{}",
        report.host,
        if report.model.trim().is_empty() {
            "<unset>"
        } else {
            report.model.as_str()
        },
        if report.dry_run { "  (dry run)" } else { "" }
    );
    println!(
        "durable {}  bad before {}  attempted {}  elapsed {}ms",
        report.durable_count, bad_total, report.attempted, report.elapsed_ms
    );
    let mut kinds = report
        .before
        .keys()
        .chain(report.after.keys())
        .cloned()
        .collect::<Vec<_>>();
    kinds.sort();
    kinds.dedup();
    println!("{:<16} {:>18} {:>18}", "cli", "before ok/fb/miss", "after ok/fb/miss");
    for kind in kinds {
        let before = report.before.get(&kind).cloned().unwrap_or_default();
        let after = report.after.get(&kind).cloned().unwrap_or_default();
        println!(
            "{:<16} {:>18} {:>18}",
            kind,
            format!("{}/{}/{}", before.ok, before.fallback, before.missing),
            format!("{}/{}/{}", after.ok, after.fallback, after.missing),
        );
    }
    for row in &report.resolved {
        match (&row.title, &row.error) {
            (Some(title), _) => println!("  + {} [{}] {}", &row.session_id[..8.min(row.session_id.len())], row.source, title),
            (None, Some(error)) => println!("  ! {} {}", &row.session_id[..8.min(row.session_id.len())], error),
            (None, None) => println!("  · {} [{}] nothing to name it from", &row.session_id[..8.min(row.session_id.len())], row.source),
        }
    }
    if !report.pruned.is_empty() {
        println!("pruned {} orphaned copy entries", report.pruned.len());
    }
    if let Some(reason) = &report.prune_skipped_reason {
        println!("prune skipped: {reason}");
    }
    if let Some(remaining) = report.stopped_on_endpoint_pause_ms {
        println!(
            "⛔ stopped: the interface LLM endpoint is refusing; {}s until the next probe. \
             The rows after this point were NOT attempted.",
            remaining / 1000
        );
    }
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yggterm_core::SessionKind;

    fn row(title: Option<&str>, generated: Option<&str>) -> StartpageDurableRow {
        let effective = title.or(generated);
        StartpageDurableRow {
            session_id: "00000000-0000-4000-8000-0000000000c1".to_string(),
            cwd: "/home/user/project".to_string(),
            title: title.map(str::to_string),
            generated_title: generated.map(str::to_string),
            effective_title: effective.map(str::to_string),
            detail: None,
            kind: SessionKind::Codex,
            modified_epoch_ms: 0,
            storage_path: "/home/user/.codex/sessions/rollout.jsonl".to_string(),
            display_path: "rollout.jsonl".to_string(),
        }
    }

    /// The sweep and the chore must not disagree about which titles are bad —
    /// so this asks the SAME recognizer, and the test pins that a short hash is
    /// not quietly counted as a title just because the column is non-empty.
    #[test]
    fn a_short_hash_is_not_a_title_and_an_absent_one_is_its_own_state() {
        assert_eq!(title_state(&row(Some("Daemon Leak Audit"), None)), TitleState::Ok);
        assert_eq!(title_state(&row(Some("43936dd"), None)), TitleState::Fallback);
        assert_eq!(
            title_state(&row(None, Some("untitled session"))),
            TitleState::Fallback
        );
        assert_eq!(title_state(&row(None, None)), TitleState::Missing);
        assert_eq!(title_state(&row(Some("   "), None)), TitleState::Missing);
    }

    /// ⚖ Bad is bad — but the two states are reported separately because they
    /// have different causes: a fallback means generation ran and produced
    /// junk, a missing one means it never ran at all.
    #[test]
    fn the_counts_keep_the_two_kinds_of_bad_apart() {
        let rows = vec![
            row(Some("Daemon Leak Audit"), None),
            row(Some("43936dd"), None),
            row(None, None),
        ];
        let counts = counts_by_kind(&rows);
        let codex = counts.get("codex").expect("codex rows are counted under codex");
        assert_eq!((codex.ok, codex.fallback, codex.missing), (1, 1, 1));
        assert_eq!(codex.bad(), 2);
    }
}
