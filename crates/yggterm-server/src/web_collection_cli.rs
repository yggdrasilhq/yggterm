//! THE `collection …` / `snapshot now` verb plane, owned once for BOTH
//! binaries.
//!
//! Same rule as `automation_cli` and `app_control_web_cli`: a flag must mean one
//! thing whether it was typed at `yggterm` or `yggterm-headless`, so neither
//! binary carries a copy of this parser. Wired in both `apps/yggterm/src/main.rs`
//! and `apps/yggterm/src/bin/yggterm-headless.rs`, exactly the way `automation`
//! is.
//!
//! Spec of record: `ychrome/docs/collections.md` — increment I3. The verbs are
//! written there as `ychrome collection …`; the §Correction at the end of that
//! document is why they live here instead (ychrome does not depend on
//! `yggterm-core`, and the store sits under `~/.yggterm/web-profiles/<profile>/`
//! next to the `history.jsonl` these are built from).
//!
//! # What this file is NOT allowed to contain
//!
//! - A second collection parser. [`yggterm_core::web_collection`] is the one.
//! - A second store. [`yggterm_core::web_collection_store`] is the one, and it
//!   is where every `now_ms`-taking decision lives — this file supplies the
//!   clock and the argv, and nothing else.
//! - A second history reader. [`yggterm_core::web_history`] is the one, and it
//!   is the same reader the GUI's history viewer uses.

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use serde_json::{Value, json};

use yggterm_core::cli_flag_value;
use yggterm_core::web_collection::Collection;
use yggterm_core::web_collection_store::{
    CollectionKind, CollectionStore, CollectionSummary, DEFAULT_SNAPSHOT_MAX_AGE_DAYS,
    DEFAULT_SNAPSHOT_MAX_COUNT, NewCollection, PrunePolicy, add_link, add_tag, build_collection,
    items_in_folder, note_of, plan_snapshot_prune, promote, should_write_snapshot, summarize,
    tags_of, touch,
};
use yggterm_core::web_history::{WebHistoryEntry, web_history_entries};
use yggterm_core::web_profile::normalize_web_profile;

/// How deep into `history.jsonl` `add-from-history` reads before filtering. The
/// same cap the GUI's history page uses; a `--since` older than this many
/// distinct URLs simply sees what the viewer sees.
const HISTORY_SCAN_LIMIT: usize = 5000;

pub fn web_collection_usage_block(binary: &str) -> String {
    format!(
        "collections (history organised into things worth keeping — see ychrome/docs/collections.md):
  {binary} collection list [--profile <p>] [--json]
  {binary} collection show <id> [--profile <p>] [--json]
  {binary} collection new <name> [--tag <t>]... [--note <text>|--note-stdin] [--profile <p>]
  {binary} collection add <id> --url <u> [--title <t>] [--folder <path>] [--profile <p>]
  {binary} collection add-from-history <id> --since <when> [--match <substr>] [--limit <n>]
      --since is `30m`, `6h`, `7d`, `2w`, `all`, or an RFC-3339 instant
  {binary} collection move <id> --item <url> --to-folder <path>
  {binary} collection rename <id> <name>
  {binary} collection tag <id> <t>
  {binary} collection note <id> (--text <t>|--stdin)
  {binary} collection promote <id> --name <name>          # snapshot -> collection
  {binary} collection open <id> [--folder <path>] [--json]
  {binary} collection export <id> [--as md|json] [--out <file>]
  {binary} collection prune [--profile <p>] [--dry-run]
  {binary} collection import --browser <id> [--source-profile <p>] [--dry-run]
    an alias for `web-import run`, which is the implementation — one owner
  {binary} snapshot now [--profile <p>] (--url <u> [--title <t>])... [--stdin] [--name <n>]

  Every verb defaults to --profile default. A collection IS a Markdown file at
  ~/.yggterm/web-profiles/<profile>/collections/<id>.md — `export` with no --out
  prints it, because the file already is the export format.
  `prune` only ever touches snapshots: a collection is never pruned, and the
  store's only delete re-reads the file's own kind before unlinking.
  `snapshot now` takes its items explicitly; the close hook and the cadence
  chore that will feed it the open tabs are increment I4."
    )
}

/// The name the user actually typed, so a copied usage line names the binary
/// they are holding rather than the one this file happened to be written for.
fn binary_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().to_string()))
            .unwrap_or_else(|| "yggterm-headless".to_string())
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// The local UTC offset, so a collection's `created_at` reads in the timezone
/// the user lives in rather than in UTC. Shared with the automation plane —
/// there is one reader of this machine's offset, not two.
fn utc_offset_secs(now: u64) -> i32 {
    crate::automation_cli::local_utc_offset_secs(now).unwrap_or(0)
}

fn print(value: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn read_stdin() -> anyhow::Result<String> {
    let mut value = String::new();
    std::io::stdin()
        .read_to_string(&mut value)
        .context("reading from stdin")?;
    Ok(value)
}

/// Every `--<flag> <value>` occurrence, in order. `cli_flag_value` answers with
/// the first; `--tag` and `--url` are repeatable and need all of them.
fn repeated_flag_values<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index] == flag {
            out.push(args[index + 1].as_str());
            index += 2;
            continue;
        }
        index += 1;
    }
    out
}

fn profile_of(args: &[String]) -> String {
    normalize_web_profile(cli_flag_value(args, "--profile"))
}

fn store_for(args: &[String]) -> anyhow::Result<(String, CollectionStore)> {
    let profile = profile_of(args);
    let store = CollectionStore::for_profile(&profile).ok_or_else(|| {
        anyhow!(
            "the {profile:?} profile keeps nothing on disk, so it has no collections — \
             that is what makes it the private-browsing profile"
        )
    })?;
    Ok((profile, store))
}

fn load(store: &CollectionStore, id: &str) -> anyhow::Result<Collection> {
    store.load(id).with_context(|| {
        format!(
            "no collection {id:?} in {} — `collection list` shows what is there",
            store.dir().display()
        )
    })
}

fn positional<'a>(args: &'a [String], index: usize) -> Option<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.starts_with("--"))
}

// ---------------------------------------------------------------------------
// `--since`
// ---------------------------------------------------------------------------

/// Parse `--since` into an instant. Pure: `now_ms` is an argument, so `7d`
/// means the same thing in a test as it does at 3 a.m.
///
/// Accepted: `all` (everything), a duration (`45m`, `6h`, `7d`, `2w`), or an
/// RFC-3339 instant (`2026-07-01T00:00:00+05:30`). Anything else is an ERROR
/// rather than a guess — a `--since` we misread silently adds the wrong pages.
pub fn parse_since(text: &str, now_ms: u64) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("--since needs a value: `7d`, `6h`, `all`, or an RFC-3339 instant".to_string());
    }
    if text.eq_ignore_ascii_case("all") {
        return Ok(0);
    }
    if let Some(instant) = yggterm_core::web_collection_store::parse_timestamp(text) {
        return Ok(instant);
    }
    let (digits, unit) = text.split_at(text.len() - 1);
    let scale_ms = match unit {
        "m" => 60_000u64,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => {
            return Err(format!(
                "cannot read --since {text:?}. Use a duration (`45m`, `6h`, `7d`, `2w`), `all`, \
                 or an RFC-3339 instant like `2026-07-01T00:00:00+05:30`"
            ));
        }
    };
    let count: u64 = digits
        .parse()
        .map_err(|_| format!("cannot read the number in --since {text:?}"))?;
    Ok(now_ms.saturating_sub(count.saturating_mul(scale_ms)))
}

/// Which history entries an `add-from-history` should take, given the entries
/// the ONE reader returned.
///
/// Pure, and separated from the read so the window, the substring filter and
/// the cap can be pinned against fixed entries. Order is the reader's:
/// newest-first, which is the order they land in the file.
pub fn history_selection<'a>(
    entries: &'a [WebHistoryEntry],
    since_ms: u64,
    needle: Option<&str>,
    limit: usize,
) -> Vec<&'a WebHistoryEntry> {
    let needle = needle.map(str::to_lowercase);
    entries
        .iter()
        .filter(|entry| entry.ts_ms >= since_ms)
        .filter(|entry| match &needle {
            None => true,
            Some(needle) => {
                entry.url.to_lowercase().contains(needle)
                    || entry.title.to_lowercase().contains(needle)
            }
        })
        .take(limit)
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn summary_to_json(summary: &CollectionSummary) -> Value {
    json!({
        "id": summary.id,
        "name": summary.name,
        "kind": summary.kind.as_str(),
        "created_at_ms": summary.created_at_ms,
        "updated_at_ms": summary.updated_at_ms,
        "tags": summary.tags,
        "items": summary.item_count,
        "folders": summary.folder_count,
    })
}

fn print_summary_table(entries: &[CollectionSummary]) {
    for entry in entries {
        println!(
            "{:<28} {:<10} {:>4} items  {}",
            entry.id,
            entry.kind.as_str(),
            entry.item_count,
            entry.name.as_deref().unwrap_or("")
        );
    }
}

/// Save and report in one shape, so every mutating verb answers the same way.
fn save_and_report(
    store: &CollectionStore,
    id: &str,
    collection: &Collection,
    extra: Value,
) -> anyhow::Result<()> {
    store
        .save(id, collection)
        .with_context(|| format!("writing {}", store.dir().join(format!("{id}.md")).display()))?;
    let mut value = summary_to_json(&summarize(id, collection));
    if let (Some(object), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in extra {
            object.insert(key.clone(), item.clone());
        }
    }
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "path".to_string(),
            json!(store.dir().join(format!("{id}.md")).display().to_string()),
        );
    }
    print(&value)
}

// ---------------------------------------------------------------------------
// The CLI
// ---------------------------------------------------------------------------

/// `args` is the full argv tail, e.g. `["collection", "list", "--json"]` or
/// `["snapshot", "now"]`.
pub fn run_web_collection_cli(args: &[String]) -> anyhow::Result<()> {
    let head = args
        .first()
        .map(String::as_str)
        .context("missing verb — try `collection list`")?;
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h" | "help"))
    {
        println!("{}", web_collection_usage_block(binary_name()));
        return Ok(());
    }
    if head == "snapshot" {
        return run_snapshot(args);
    }
    let action = args.get(1).map(String::as_str).ok_or_else(|| {
        anyhow!(
            "missing collection action\n\n{}",
            web_collection_usage_block("yggterm-headless")
        )
    })?;
    // `collection snapshot now` is accepted too, so the verb is discoverable
    // under the noun it belongs to.
    if action == "snapshot" {
        return run_snapshot(&args[1..]);
    }
    let json_out = args.iter().any(|arg| arg == "--json");
    let now = now_ms();
    let offset = utc_offset_secs(now);
    let (profile, store) = store_for(args)?;

    match action {
        "list" => {
            let entries = store.list();
            if json_out {
                print(&json!({
                    "profile": profile,
                    "dir": store.dir().display().to_string(),
                    "collections": entries.iter().map(summary_to_json).collect::<Vec<_>>(),
                }))
            } else if entries.is_empty() {
                println!(
                    "no collections in {}. `collection new <name>` to make one.",
                    store.dir().display()
                );
                Ok(())
            } else {
                print_summary_table(&entries);
                Ok(())
            }
        }
        "show" => {
            let id = positional(args, 2).context("missing collection id")?;
            let collection = load(&store, id)?;
            let mut value = summary_to_json(&summarize(id, &collection));
            if let Some(object) = value.as_object_mut() {
                object.insert("note".to_string(), json!(note_of(&collection)));
                object.insert(
                    "folders_named".to_string(),
                    json!(
                        collection
                            .folders()
                            .iter()
                            .map(|(depth, name)| json!({ "depth": depth, "name": name }))
                            .collect::<Vec<_>>()
                    ),
                );
                object.insert(
                    "links".to_string(),
                    json!(
                        collection
                            .items()
                            .map(|item| json!({ "title": item.title, "url": item.url }))
                            .collect::<Vec<_>>()
                    ),
                );
                object.insert(
                    "path".to_string(),
                    json!(store.dir().join(format!("{id}.md")).display().to_string()),
                );
            }
            if json_out {
                return print(&value);
            }
            println!(
                "{id}  [{}]  {} items",
                summarize(id, &collection).kind.as_str(),
                collection.item_count()
            );
            if let Some(name) = collection.name() {
                println!("name: {name}");
            }
            let tags = tags_of(&collection);
            if !tags.is_empty() {
                println!("tags: {}", tags.join(", "));
            }
            let note = note_of(&collection);
            if !note.is_empty() {
                println!("\n{note}\n");
            }
            for item in collection.items() {
                println!("  {}  {}", item.title, item.url);
            }
            Ok(())
        }
        "new" => {
            let name = positional(args, 2).context(
                "missing collection name — `collection new \"Quant reading\"`",
            )?;
            let note = if args.iter().any(|arg| arg == "--note-stdin") {
                Some(read_stdin()?)
            } else {
                cli_flag_value(args, "--note").map(str::to_string)
            };
            let tags: Vec<String> = repeated_flag_values(args, "--tag")
                .into_iter()
                .map(str::to_string)
                .collect();
            let id = store.allocate(name);
            let collection = build_collection(
                &NewCollection {
                    id: &id,
                    name: Some(name),
                    profile: &profile,
                    tags: &tags,
                    note: note.as_deref(),
                },
                CollectionKind::Collection,
                now,
                offset,
            );
            save_and_report(&store, &id, &collection, json!({ "created": true }))
        }
        "add" => {
            let id = positional(args, 2).context("missing collection id")?;
            let url = cli_flag_value(args, "--url").context("--url is the link to add")?;
            let title = cli_flag_value(args, "--title").unwrap_or(url);
            let folder = cli_flag_value(args, "--folder");
            let mut collection = load(&store, id)?;
            let added = add_link(&mut collection, folder, title, url);
            if added {
                touch(&mut collection, now, offset);
            }
            save_and_report(
                &store,
                id,
                &collection,
                json!({ "added": added, "url": url, "folder": folder }),
            )
        }
        "add-from-history" => {
            let id = positional(args, 2).context("missing collection id")?;
            let since = parse_since(
                cli_flag_value(args, "--since").unwrap_or("all"),
                now,
            )
            .map_err(|message| anyhow!(message))?;
            let limit = cli_flag_value(args, "--limit")
                .map(|raw| {
                    raw.parse::<usize>()
                        .with_context(|| format!("--limit expects a whole number, got {raw:?}"))
                })
                .transpose()?
                .unwrap_or(usize::MAX);
            let folder = cli_flag_value(args, "--folder");
            let needle = cli_flag_value(args, "--match");
            // ONE reader — the same file, the same order, the same dedupe the
            // omnibox and the history page see.
            let entries = web_history_entries(&profile, HISTORY_SCAN_LIMIT);
            let selected = history_selection(&entries, since, needle, limit);
            let mut collection = load(&store, id)?;
            let mut added = Vec::new();
            let mut skipped = 0usize;
            // Oldest-first into the file, so the collection reads in the order
            // the pages were visited rather than backwards.
            for entry in selected.iter().rev() {
                if add_link(&mut collection, folder, &entry.title, &entry.url) {
                    added.push(entry.url.clone());
                } else {
                    skipped += 1;
                }
            }
            if !added.is_empty() {
                touch(&mut collection, now, offset);
            }
            save_and_report(
                &store,
                id,
                &collection,
                json!({
                    "scanned": entries.len(),
                    "matched": selected.len(),
                    "added": added,
                    "already_present": skipped,
                    "since_ms": since,
                }),
            )
        }
        "move" => {
            let id = positional(args, 2).context("missing collection id")?;
            let url = cli_flag_value(args, "--item").context("--item is the URL to move")?;
            let folder = cli_flag_value(args, "--to-folder");
            let mut collection = load(&store, id)?;
            let moved = collection.move_item(url, folder);
            if !moved {
                return Err(anyhow!("{id} holds no item with url {url:?}"));
            }
            touch(&mut collection, now, offset);
            save_and_report(
                &store,
                id,
                &collection,
                json!({ "moved": url, "to_folder": folder }),
            )
        }
        "rename" => {
            let id = positional(args, 2).context("missing collection id")?;
            let name = positional(args, 3).context("missing new name")?;
            let mut collection = load(&store, id)?;
            collection.set_field("name", name);
            touch(&mut collection, now, offset);
            // The id is the FILENAME and does not move: renaming a collection
            // must not break a link somebody wrote down.
            save_and_report(&store, id, &collection, json!({ "renamed_to": name }))
        }
        "tag" => {
            let id = positional(args, 2).context("missing collection id")?;
            let tag = positional(args, 3)
                .or_else(|| cli_flag_value(args, "--tag"))
                .context("missing tag")?;
            let mut collection = load(&store, id)?;
            let added = add_tag(&mut collection, tag);
            if added {
                touch(&mut collection, now, offset);
            }
            save_and_report(&store, id, &collection, json!({ "tag": tag, "added": added }))
        }
        "note" => {
            let id = positional(args, 2).context("missing collection id")?;
            let note = if args.iter().any(|arg| arg == "--stdin") {
                read_stdin()?
            } else {
                cli_flag_value(args, "--text")
                    .context("`collection note <id> --text <t>` or --stdin")?
                    .to_string()
            };
            let mut collection = load(&store, id)?;
            yggterm_core::web_collection_store::set_note(&mut collection, &note);
            touch(&mut collection, now, offset);
            save_and_report(&store, id, &collection, json!({ "note_chars": note.len() }))
        }
        "promote" => {
            let id = positional(args, 2).context("missing snapshot id")?;
            let name = cli_flag_value(args, "--name")
                .context("--name is what turns a snapshot into a collection")?;
            let mut collection = load(&store, id)?;
            promote(&mut collection, name, now, offset).map_err(|message| anyhow!(message))?;
            save_and_report(&store, id, &collection, json!({ "promoted": true }))
        }
        "open" => {
            let id = positional(args, 2).context("missing collection id")?;
            let folder = cli_flag_value(args, "--folder");
            let collection = load(&store, id)?;
            let targets = items_in_folder(&collection, folder);
            if json_out {
                return print(&json!({
                    "id": id,
                    "folder": folder,
                    "targets": targets
                        .iter()
                        .map(|(title, url)| json!({ "title": title, "url": url }))
                        .collect::<Vec<_>>(),
                    // Stated rather than faked. There is no app-control command
                    // that opens a URL into the tab-placement owner today (the
                    // owner is GUI-side), and inventing a second placement path
                    // here is exactly what the spec forbids. `open` therefore
                    // RESOLVES; the rail in I6 calls this same resolver.
                    "placement": "unwired_until_i6",
                }));
            }
            for (_, url) in &targets {
                println!("{url}");
            }
            Ok(())
        }
        "export" => {
            let id = positional(args, 2).context("missing collection id")?;
            let collection = load(&store, id)?;
            let as_json = cli_flag_value(args, "--as").is_some_and(|value| value == "json");
            let body = if as_json {
                let mut value = summary_to_json(&summarize(id, &collection));
                if let Some(object) = value.as_object_mut() {
                    object.insert("note".to_string(), json!(note_of(&collection)));
                    object.insert(
                        "links".to_string(),
                        json!(
                            collection
                                .items()
                                .map(|item| json!({ "title": item.title, "url": item.url }))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                format!("{}\n", serde_json::to_string_pretty(&value)?)
            } else {
                // The FILE is the export format. Re-serialising it would be a
                // second source of truth for what a collection says.
                collection.to_markdown()
            };
            match cli_flag_value(args, "--out") {
                Some(out) => {
                    let path = PathBuf::from(out);
                    std::fs::write(&path, &body)
                        .with_context(|| format!("writing {}", path.display()))?;
                    print(&json!({
                        "id": id,
                        "out": path.display().to_string(),
                        "bytes": body.len(),
                        "as": if as_json { "json" } else { "md" },
                    }))
                }
                None => {
                    print!("{body}");
                    Ok(())
                }
            }
        }
        // `collection import` DELEGATES to the browser-import plane rather than
        // re-implementing it. The import lane landed its verb as
        // `web-import run` because this dispatcher had already merged without
        // an import arm; the two must not become two ways to do one thing, so
        // this arm forwards and the other stays the implementation.
        "import" => {
            let mut forwarded: Vec<String> = vec!["web-import".to_string(), "run".to_string()];
            forwarded.extend(args.iter().skip(2).cloned());
            return crate::run_browser_import_cli(&forwarded);
        }
        "prune" => {
            let dry_run = args.iter().any(|arg| arg == "--dry-run");
            let policy = PrunePolicy {
                max_age_days: cli_flag_value(args, "--max-age-days")
                    .map(|raw| raw.parse::<u64>().context("--max-age-days expects a number"))
                    .transpose()?
                    .unwrap_or(DEFAULT_SNAPSHOT_MAX_AGE_DAYS),
                max_count: cli_flag_value(args, "--max-count")
                    .map(|raw| raw.parse::<usize>().context("--max-count expects a number"))
                    .transpose()?
                    .unwrap_or(DEFAULT_SNAPSHOT_MAX_COUNT),
            };
            let plan = plan_snapshot_prune(&store.list(), now, policy);
            let mut removed = Vec::new();
            let mut refused = Vec::new();
            if !dry_run {
                for (id, _) in &plan.prune {
                    // The store's guard re-reads the file's own kind. A plan
                    // that went stale between here and now cannot delete a
                    // collection.
                    match store.remove_snapshot(id) {
                        Ok(()) => removed.push(id.clone()),
                        Err(message) => refused.push(json!({ "id": id, "reason": message })),
                    }
                }
            }
            print(&json!({
                "profile": profile,
                "dir": store.dir().display().to_string(),
                "policy": {
                    "max_age_days": policy.max_age_days,
                    "max_count": policy.max_count,
                },
                "dry_run": dry_run,
                "planned": plan
                    .prune
                    .iter()
                    .map(|(id, reason)| json!({ "id": id, "reason": reason.as_str() }))
                    .collect::<Vec<_>>(),
                "removed": removed,
                "refused": refused,
                "kept_snapshots": plan.keep,
                "protected_collections": plan.protected,
            }))
        }
        other => Err(anyhow!(
            "unknown collection action {other:?}\n\n{}",
            web_collection_usage_block("yggterm-headless")
        )),
    }
}

/// `snapshot now` — write a snapshot IF it says something the last one did not.
///
/// The items are explicit (`--url`/`--title`, or `--stdin`). The close hook and
/// the cadence chore that will feed it the profile's open tabs are increment
/// I4; the GUI owns the tab set, and reading `tabs.json` from here would be the
/// second reader this feature is not allowed to grow.
fn run_snapshot(args: &[String]) -> anyhow::Result<()> {
    let action = args.get(1).map(String::as_str).unwrap_or("now");
    if action != "now" {
        return Err(anyhow!(
            "unknown snapshot action {action:?} — only `snapshot now` exists\n\n{}",
            web_collection_usage_block("yggterm-headless")
        ));
    }
    let now = now_ms();
    let offset = utc_offset_secs(now);
    let (profile, store) = store_for(args)?;

    let mut items: Vec<(String, String)> = Vec::new();
    let urls = repeated_flag_values(args, "--url");
    let titles = repeated_flag_values(args, "--title");
    for (index, url) in urls.iter().enumerate() {
        items.push((
            titles.get(index).copied().unwrap_or(url).to_string(),
            (*url).to_string(),
        ));
    }
    if args.iter().any(|arg| arg == "--stdin") {
        for line in read_stdin()?.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (url, title) = match line.split_once('\t') {
                Some((url, title)) => (url.trim(), title.trim()),
                None => (line, line),
            };
            items.push((title.to_string(), url.to_string()));
        }
    }
    if items.is_empty() {
        return Err(anyhow!(
            "a snapshot with no items is not a browsing session worth a file. Pass \
             --url <u> [--title <t>] (repeatable) or --stdin with `url<TAB>title` lines. \
             The close hook and cadence chore that supply the open tabs are increment I4 \
             (ychrome/docs/collections.md)."
        ));
    }

    let id = store.allocate(&format!("snapshot-{}", format_snapshot_stem(now, offset)));
    let name = cli_flag_value(args, "--name");
    let mut candidate = build_collection(
        &NewCollection {
            id: &id,
            name,
            profile: &profile,
            tags: &[],
            note: None,
        },
        CollectionKind::Snapshot,
        now,
        offset,
    );
    for (title, url) in &items {
        add_link(&mut candidate, None, title, url);
    }

    let previous = store.latest_snapshot();
    if !should_write_snapshot(previous.as_ref().map(|(_, held)| held), &candidate) {
        // THE refusal. An idle browser produces one snapshot, not twenty-four.
        return print(&json!({
            "profile": profile,
            "written": false,
            "reason": "identical to the previous snapshot",
            "previous": previous.map(|(id, _)| id),
            "items": items.len(),
        }));
    }
    save_and_report(
        &store,
        &id,
        &candidate,
        json!({
            "written": true,
            "previous": previous.map(|(id, _)| id),
        }),
    )
}

/// The stem a snapshot id is built from: the local instant, digits only.
/// Deterministic for an instant, and readable in a directory listing.
fn format_snapshot_stem(now_ms: u64, utc_offset_secs: i32) -> String {
    yggterm_core::web_collection_store::format_timestamp(now_ms, utc_offset_secs)
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-01T16:04:12+05:30.
    const AUG_1_2026: u64 = 1_785_580_452_000;
    const IST: i32 = 5 * 3600 + 1800;

    fn entry(ts_ms: u64, url: &str, title: &str) -> WebHistoryEntry {
        WebHistoryEntry {
            ts_ms,
            url: url.to_string(),
            title: title.to_string(),
        }
    }

    #[test]
    fn since_reads_durations_instants_and_all_but_never_guesses() {
        let day = 86_400_000u64;
        assert_eq!(parse_since("7d", AUG_1_2026), Ok(AUG_1_2026 - 7 * day));
        assert_eq!(parse_since("6h", AUG_1_2026), Ok(AUG_1_2026 - 6 * 3_600_000));
        assert_eq!(parse_since("45m", AUG_1_2026), Ok(AUG_1_2026 - 45 * 60_000));
        assert_eq!(parse_since("2w", AUG_1_2026), Ok(AUG_1_2026 - 14 * day));
        assert_eq!(parse_since("all", AUG_1_2026), Ok(0));
        assert_eq!(parse_since("ALL", AUG_1_2026), Ok(0));
        assert_eq!(
            parse_since("2026-08-01T16:04:12+05:30", AUG_1_2026),
            Ok(AUG_1_2026)
        );
        // A `--since` we cannot read adds the WRONG pages, silently — so it is
        // an error, not a fallback.
        for bad in ["", "yesterday", "7", "7x", "-3d", "d"] {
            assert!(parse_since(bad, AUG_1_2026).is_err(), "{bad:?} must refuse");
        }
        // A window longer than the epoch clamps rather than wrapping.
        assert_eq!(parse_since("99999w", 1000), Ok(0));
    }

    #[test]
    fn the_history_window_the_substring_and_the_cap_all_apply() {
        let hour = 3_600_000u64;
        let entries = vec![
            entry(AUG_1_2026, "https://news.example/rust", "Rust news"),
            entry(AUG_1_2026 - hour, "https://docs.example/rust", "The Book"),
            entry(AUG_1_2026 - 48 * hour, "https://old.example/rust", "Ancient"),
            entry(AUG_1_2026 - 2 * hour, "https://other.example/go", "Go news"),
        ];
        let day_ago = parse_since("1d", AUG_1_2026).unwrap();
        let picked = history_selection(&entries, day_ago, None, usize::MAX);
        assert_eq!(picked.len(), 3, "the 48h-old entry is outside the window");
        let matched = history_selection(&entries, day_ago, Some("rust"), usize::MAX);
        assert_eq!(
            matched.iter().map(|e| e.url.as_str()).collect::<Vec<_>>(),
            vec!["https://news.example/rust", "https://docs.example/rust"]
        );
        // The needle reads the TITLE too, not just the url.
        let by_title = history_selection(&entries, day_ago, Some("go news"), usize::MAX);
        assert_eq!(by_title.len(), 1);
        // …and the cap takes the NEWEST, because the reader is newest-first.
        let capped = history_selection(&entries, day_ago, None, 1);
        assert_eq!(capped[0].url, "https://news.example/rust");
    }

    #[test]
    fn a_repeatable_flag_collects_every_occurrence_in_order() {
        let args: Vec<String> = ["collection", "new", "X", "--tag", "a", "--tag", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(repeated_flag_values(&args, "--tag"), vec!["a", "b"]);
        assert!(repeated_flag_values(&args, "--note").is_empty());
        // A flag in the final position has no value and must not panic.
        let trailing: Vec<String> = ["collection", "new", "--tag"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(repeated_flag_values(&trailing, "--tag").is_empty());
    }

    #[test]
    fn a_snapshot_id_is_readable_and_deterministic_for_an_instant() {
        assert_eq!(format_snapshot_stem(AUG_1_2026, IST), "20260801160412");
        assert_eq!(format_snapshot_stem(AUG_1_2026, 0), "20260801103412");
    }

    #[test]
    fn the_usage_block_names_the_binary_it_was_asked_about() {
        assert!(web_collection_usage_block("yggterm-headless")
            .contains("yggterm-headless collection list"));
        assert!(web_collection_usage_block("yggterm").contains("yggterm collection list"));
        // Every verb the dispatcher accepts has to be IN the usage — an agent
        // reading --help and concluding a verb does not exist is a docs bug
        // (docs/agent-control-plane.md, 2026-07-22).
        let usage = web_collection_usage_block("yggterm");
        for verb in [
            "list",
            "show",
            "new",
            "add",
            "add-from-history",
            "move",
            "rename",
            "tag",
            "note",
            "promote",
            "open",
            "export",
            "prune",
            "import",
        ] {
            assert!(
                usage.contains(&format!("collection {verb}")),
                "the usage block never names `collection {verb}`"
            );
        }
        assert!(usage.contains("snapshot now"));
    }

    /// ⚠ THE DISPATCHER-PARITY LOCK. The usage block above is hand-written;
    /// this reads the dispatcher's OWN match arms out of the source and fails
    /// when one is added without a usage line. Same defect class the
    /// `WEB_ACTIONS` lock covers for `server app web`.
    #[test]
    fn every_dispatched_action_is_documented() {
        let source = include_str!("web_collection_cli.rs");
        let body = source
            .split("pub fn run_web_collection_cli(")
            .nth(1)
            .expect("the dispatcher")
            .split("fn run_snapshot(")
            .next()
            .expect("the end of the dispatcher");
        let usage = web_collection_usage_block("yggterm");
        let mut seen = 0usize;
        for line in body.lines().map(str::trim) {
            let Some(rest) = line.strip_suffix(" => {") else {
                continue;
            };
            let Some(name) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
                continue;
            };
            if name == "snapshot" {
                continue;
            }
            seen += 1;
            assert!(
                usage.contains(&format!("collection {name}")),
                "the dispatcher accepts `collection {name}` and the usage block never says so"
            );
        }
        assert!(
            seen >= 13,
            "the parity lock matched only {seen} arms — it is not reading the dispatcher"
        );
    }
}
