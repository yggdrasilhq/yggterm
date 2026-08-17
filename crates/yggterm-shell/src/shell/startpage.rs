#[component]
fn StatusPill(label: String, value: String, palette: Palette) -> Element {
    rsx! {
        div {
            style: format!(
                "display:inline-flex; align-items:center; gap:6px; padding:6px 10px; border-radius:999px; \
                 background:rgba(255,255,255,0.62); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.46);"
            ),
            span {
                style: format!("font-size:11px; font-weight:700; color:{};", palette.muted),
                "{label}"
            }
            span {
                style: format!("font-size:11px; font-weight:700; color:{};", palette.text),
                "{value}"
            }
        }
    }
}
#[component]
fn TerminalCard(title: String, subtitle: String, lines: Vec<String>, palette: Palette) -> Element {
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:12px; background:{}; border:none; \
                 border-radius:14px; padding:15px 16px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.38);",
                palette.panel_alt
            ),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                div {
                    style: format!("font-size:13px; font-weight:700; color:{};", palette.text),
                    "{title}"
                }
                div {
                    style: format!("font-size:11px; color:{};", palette.muted),
                    "{subtitle}"
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:8px; padding:12px; background:{}; \
                     border-radius:12px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.62);",
                    palette.panel
                ),
                for (ix, line) in lines.iter().enumerate() {
                    div {
                        style: format!(
                            "font-size:12px; line-height:1.45; color:{}; white-space:pre-wrap;",
                            if line.starts_with('$') { palette.accent } else { palette.text }
                        ),
                        "{ix + 1:02}  {line}"
                    }
                }
            }
        }
    }
}
#[cfg(test)]
fn start_page_recent_rows(snapshot: &RenderSnapshot) -> Vec<BrowserRow> {
    start_page_recent_rows_from_browser_rows(snapshot, &snapshot.rows)
}

fn start_page_recent_rows_from_browser_rows(
    snapshot: &RenderSnapshot,
    browser_rows: &[BrowserRow],
) -> Vec<BrowserRow> {
    // The scanned last-used times, resolved ONCE per build rather than per row
    // per render — see [`start_page_scanned_last_used_epochs`].
    let scanned = start_page_scanned_last_used_epochs(snapshot);
    start_page_recent_rows_from_browser_rows_with_modified_epochs(
        snapshot,
        browser_rows,
        |row| start_page_row_last_used_epoch(row, &scanned),
    )
}

/// Last-used time, by session id, for every session a SCAN has timestamped.
///
/// ⛔ **This map is the reason the page can be ordered at all.** The rank key
/// used to be read off the row's own path — `std::fs::metadata` on
/// `row.full_path`, returning 0 for anything that was not a local `.jsonl`
/// file. Measured against the live host's 41 session rows on 2026-08-13:
/// **not one of them was a `.jsonl` path.** They are `remote-cc://` (32),
/// `local://` (6), `remote-session://` and `live::` — so the sort key was
/// CONSTANT ZERO across the entire corpus, every comparison fell through to the
/// tie-breaks, and the page came out ordered by scheme-then-uuid. That is what
/// "the order is weird" was: alphabetical by UUID, which is unnameable by
/// construction because a UUID means nothing.
///
/// A row and the scan that timestamped it agree on exactly one thing — the
/// SESSION ID — which is the same key [`start_page_recent_identity_keys`]
/// already dedups on. So the epoch is looked up by id, and a live `remote-cc://`
/// row inherits the mtime of the transcript it is a running instance of.
fn start_page_scanned_last_used_epochs(snapshot: &RenderSnapshot) -> HashMap<String, i64> {
    scanned_last_used_epochs_by_session_id(&snapshot.remote_machines)
}

/// The ONE implementation of "when was this session last used", by session id.
///
/// Two surfaces order by it — the start page's Recent work and the cwd tree's
/// injected live rows ([`RemoteSessionIndex::last_used_epoch`]) — and they read
/// the same function rather than each deriving an epoch of their own, because
/// two encodings of one concept are how the sidebar and the start page came to
/// disagree about what "most recent" means in the first place.
fn scanned_last_used_epochs_by_session_id(
    remote_machines: &[RemoteMachineSnapshot],
) -> HashMap<String, i64> {
    let mut epochs = HashMap::<String, i64>::new();
    for machine in remote_machines {
        for session in &machine.sessions {
            let session_id = session.session_id.trim();
            if session_id.is_empty() || session.modified_epoch <= 0 {
                continue;
            }
            // A session id can be scanned by more than one machine entry; the
            // NEWEST sighting is the one that describes when it was last used.
            epochs
                .entry(session_id.to_string())
                .and_modify(|existing| *existing = (*existing).max(session.modified_epoch))
                .or_insert(session.modified_epoch);
        }
    }
    epochs
}

/// When this row was last used, in epoch seconds; `0` when nothing knows.
///
/// Two sources, in order, and **both are explicit** — a silent mtime fallback
/// is the shape this project has already been bitten by:
///
/// 1. the scan that timestamped this session id (the answer for live rows,
///    remote rows, and anything whose row path is a scheme rather than a file);
/// 2. the stored transcript's own mtime, for a local `.jsonl` row that no scan
///    covered.
///
/// ⚠ Source 2 stats the filesystem, which is why it is LAST and why the result
/// is memoized per build by the caller: `AGENTS.md` names render-path
/// filesystem IO as a bug, and this used to run for every row on every render.
fn start_page_row_last_used_epoch(row: &BrowserRow, scanned: &HashMap<String, i64>) -> i64 {
    if let Some(session_id) = row.session_id.as_deref().map(str::trim)
        && !session_id.is_empty()
        && let Some(epoch) = scanned.get(session_id)
    {
        return *epoch;
    }
    start_page_browser_row_modified_epoch(row)
}

fn start_page_recent_rows_from_browser_rows_with_modified_epochs(
    snapshot: &RenderSnapshot,
    browser_rows: &[BrowserRow],
    mut browser_row_modified_epoch: impl FnMut(&BrowserRow) -> i64,
) -> Vec<BrowserRow> {
    let mut seen = HashSet::<String>::new();
    let active_path = snapshot
        .active_session_path
        .as_deref()
        .map(normalize_live_session_path);
    let scope = start_page_recent_scope(snapshot);
    let live_projection_paths = start_page_live_projection_paths(snapshot);
    let live_paths_for_rank = &live_projection_paths;
    // An app row is not a session anyone resumes — dropped in `push_candidate`
    // so that all three candidate sources are covered by one gate rather than
    // three filters that can drift apart.
    let app_surface_paths = start_page_app_surface_paths(snapshot);
    // ⛔ THE SCOPE RANKS, IT DOES NOT DROP. (Root-caused 2026-08-08.)
    //
    // These predicates used to FILTER, and the header said only "N shown" — so
    // one page read three times with a different row selected answered 188,
    // then 40, then 4, and a row outside the selected row's `{machine_key, cwd}`
    // looked exactly like a row that never existed. The owner hit that removing
    // a delegate and going to the start page to respawn it.
    //
    // The scope is RIGHT for the create buttons ("create work in this scope" is
    // what the subtitle promises) and wrong for a list that
    // [[spec-active-sessions-dual-presence]] binds to showing every session. So
    // it stays, as a RANK: in-scope work still leads, and nothing vanishes.
    let mut candidates = Vec::<(BrowserRow, bool, bool, i64, String, usize)>::new();
    // Dedup on SESSION IDENTITY, not on the path string. A running session and
    // its stored JSONL row are one session under two spellings — `local://<id>`
    // and `~/.codex/sessions/<...>.jsonl` — which no path normalization relates,
    // so a path key would put the same session on the page twice now that the
    // live row is no longer filtered out. The session id is the one thing both
    // spellings agree on; the normalized path is the fallback for rows that
    // carry no id (documents, recipes).
    let mut push_candidate =
        |row: BrowserRow, modified_epoch: i64, started_at: String, in_scope: bool| {
            let keys = start_page_recent_identity_keys(&row);
            if active_path
                .as_deref()
                .is_some_and(|active| active == normalize_live_session_path(&row.full_path))
            {
                return;
            }
            // Gap 3: a libyggterm app row is not a resumable session.
            if app_surface_paths.contains(&row.full_path)
                || app_surface_paths.contains(&normalize_live_session_path(&row.full_path))
            {
                return;
            }
            // ANY key already claimed means this session is already on the page
            // under one of its other names. Then claim them all, so the next
            // spelling matches whichever name it happens to know.
            let fresh = !keys.iter().any(|key| seen.contains(key));
            for key in keys {
                seen.insert(key);
            }
            if fresh {
                let ix = candidates.len();
                // A running session is the most current thing on the page, and a
                // stored transcript's mtime cannot express that — a live row often
                // carries no epoch at all and would sort BELOW week-old files.
                let is_live =
                    live_paths_for_rank.contains(&normalize_live_session_path(&row.full_path));
                candidates.push((row, is_live, in_scope, modified_epoch, started_at, ix));
            }
        };

    // SPEC (user directive 2026-08-06, REVERSING the 2026-05-26 call recorded
    // in [[spec-active-sessions-dual-presence]]): a running session KEEPS its
    // start-page row, and opening it SWITCHES to the running session instead of
    // starting a second one.
    //
    // The old rule stripped every row whose path matched a live session, so the
    // act of launching a session made its row vanish from the start page. That
    // was defensible only under the old rule's own assumption — "live sessions
    // are already surfaced in the Live Sessions sidebar group" — and when the
    // daemon-reachability bug broke that assumption, the strip turned a sidebar
    // outage into total invisibility: the sessions were running, and there was
    // nowhere left in the UI that admitted they existed. The user's words: "If
    // the session is open it should switch to that not LIE about sessions
    // present."
    //
    // So live rows are no longer filtered out; they are PREFERRED. The live row
    // is pushed before the stored row for the same session, and the normalized
    // dedup in `push_candidate` collapses the pair to one row — the live one,
    // which `spawn_open_session_row` routes to a focus rather than a launch.
    // Live rows go in FIRST so that when the same session also has a scanned
    // remote row or a stored JSONL row, the normalized dedup keeps the LIVE
    // one. Order here is the whole mechanism: push a stored row first and the
    // start page would offer "resume" for a session that is already running.
    let mut live_first = browser_rows
        .iter()
        .filter(|row| row.is_start_page_candidate())
        .filter(|row| live_projection_paths.contains(&normalize_live_session_path(&row.full_path)))
        .collect::<Vec<_>>();
    // Same recency order the page sorts by, so that when two live spellings of
    // one session both qualify the winner is deterministic rather than
    // dependent on sidebar assembly order.
    live_first.sort_by(|left, right| {
        browser_row_modified_epoch(right)
            .cmp(&browser_row_modified_epoch(left))
            .then_with(|| left.full_path.cmp(&right.full_path))
    });
    for row in live_first {
        let modified_epoch = browser_row_modified_epoch(row);
        let in_scope = start_page_recent_scope_allows_browser_row(&scope, row);
        push_candidate(row.clone(), modified_epoch, String::new(), in_scope);
    }

    for machine in &snapshot.remote_machines {
        let remote_short_ids = unique_session_short_ids_for_pairs(
            &machine
                .sessions
                .iter()
                .map(|session| (session.session_path.clone(), session.session_id.clone()))
                .collect::<Vec<_>>(),
        );
        for session in &machine.sessions {
            if !remote_scanned_session_is_start_page_durable(session) {
                continue;
            }
            push_candidate(
                browser_row_for_remote_scanned_session(machine, session, &remote_short_ids),
                session.modified_epoch,
                session.started_at.clone(),
                start_page_recent_scope_allows_remote_session(&scope, machine, session),
            );
        }
    }

    for row in browser_rows
        .iter()
        .filter(|row| row.is_start_page_candidate())
    {
        let modified_epoch = browser_row_modified_epoch(row);
        let in_scope = start_page_recent_scope_allows_browser_row(&scope, row);
        push_candidate(row.clone(), modified_epoch, String::new(), in_scope);
    }

    // Running sessions first, then recency. Ordering "by recency" was the
    // 2026-05-25 spec and still governs everything below the live block; live
    // rows sit above it because "running right now" outranks any file mtime,
    // and because a live row's epoch is frequently 0.
    //
    // ⭐ THEN the scope, which is where a filter used to be. In-scope work leads
    // the list, exactly as it did when everything else was thrown away — the
    // difference is that "further down" replaced "gone", and a row the owner is
    // looking for can now be FOUND rather than only remembered.
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| right.3.cmp(&left.3))
            .then_with(|| right.4.cmp(&left.4))
            .then_with(|| left.5.cmp(&right.5))
    });
    candidates.into_iter().map(|(row, ..)| row).collect()
}

/// The two start page split-button families.
///
/// Sessions and apps are separate controls rather than one long list because
/// they are separate decisions: "what am I about to work in" versus "what tool
/// am I opening". Collapsing both into a single menu would put Yggdrasil Maker
/// one row below Claude Code and make the frequent choice hunt for itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StartPageFamily {
    Session,
    App,
}

/// The SESSION family, in menu order.
///
/// Order is deliberate and not frequency-sorted: a menu that reorders itself
/// under the user destroys the muscle memory the sticky face is there to build.
/// The face moves; the list does not.
/// The SESSION family: one member per REGISTERED agent CLI, in registry order —
/// the same derivation the cwd-tree row menu uses, so the two surfaces cannot
/// offer different CLIs. It used to be two hardcoded entries, which is one of
/// the four places a new CLI had to be remembered by hand.
///
/// ⛔ **A plain terminal is NOT a member** (owner directive 2026-08-08: *"New
/// terminal should be a separate button and not included in sessions"*). It sat
/// in the family as an unaccented last entry, and being in the family means
/// being STICKY: choosing a shell once made "New Terminal" the face, so the
/// button the user reaches for by muscle memory to start an agent quietly
/// started a shell instead. It also grew from one of two entries to one of ten
/// as the CLIs landed, so the shell became the least findable thing on a page
/// whose whole job is starting one. It is [`start_page_new_terminal_button`]
/// now — always visible, never the face of something else.
fn start_page_session_items(accent: &str) -> Vec<SplitButtonItem> {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .map(|descriptor| {
            SplitButtonItem::new(descriptor.slug, format!("{} Session", descriptor.display_name))
                .detail(format!(
                    "Start {} in the selected scope",
                    descriptor.display_name
                ))
                .accent(session_kind_primary_bg(descriptor.kind, accent))
        })
        .collect()
}

/// The id an app verb answers to, and the key its stickiness is stored under.
fn start_page_app_item_id(app_name: &str, verb_id: &str) -> String {
    format!("app:{app_name}:{verb_id}")
}

/// The APPS family, read from the SAME launcher registry the titlebar `+` menu
/// and the cwd-tree context menu use — one list, three surfaces.
fn start_page_app_items(apps: &[AppManifest]) -> Vec<SplitButtonItem> {
    app_launcher_entries(apps)
        .into_iter()
        .map(|(app, verb)| {
            let label = if app.icon.trim().is_empty() {
                verb.label.clone()
            } else {
                format!("{} {}", app.icon.trim(), verb.label)
            };
            SplitButtonItem::new(start_page_app_item_id(&app.name, &verb.id), label)
        })
        .collect()
}

/// Dress the shared split button in yggterm's own palette.
///
/// One adapter, so both families cannot drift apart, and so the component keeps
/// owning material while `DESIGN.md` keeps owning colour. Every value here is
/// already what the buttons this control replaced were using — the shape
/// changed, the palette did not.
/// ⛔ The BUTTON's fill and the MENU's fill are two decisions, and they were one
/// until 2026-08-08. `panel_alt` is a tint — right for the button, because the
/// start page is opaque behind it — and it made the floating menu see-through.
/// The menu reads [`overlay_surface`], the same fill the right-click menu uses,
/// which is the reference the owner named.
fn start_page_split_palette(palette: &Palette) -> SplitButtonPalette {
    SplitButtonPalette::new(
        palette.text,
        palette.muted,
        palette.panel_alt,
        overlay_surface(*palette),
        "rgba(120,142,166,0.16)",
        palette.panel,
        palette.accent,
        "#ffffff",
    )
}

/// Run one member of the SESSION family and make it the sticky face.
///
/// The phantom-start guard is preserved per member: the start page can be
/// out from under a queued click, and starting a session the user did not ask
/// for is worse than dropping the click.
fn start_page_run_session_choice(
    mut state: Signal<ShellState>,
    id: &str,
    row: Option<BrowserRow>,
) {
    let agent_kind = yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .find(|descriptor| descriptor.slug == id)
        .map(|descriptor| descriptor.kind);
    // The trace name is per-family, not per-CLI: a name minted from the slug
    // would make every new CLI a new event nobody has a dashboard for, and the
    // thing being traced is "the start page launched a session", which the
    // payload already qualifies.
    let (trace_name, id_owned) = match agent_kind {
        Some(_) => ("start_page_new_agent_session", id.to_string()),
        // A plain shell is no longer a family member — it is its own button
        // (`start_page_run_new_terminal`), so an id that names no CLI names
        // nothing this function can run.
        None => return,
    };
    if !start_page_is_current_surface(&state) {
        suppress_phantom_start_action(
            trace_name,
            json!({ "row": row.as_ref().map(|row| row.full_path.clone()) }),
        );
        return;
    }
    // Remembered BEFORE the spawn, so the face is right even if the launch
    // itself fails — the button records what you chose, not what succeeded.
    state.with_mut_counted(|shell| {
        shell.remember_start_page_choice(StartPageFamily::Session, &id_owned);
    });
    match agent_kind {
        // Codex means "whatever the user set as their default agent" — the
        // `AgentSessionProfile` setting picks the fork, as it always has.
        Some(SessionKind::Codex) => spawn_start_preferred_agent_session_for_row(state, row),
        Some(kind) => spawn_start_agent_session_for_row(state, row, kind),
        None => {}
    }
}

/// Start a plain shell from the start page's own button.
///
/// Not a family member and therefore NOT sticky: it starts a shell every time
/// and never becomes the face of the session button. That is the whole point of
/// the split — see [`start_page_session_items`].
///
/// The phantom-start guard is the same one every family member gets, and for
/// the same reason: the start page can go out from under a queued click, and
/// starting a session the user did not ask for is worse than dropping the
/// click. Sharing the guard rather than reimplementing it is what keeps the two
/// paths from drifting.
fn start_page_run_new_terminal(state: Signal<ShellState>, row: Option<BrowserRow>) {
    if !start_page_is_current_surface(&state) {
        suppress_phantom_start_action(
            "start_page_new_terminal",
            json!({ "row": row.as_ref().map(|row| row.full_path.clone()) }),
        );
        return;
    }
    spawn_start_terminal_session_for_row(state, row);
}

/// Run one member of the APPS family and make it the sticky face.
///
/// The id is resolved back through the launcher registry rather than parsed,
/// so an id naming an app that is no longer installed simply finds nothing and
/// does nothing.
fn start_page_run_app_choice(
    mut state: Signal<ShellState>,
    id: &str,
    apps: &[AppManifest],
    row: Option<BrowserRow>,
) {
    let Some((app, verb)) = app_launcher_entries(apps)
        .into_iter()
        .find(|(app, verb)| start_page_app_item_id(&app.name, &verb.id) == id)
    else {
        return;
    };
    if !start_page_is_current_surface(&state) {
        suppress_phantom_start_action(
            "start_page_launch_app_verb",
            json!({ "app": app.name, "verb": verb.id }),
        );
        return;
    }
    let id_owned = id.to_string();
    state.with_mut_counted(|shell| {
        shell.remember_start_page_choice(StartPageFamily::App, &id_owned);
    });
    let insert_after = row.as_ref().map(|row| row.full_path.clone());
    let launch_context = launch_context_for_optional_row(state, row);
    spawn_launch_app_verb(state, app, verb, launch_context, insert_after);
}

/// The identity a start-page row is deduped by: one SESSION, one card.
///
/// A live row and the stored transcript row for the same session are the same
/// thing wearing two paths, and only the session id relates them. Rows without
/// an id fall back to the normalized path, which is what documents and terminal
/// recipes are keyed by.
/// EVERY name one session answers to, so a dedup on any one of them collapses
/// the rest.
///
/// ⛔ It used to return ONE key — the session id when there was one, the
/// normalized path otherwise — and that is a dedup that misses exactly when two
/// spellings of one session disagree about whether they carry an id. The sidebar
/// holds precisely that pair: the live row knows its session id and the folder
/// row for the same session does not, so the two keys never met and the page
/// listed the session twice.
///
/// It went unseen because the scope FILTER happened to drop the second copy
/// first: `start_page_recent_scope_allows_browser_row` refuses any row with no
/// `session_cwd`, which is the same rows that carry no id. Turning that filter
/// into a rank (2026-08-08) removed the mask and the duplicate appeared — which
/// is the honest order of events, and the reason this is fixed in the same
/// change rather than shipped as a "new" bug.
fn start_page_recent_identity_keys(row: &BrowserRow) -> Vec<String> {
    let path_key = format!("path:{}", normalize_live_session_path(&row.full_path));
    match row.session_id.as_deref().map(str::trim) {
        Some(session_id) if !session_id.is_empty() => {
            vec![format!("session-id:{session_id}"), path_key]
        }
        _ => vec![path_key],
    }
}

/// Every row path that is a libyggterm APP rather than a session.
///
/// ⛔ **The start page's job is picking a session to resume, and an app row is
/// not something anyone resumes** — ychrome and yedit rows sat among the agent
/// sessions as pure noise on the one surface whose whole purpose is finding
/// work again.
///
/// The discriminator is the row's persisted `Source` stamp, which an app launch
/// writes as the `app:<name>:<verb>` token — by its own documentation "the one
/// place a row says WHICH app it is", and the same token the daemon re-derives
/// the row's command from across a restart. Parsed with
/// [`yggterm_server::app_verb_token_parts`], the token's ONE parser.
///
/// ⚠ It is deliberately NOT the label. An app row is a `SessionKind::Shell` on
/// a `local://` path, so kind and scheme cannot separate it from a plain shell,
/// and the only other visible signal is that its title happens to read
/// "New Ychrome" — which is a string an app or a user may change at any moment.
/// Sniffing that would be a second encoding of a fact the `Source` stamp
/// already owns.
fn start_page_app_surface_paths(snapshot: &RenderSnapshot) -> HashSet<String> {
    snapshot
        .live_sessions
        .iter()
        .filter(|session| {
            session.metadata.iter().any(|entry| {
                entry.label == "Source"
                    && yggterm_server::app_verb_token_parts(&entry.value).is_some()
            })
        })
        .flat_map(|session| {
            [
                session.session_path.clone(),
                normalize_live_session_path(&session.session_path),
            ]
        })
        .collect()
}

fn start_page_live_projection_paths(snapshot: &RenderSnapshot) -> HashSet<String> {
    snapshot
        .live_sessions
        .iter()
        .flat_map(|session| {
            [
                session.session_path.clone(),
                normalize_live_session_path(&session.session_path),
            ]
        })
        .collect()
}

fn remote_scanned_session_is_start_page_durable(session: &RemoteScannedSession) -> bool {
    !session.storage_path.trim().is_empty()
        || yggterm_core::agent_scheme::remote_agent_row_schemes()
            .any(|scheme| session.session_path.starts_with(scheme.prefix))
}

fn start_page_browser_row_modified_epoch(row: &BrowserRow) -> i64 {
    if row.kind != BrowserRowKind::Session {
        return 0;
    }
    if row.full_path.starts_with("remote-session://") || row.full_path.starts_with("ssh://") {
        return 0;
    }
    let path = Path::new(&row.full_path);
    if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
        return 0;
    }
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StartPageRecentScope {
    machine_key: Option<String>,
    cwd: Option<String>,
}

const LOCAL_START_PAGE_RECENT_MACHINE_KEY: &str = "__local__";

fn start_page_recent_scope_is_local_row(row: &BrowserRow) -> bool {
    let host_label = row.host_label.trim();
    if !host_label.is_empty() && host_label != "local" {
        return false;
    }
    let normalized_path = row.full_path.trim_start_matches('/');
    if normalized_path.starts_with("remote-session://")
        || normalized_path.starts_with("__remote_folder__/")
        || normalized_path.starts_with("__remote_machine__/")
    {
        return false;
    }
    row.full_path == "local"
        || row.full_path.starts_with('/')
        || row.full_path.starts_with("local://")
}

fn start_page_recent_scope(snapshot: &RenderSnapshot) -> Option<StartPageRecentScope> {
    let row = snapshot.selected_row.as_ref()?;
    if let Some((machine_key, _)) = parse_remote_scanned_session_path(&row.full_path) {
        return Some(StartPageRecentScope {
            machine_key: Some(machine_key.to_string()),
            cwd: row.session_cwd.clone(),
        });
    }
    if start_page_recent_scope_is_local_row(row) {
        let cwd = group_session_cwd(row).or_else(|| row.session_cwd.clone());
        return Some(StartPageRecentScope {
            machine_key: Some(LOCAL_START_PAGE_RECENT_MACHINE_KEY.to_string()),
            cwd: cwd.map(|value| normalize_recent_scope_cwd(&value)),
        });
    }
    if let Some(cwd) = remote_folder_cwd(row).or_else(|| row.session_cwd.clone()) {
        let normalized_path = row.full_path.trim_start_matches('/');
        let machine_key = if row.host_label.trim().is_empty() {
            normalized_path
                .strip_prefix("__remote_folder__/")
                .and_then(|rest| rest.split_once('/').map(|(machine, _)| machine.to_string()))
        } else {
            Some(row.host_label.clone())
        };
        let machine_key = machine_key.or_else(|| {
            normalized_path
                .strip_prefix("__remote_folder__/")
                .and_then(|rest| rest.split_once('/').map(|(machine, _)| machine.to_string()))
        });
        return Some(StartPageRecentScope {
            machine_key,
            cwd: Some(normalize_recent_scope_cwd(&cwd)),
        });
    }
    if let Some(machine_key) = row
        .full_path
        .strip_prefix("__remote_machine__/")
        .filter(|value| !value.trim().is_empty())
    {
        return Some(StartPageRecentScope {
            machine_key: Some(machine_key.to_string()),
            cwd: None,
        });
    }
    None
}

fn normalize_recent_scope_cwd(cwd: &str) -> String {
    let trimmed = cwd.trim();
    if trimmed == "/" {
        return "/".to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn recent_cwd_matches_scope(cwd: &str, scope_cwd: &str) -> bool {
    let cwd = normalize_recent_scope_cwd(cwd);
    let scope_cwd = normalize_recent_scope_cwd(scope_cwd);
    if cwd == scope_cwd {
        return true;
    }
    if scope_cwd == "/" {
        return cwd.starts_with('/');
    }
    cwd.strip_prefix(&scope_cwd)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn start_page_recent_scope_allows_remote_session(
    scope: &Option<StartPageRecentScope>,
    machine: &RemoteMachineSnapshot,
    session: &RemoteScannedSession,
) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    if let Some(machine_key) = scope.machine_key.as_deref() {
        if machine_key == LOCAL_START_PAGE_RECENT_MACHINE_KEY {
            return false;
        }
        if machine.machine_key != machine_key {
            return false;
        }
    }
    if let Some(cwd) = scope.cwd.as_deref() {
        if !recent_cwd_matches_scope(&session.cwd, cwd) {
            return false;
        }
    }
    true
}

fn start_page_recent_scope_allows_browser_row(
    scope: &Option<StartPageRecentScope>,
    row: &BrowserRow,
) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    if let Some(machine_key) = scope.machine_key.as_deref() {
        if machine_key == LOCAL_START_PAGE_RECENT_MACHINE_KEY {
            if parse_remote_scanned_session_path(&row.full_path).is_some() {
                return false;
            }
            let host_label = row.host_label.trim();
            if !host_label.is_empty() && host_label != "local" {
                return false;
            }
        } else if let Some((row_machine, _)) = parse_remote_scanned_session_path(&row.full_path) {
            if row_machine != machine_key {
                return false;
            }
        } else if row.host_label != machine_key {
            return false;
        }
    }
    if let Some(cwd) = scope.cwd.as_deref() {
        let Some(row_cwd) = row.session_cwd.as_deref() else {
            return false;
        };
        if !recent_cwd_matches_scope(row_cwd, cwd) {
            return false;
        }
    }
    true
}

fn browser_row_for_remote_scanned_session(
    machine: &RemoteMachineSnapshot,
    session: &RemoteScannedSession,
    short_ids: &HashMap<String, String>,
) -> BrowserRow {
    let label = remote_scanned_session_label(session, short_ids);
    let session_kind = yggterm_core::agent_scheme::session_kind_for_path(&session.session_path)
        .or(Some(SessionKind::Codex));
    BrowserRow {
        kind: BrowserRowKind::Session,
        full_path: session.session_path.clone(),
        label: label.clone(),
        detail_label: session
            .cached_summary
            .clone()
            .filter(|summary| !summary.trim().is_empty())
            .filter(|summary| !memoized_low_signal_generated_copy(summary))
            .unwrap_or_default(),
        document_kind: None,
        group_kind: None,
        session_title: Some(label),
        depth: 0,
        host_label: machine.machine_key.clone(),
        descendant_sessions: 1,
        expanded: true,
        session_id: Some(session.session_id.clone()),
        session_cwd: Some(session.cwd.clone()),
        session_kind,
    }
}

fn start_page_row_context(row: &BrowserRow) -> String {
    if !row.detail_label.trim().is_empty() {
        return row.detail_label.clone();
    }
    if let Some(cwd) = row
        .session_cwd
        .as_ref()
        .filter(|cwd| !cwd.trim().is_empty())
    {
        return cwd.clone();
    }
    if !row.host_label.trim().is_empty() {
        return row.host_label.clone();
    }
    match row.kind {
        BrowserRowKind::Document => "Saved workspace item".to_string(),
        _ => "Saved terminal session".to_string(),
    }
}

#[component]
fn StartPage(snapshot: SharedSnapshot, state: Signal<ShellState>) -> Element {
    let palette = snapshot.palette;
    let all_sidebar_rows = state.read().all_sidebar_rows_for_selection();
    let ordered_rows =
        start_page_recent_rows_from_browser_rows(snapshot.as_ref(), &all_sidebar_rows);
    let ordered_count = ordered_rows.len();
    // Gap 2, and the load-bearing one: the page is reached WHEN THE SIDEBAR HAS
    // FAILED, and at that moment the reader knows what a session was DOING, not
    // what it was called. So the filter runs over the same blob the sidebar's
    // search does — `row_search_blob`, which already carries the generated
    // summary, the title, the cwd, the host, the session id and the transcript
    // context — rather than over the label alone.
    //
    // ⭐ ONE predicate, deliberately: reusing `row_matches_search` means the
    // start page and the cwd tree cannot disagree about what a query matches,
    // which is the failure mode the holistic-spec rule exists to prevent. It is
    // also already memoized (an LRU over rows+query, and a fingerprint-gated
    // transcript context store), so it is safe to run on every keystroke — an
    // unbounded re-scan of the session stores is exactly what this must not be.
    let mut start_page_query = use_signal(String::new);
    let query = start_page_query();
    let terms = search_terms(&query);
    let recent_rows = if terms.is_empty() {
        ordered_rows
    } else {
        ordered_rows
            .into_iter()
            .filter(|row| row_matches_search(row, &terms))
            .collect::<Vec<_>>()
    };
    let recent_count = recent_rows.len();
    let searching = !terms.is_empty();
    let search_field_style = format!(
        "min-width:0; width:220px; height:28px; padding:0 10px; border:none; border-radius:7px; \
         background:{}; color:{}; font-size:12px; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.20);",
        palette.panel, palette.text
    );
    let selected_action_row = snapshot.selected_row.clone();
    let selected_agent_action_row = selected_action_row.clone();
    let selected_app_action_row = selected_action_row.clone();
    let selected_terminal_action_row = selected_action_row.clone();
    let session_items = start_page_session_items(&palette.accent);
    let app_items = start_page_app_items(&snapshot.apps);
    let can_create_folder_in_selected = selected_action_row
        .as_ref()
        .is_some_and(|row| row.full_path == "local" || is_workspace_row(row));
    let quick_button_style = format!(
        "display:inline-flex; align-items:center; justify-content:center; gap:8px; min-height:34px; \
         padding:0 13px; border:none; border-radius:8px; background:{}; color:{}; \
         font-size:12px; font-weight:700; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.16);",
        palette.panel_alt, palette.text
    );
    // `quick_button_style` survives for "New Folder", which is NOT a family
    // member: it is conditional on the selection and belongs to no menu.
    rsx! {
        style { {SPLIT_BUTTON_CSS} }
        div {
            "data-yggterm-start-page": "1",
            "data-yggterm-start-page-recent-count": "{recent_count}",
            style: "display:flex; align-items:stretch; justify-content:center; width:100%; height:100%; overflow:auto;",
            div {
                style: "display:flex; flex-direction:column; gap:24px; width:min(880px, 100%); margin:auto; padding:44px 42px;",
                div {
                    style: "display:flex; flex-direction:column; gap:10px;",
                    div {
                        style: format!("font-size:13px; line-height:1.2; font-weight:800; color:{};", palette.accent),
                        "Yggterm"
                    }
                    div {
                        style: format!("font-size:28px; line-height:1.16; font-weight:800; color:{}; letter-spacing:0;", palette.text),
                        "Start a session"
                    }
                    div {
                        style: format!("font-size:13px; line-height:1.65; color:{}; max-width:620px;", palette.muted),
                        "Open recent work, start a local terminal, or create work in this scope."
                    }
                }
                div {
                    style: "display:flex; flex-wrap:wrap; gap:10px;",
                    // TWO controls, not one per startable thing. This row had
                    // grown to seven buttons, which the owner called ugly and
                    // which also lied about frequency — seven equal buttons
                    // imply the seventh is as likely as the first. Each family
                    // collapses to one split button whose face is the member
                    // last run. See `yggui::split_button`.
                    SplitButton {
                        palette: start_page_split_palette(&palette),
                        items: session_items.clone(),
                        selected_id: snapshot.start_page_session_choice.clone(),
                        open: snapshot.start_page_session_menu_open,
                        prefix: "New".to_string(),
                        on_open_change: move |open: bool| {
                            state.with_mut_counted(|shell| {
                                shell.close_start_page_menus();
                                shell.start_page_session_menu_open = open;
                            });
                        },
                        on_activate: {
                            let row = selected_agent_action_row.clone();
                            move |id: String| {
                                start_page_run_session_choice(state, &id, row.clone());
                            }
                        },
                    }
                    // Contributed by the libyggterm apps installed on this host.
                    // Same registry the titlebar `+` menu and the cwd-tree
                    // context menu read — one list, three surfaces.
                    SplitButton {
                        palette: start_page_split_palette(&palette),
                        items: app_items.clone(),
                        selected_id: snapshot.start_page_app_choice.clone(),
                        open: snapshot.start_page_app_menu_open,
                        // No prefix here. App verbs name themselves already
                        // ("New Ychrome", "Open Yggdrasil Maker"), so a prefix
                        // stutters into "Open New Ychrome". The session family
                        // takes one because its members are nouns.
                        on_open_change: move |open: bool| {
                            state.with_mut_counted(|shell| {
                                shell.close_start_page_menus();
                                shell.start_page_app_menu_open = open;
                            });
                        },
                        on_activate: {
                            let row = selected_app_action_row.clone();
                            let apps = snapshot.apps.clone();
                            move |id: String| {
                                start_page_run_app_choice(state, &id, &apps, row.clone());
                            }
                        },
                    }
                    // A plain shell, on its own button and never inside the
                    // session menu (owner directive 2026-08-08). Unconditional,
                    // unlike "New Folder" below: a terminal in the selected
                    // scope is always startable, and the point of pulling it
                    // out of the family was that it should always be one click.
                    button {
                        r#type: "button",
                        "data-yggterm-start-action": "terminal",
                        style: "{quick_button_style}",
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        onclick: {
                            let row = selected_terminal_action_row.clone();
                            move |evt: MouseEvent| {
                                evt.prevent_default();
                                evt.stop_propagation();
                                start_page_run_new_terminal(state, row.clone());
                            }
                        },
                        "New Terminal"
                    }
                    if can_create_folder_in_selected {
                        if let Some(row) = selected_action_row.clone() {
                            button {
                                r#type: "button",
                                "data-yggterm-start-action": "folder",
                                style: "{quick_button_style}",
                                onmousedown: |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                },
                                onclick: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    queue_new_group_for_row(state, row.clone());
                                },
                                "New Folder"
                            }
                        }
                    }
                    // Rename + Edit Summary intentionally NOT in this header. Both
                    // operate on snapshot.selected_row (sidebar selection) — a target
                    // that's invisible from the start page perspective. Per-card
                    // pencils at ~line 64063 (rename title) and ~line 64114 (edit
                    // summary) provide the same actions with clear contextual scope.
                    // Header buttons here only host actions that DON'T have a per-row
                    // equivalent (New Codex/Claude/Terminal/Folder).
                }
                div {
                    style: "display:flex; flex-direction:column; gap:10px; min-width:0;",
                    div {
                        style: "display:flex; align-items:center; justify-content:space-between; gap:16px; flex-wrap:wrap;",
                        div {
                            style: "display:flex; align-items:baseline; gap:8px; min-width:0;",
                            div {
                                style: format!("font-size:12px; font-weight:800; color:{}; text-transform:uppercase; letter-spacing:0;", palette.muted),
                                "Recent work"
                            }
                            // ⭐ THE ORDERING RULE, SAID OUT LOUD. The page was
                            // reported as ordered "weird" — and the reporter
                            // could not name the rule, which is the finding: an
                            // order nobody can name cannot be trusted or used.
                            // Naming it makes it falsifiable by the person
                            // reading the page.
                            div {
                                "data-yggterm-start-page-order-rule": "most-recently-used",
                                style: format!("font-size:11px; color:{}; text-transform:none;", palette.muted),
                                "most recently used first"
                            }
                        }
                        div {
                            style: "display:flex; align-items:center; gap:10px;",
                            input {
                                "data-yggterm-start-page-search": "1",
                                r#type: "search",
                                value: "{query}",
                                placeholder: "Search title, summary, path",
                                style: "{search_field_style}",
                                oninput: move |evt| start_page_query.set(evt.value()),
                            }
                            div {
                                "data-yggterm-start-page-recent-count": "{recent_count}",
                                style: format!("font-size:12px; color:{}; white-space:nowrap;", palette.muted),
                                if searching {
                                    "{recent_count} of {ordered_count}"
                                } else {
                                    "{recent_count} shown"
                                }
                            }
                        }
                    }
                    if recent_rows.is_empty() {
                        div {
                            "data-yggterm-start-page-empty-recent": "1",
                            style: format!(
                                "display:flex; align-items:center; min-height:72px; padding:18px; border-radius:8px; \
                                 background:{}; color:{}; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.14); font-size:13px;",
                                palette.panel_alt, palette.muted
                            ),
                            // A search that found nothing is a different state
                            // from having no sessions, and saying "no saved
                            // sessions yet" to someone with 44 of them reads as
                            // the page having lost their work — which, on the
                            // surface reached BECAUSE work went missing, is the
                            // worst sentence available.
                            if searching {
                                "No session matches that. The search covers each session's title, its generated summary, its folder and its host."
                            } else {
                                "No saved sessions yet. Start a session or create work in this scope."
                            }
                        }
                    }
                    for row in recent_rows.into_iter() {
                        {
                            let row_for_click = row.clone();
                            let row_for_title_rename = row.clone();
                            let row_for_summary = row.clone();
                            let row_for_context_menu = row.clone();
                            let row_for_delete = row.clone();
                            let context = start_page_row_context(&row);
                            let session_uuid = row.session_id.clone().unwrap_or_default();
                            let host = if row.host_label.trim().is_empty() {
                                "local".to_string()
                            } else {
                                row.host_label.clone()
                            };
                            // NAME THE CLI, and paint it. Both come from the
                            // agent-CLI registry, so a tenth CLI arrives with a
                            // verb and a colour already correct — this used to
                            // be two parallel `match`es that between them knew
                            // about three of the nine registered CLIs and
                            // answered a bare grey "Open" for the rest.
                            //
                            // ⚠ The generic arm is NOT dead: a plain shell and a
                            // terminal recipe have no CLI to name, and they keep
                            // the accent-on-panel treatment precisely so the
                            // solid brand fill continues to mean "this is an
                            // agent session".
                            let row_kind = row_session_kind(&row);
                            let open_button_label =
                                yggterm_core::agent_cli::agent_cli_open_session_label(row_kind);
                            let open_button_style = match row_kind
                                .and_then(yggterm_core::agent_cli::agent_cli_brand_color)
                            {
                                Some(brand) => format!(
                                    "display:inline-flex; align-items:center; justify-content:center; \
                                     min-height:28px; padding:0 10px; border:none; border-radius:7px; \
                                     background:{brand}; color:white; font-size:12px; font-weight:800;",
                                ),
                                None => format!(
                                    "display:inline-flex; align-items:center; justify-content:center; \
                                     min-height:28px; padding:0 10px; border:none; border-radius:7px; \
                                     background:{}; color:{}; font-size:12px; font-weight:800;",
                                    palette.panel, palette.accent
                                ),
                            };
                            let card_icon_button_style = format!(
                                "display:inline-flex; align-items:center; justify-content:center; width:28px; height:28px; \
                                 border:none; border-radius:7px; background:{}; color:{}; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.14); cursor:pointer;",
                                palette.panel, palette.text
                            );
                            let card_delete_button_style = format!(
                                "display:inline-flex; align-items:center; justify-content:center; width:28px; height:28px; \
                                 border:none; border-radius:7px; background:{}; color:{}; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.14); cursor:pointer;",
                                palette.panel, palette.close_hover
                            );
                            rsx! {
                                div {
                                    "data-yggterm-start-page-recent-session": "1",
                                    // §8 — lists are navigated, not badged. Each
                                    // card carries four affordances, and this list
                                    // is as long as the user's history: measured on
                                    // a 283-row start page the ALT layer derived
                                    // 1,135 badges in ONE scope, more than the
                                    // alphabet can even address. So the four are
                                    // per-element exempt with the `list-item`
                                    // reason (the row menu and the cwd tree reach
                                    // the same actions), and the layer badges the
                                    // NAMED chrome above them.
                                    "data-session-path": "{row.full_path}",
                                    style: format!(
                                        "display:grid; grid-template-columns:minmax(0,1fr) auto; gap:14px; align-items:center; width:100%; text-align:left; \
                                         border:none; border-radius:8px; padding:14px 15px; background:{}; color:{}; \
                                         box-shadow:inset 0 0 0 1px rgba(120,142,166,0.14);",
                                        palette.panel_alt, palette.text
                                    ),
                                    oncontextmenu: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        let coords = evt.client_coordinates();
                                        state.with_mut_counted(|shell| {
                                            shell.select_tree_row(&row_for_context_menu, TreeSelectionMode::Replace);
                                            shell.open_context_menu(
                                                row_for_context_menu.clone(),
                                                (coords.x, coords.y),
                                            );
                                        });
                                    },
                                    div {
                                        style: "display:flex; flex-direction:column; gap:5px; min-width:0;",
                                        div {
                                            style: "display:flex; align-items:center; gap:8px; min-width:0;",
                                            span {
                                                style: format!("font-size:13px; line-height:1.3; font-weight:800; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", palette.text),
                                                "{row.label}"
                                            }
                                            button {
                                                r#type: "button",
                                                "data-yggterm-start-action": "rename-recent",
                                                "data-keytip-exempt": "list-item",
                                                title: "Rename title",
                                                style: "{card_icon_button_style}",
                                                onmousedown: |evt| {
                                                    evt.prevent_default();
                                                    evt.stop_propagation();
                                                },
                                                onclick: move |evt| {
                                                    evt.prevent_default();
                                                    evt.stop_propagation();
                                                    queue_copy_edit_for_row(state, row_for_title_rename.clone(), CopyEditField::Title);
                                                },
                                                PencilIcon { size: 12 }
                                            }
                                            span {
                                                style: format!("font-size:11px; line-height:1.3; color:{}; flex:0 0 auto;", palette.muted),
                                                "{host}"
                                            }
                                        }
                                        if !session_uuid.is_empty() {
                                            div {
                                                "data-yggterm-start-page-session-uuid": "1",
                                                style: format!(
                                                    "font-family:'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
                                                     font-size:10px; line-height:1.35; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                                                    palette.muted
                                                ),
                                                "{session_uuid}"
                                            }
                                        }
                                        div {
                                            "data-yggterm-start-summary-timeline": "1",
                                            style: format!(
                                                "position:relative; padding-left:11px; border-left:1px solid rgba(126,150,176,0.28); \
                                                 font-size:12px; line-height:1.45; color:{}; overflow:hidden; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical;",
                                                palette.muted
                                            ),
                                            "{context}"
                                        }
                                    }
                                    div {
                                        style: "display:flex; align-items:center; gap:7px; justify-content:flex-end; flex-wrap:wrap;",
                                        button {
                                            r#type: "button",
                                            "data-yggterm-start-action": "open-recent",
                                            "data-keytip-exempt": "list-item",
                                            style: "{open_button_style}",
                                            onclick: move |_| spawn_open_session_row(state, row_for_click.clone()),
                                            "{open_button_label}"
                                        }
                                        button {
                                            r#type: "button",
                                            "data-yggterm-start-action": "summary-recent",
                                            "data-keytip-exempt": "list-item",
                                            title: "Edit summary",
                                            style: "{card_icon_button_style}",
                                            onmousedown: |evt| {
                                                evt.prevent_default();
                                                evt.stop_propagation();
                                            },
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                evt.stop_propagation();
                                                queue_copy_edit_for_row(state, row_for_summary.clone(), CopyEditField::Summary);
                                            },
                                            PencilIcon { size: 12 }
                                        }
                                        button {
                                            r#type: "button",
                                            "data-yggterm-start-action": "delete-recent",
                                            "data-keytip-exempt": "list-item",
                                            title: "Delete session",
                                            style: "{card_delete_button_style}",
                                            onmousedown: |evt| {
                                                evt.prevent_default();
                                                evt.stop_propagation();
                                            },
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                evt.stop_propagation();
                                                state.with_mut_counted(|shell| {
                                                    shell.open_delete_dialog_for_row(&row_for_delete, false);
                                                });
                                            },
                                            TrashIcon { size: 12 }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
/// What the side rail is actually RENDERING this frame.
///
/// ⚠ `requested_mode` and `rendered_mode` differ far more often than the name
/// "hidden" suggests, and the gap is invisible from `right_panel_mode` alone:
/// **a rail that is not docked still renders a body** (the hover-reveal card
/// draws it), resolved from `right_panel_reveal_mode` rather than from what the
/// shell asked for. A probe that reports only the requested mode therefore says
/// `hidden` while a fully-painted rail is on screen — which is exactly how a
/// foreign app's tab rail beside a yedit row survived two reproductions without
/// the instruments naming what was drawing it (2026-08-08).
///
/// One owner for the question, read by [`RightRail`] and by the agent probe, so
/// the pixels and `server app state` cannot disagree about which body is up.
#[derive(Clone, Debug, PartialEq)]
struct RailRenderView {
    /// The mode the shell asked for: `RenderSnapshot::right_panel_mode`, whose
    /// app-pane tenancy `displayed_right_panel_mode` has already resolved.
    requested_mode: RightPanelMode,
    /// The mode whose BODY is built — `requested_mode` when the rail is docked,
    /// otherwise the reveal mode the collapsed card falls back to.
    rendered_mode: RightPanelMode,
    /// Is the rail docked in flow? False = collapsed or hover-revealed, both of
    /// which still render `rendered_mode`.
    docked: bool,
}
fn rail_render_view(snapshot: &RenderSnapshot) -> RailRenderView {
    let requested_mode = snapshot.right_panel_mode.clone();
    // The mode a HIDDEN rail reveals to is the shell's authoritative
    // `right_panel_reveal_mode` — `right_panel_restore_mode` (the last mode it
    // actually showed) RESOLVED through the same tenancy/liveness rules the
    // docked rail gets, so the reveal can never surface a dead `WebTabs` rail
    // ("No web surface is open") on a session that never had a surface, and a
    // document app's own sidebar reveals instead (the yedit hidden-sidebar
    // bug). The resize-grip un-hide uses the SAME value, so hover-reveal and
    // drag-dock can never disagree. Replaces a component-local `retained_mode`
    // signal that lagged and showed Metadata even after the user had switched
    // to Settings (the "hardlocked to session metadata" report, 2026-07-21).
    let restore_mode = snapshot.right_panel_reveal_mode.clone();
    // A contributed pane lives and dies with its declaration: when the app stops
    // declaring (exited, session switched, contribution swept) the pane
    // collapses, and it re-reveals when the app is back.
    // ⛔ BOTH halves, as everywhere else: the pane's OWNER must be the session
    // on screen, and that owner must still declare it. `sidebar_panes` is the
    // ACTIVE session's declaration list, so matching an id against it while
    // ignoring the owner is how another app's rail earned the right to paint
    // over this row (live-caught 2026-08-08).
    let app_pane_available = |mode: &RightPanelMode| match mode {
        RightPanelMode::AppPane(open_pane) => {
            snapshot.active_session_path.as_deref() == Some(open_pane.session.as_str())
                && snapshot
                    .sidebar_panes
                    .iter()
                    .any(|pane| pane.id == open_pane.pane)
        }
        _ => true,
    };
    let docked = requested_mode != RightPanelMode::Hidden && app_pane_available(&requested_mode);
    let rendered_mode = if docked {
        requested_mode.clone()
    } else {
        restore_mode
    };
    RailRenderView {
        requested_mode,
        rendered_mode,
        docked,
    }
}
