#[component]
fn Sidebar(
    snapshot: SharedSnapshot,
    /// The left edge's reveal state machine. Live only while the sidebar is
    /// hidden — a hidden sidebar IS an auto-hide sidebar.
    autohide: AutoHideSignals,
    /// Is the hidden sidebar currently revealed as an overlay? Computed by the
    /// shell so the web-surface cover/clamp sees the same answer the DOM does.
    autohide_revealed: bool,
    /// Is that reveal PINNED by a gesture (`sidebar_autohide_pinned`) rather
    /// than by the pointer alone? Stamped into the DOM so the page-placement
    /// rule can tell a standing claim from a transient one.
    autohide_pinned: bool,
    rename_depth: Option<usize>,
    on_prev_search_row: EventHandler<()>,
    on_next_search_row: EventHandler<()>,
    on_select_all_rows: EventHandler<()>,
    /// Arrow-key row navigation (spec §8): `(delta, to_edge)` — delta -1/+1 for
    /// up/down, `to_edge` for Home/End.
    on_navigate_rows: EventHandler<(i32, bool)>,
    on_start_sidebar_resize: EventHandler<f64>,
    on_select_row: EventHandler<(BrowserRow, TreeSelectionMode)>,
    on_press_highlight_row: EventHandler<(BrowserRow, TreeSelectionMode)>,
    /// Focus a pane (by member session path) from a compound split row's cell
    /// click ([[campaign-split-view-groups]]).
    on_focus_split_pane: EventHandler<String>,
    on_set_row_expanded: EventHandler<(BrowserRow, bool)>,
    on_delete_selected_items: EventHandler<bool>,
    on_delete_row: EventHandler<BrowserRow>,
    on_open_context_menu: EventHandler<(BrowserRow, (f64, f64))>,
    on_start_drag: EventHandler<(BrowserRow, (f64, f64))>,
    on_drag_hover: EventHandler<(BrowserRow, (f64, f64), DragDropPlacement)>,
    on_drag_move: EventHandler<(f64, f64)>,
    on_drag_leave: EventHandler<BrowserRow>,
    on_drop_into_row: EventHandler<()>,
    on_end_drag: EventHandler<()>,
    on_begin_rename: EventHandler<BrowserRow>,
    on_regenerate_row_title: EventHandler<BrowserRow>,
    on_update_rename: EventHandler<String>,
    on_focus_rename: EventHandler<()>,
    on_commit_rename: EventHandler<BrowserRow>,
    on_cancel_rename: EventHandler<()>,
) -> Element {
    let _render_span = crate::render_attribution::ComponentRenderSpan::start("Sidebar");
    // A hidden sidebar leaves the FLOW entirely (see `sidebar_panel_outer_style`):
    // the viewport keeps its full width and the reveal happens on the z axis, so
    // hovering the edge never re-fits the xterm and never touches the daemon's
    // PTY grid.
    let auto_hide = !snapshot.sidebar_open;
    let mode = if snapshot.sidebar_open {
        SidebarPanelMode::InFlow
    } else if autohide_revealed {
        SidebarPanelMode::Revealed
    } else {
        SidebarPanelMode::Collapsed
    };
    let drag_active = !snapshot.drag_paths.is_empty();
    // PERF: rows are wrapped in `Rc` so the sidebar render loop below can hand a
    // row to ~13 per-row event-handler closures with cheap refcount bumps instead
    // of deep-cloning the full `BrowserRow` (≈7 String allocs each) 13× per row,
    // every render, across ~223 rows. A real `BrowserRow` clone now only happens
    // when an event actually fires (and once for the SidebarRow prop). See
    // [[finding-gui-latency-render-path-campaign]].
    let visible_rows = snapshot
        .rows
        .iter()
        .cloned()
        .map(Rc::new)
        .scan(false, |in_live_group, row| {
            if row.depth == 0 {
                *in_live_group = row.full_path == "__live_sessions__";
            }
            let live_group_member = *in_live_group && row.depth > 0;
            Some((row, live_group_member))
        })
        .collect::<Vec<_>>();
    let live_group_paths = visible_rows
        .iter()
        .filter_map(|(row, live_group_member)| {
            (*live_group_member && row.kind == BrowserRowKind::Session)
                .then(|| row.full_path.clone())
        })
        .collect::<HashSet<_>>();
    // ONE fixed-key geometry for every state (docked / collapsed sensor /
    // revealed floating card). Fixed keys are load-bearing — see
    // `SidebarPanelMode`: diverging keys left overlay props lingering when the
    // panel toggled back to docked.
    // ASK the orientation, never assume the tree is on the left: the mirror
    // toggle moves this whole panel — its card, its shadow, its collapse slide
    // and its resize grip — to the other edge in one answer.
    let tree_edge = chrome_slot_edge(&snapshot, ChromeSlot::Tree);
    let outer_style = sidebar_panel_outer_style(
        tree_edge,
        mode,
        snapshot.sidebar_width,
        zoom_percent_f32(snapshot.settings.ui_font_size, 14.0),
    );
    let content_style =
        sidebar_panel_card_style(tree_edge, mode, snapshot.sidebar_width, snapshot.palette);
    rsx! {
        div {
            id: "yggterm-sidebar",
            "data-sidebar": "1",
            "data-sidebar-open": if snapshot.sidebar_open { "true" } else { "false" },
            "data-sidebar-auto-hide": if auto_hide { "true" } else { "false" },
            "data-sidebar-autohide-revealed": if auto_hide && autohide_revealed { "true" } else { "false" },
            // Reveal held by a GESTURE (row context menu, rename field, drag,
            // resize drag, KeyTips) rather than by hover — the page beside a
            // pinned reveal may RESIZE, a hover only TRANSLATES it
            // (`web_surface_place_page_rect`).
            "data-sidebar-autohide-pin": if auto_hide && autohide_pinned { "1" } else { "0" },
            // A revealed overlay sidebar floats over the page hole, so it
            // declares itself a cover — under glass the shell's input region
            // gets its rect back, and the legacy native-webview path clamps the
            // page beside it (`WEB_SURFACE_GEOMETRY_EVAL_JS`). Collapsed, the
            // 6px sensor stays UNdeclared: the page owns its own left edge.
            // Named by PANEL, not by side. The reconciler consumes the RECT, so
            // this string is only ever read by a human — and after a mirror
            // "sidebar-left" would be that human's first wrong turn.
            "data-covers-web-surface": if auto_hide && autohide_revealed { "sidebar-tree" },
            "data-sidebar-width": "{snapshot.sidebar_width.round() as i64}",
            style: outer_style,
            tabindex: "0",
            onmousedown: |evt| evt.stop_propagation(),
            onclick: |evt| evt.stop_propagation(),
            onmouseenter: move |_| {
                if auto_hide {
                    autohide.reveal();
                }
            },
            // `mouseenter` fires ONCE, at the point of entry. If the collapsing
            // panel shrinks out from under a resting pointer, no further enter
            // ever arrives — so a move inside the sensor re-reveals. Early-returns
            // once revealed, so a revealed panel costs no signal writes.
            onmousemove: move |_| {
                if auto_hide {
                    autohide.reveal_if_idle();
                }
            },
            onmouseleave: move |_| {
                if auto_hide {
                    autohide.handle_mouse_leave();
                }
            },
            onfocusin: move |_| {
                if auto_hide {
                    autohide.set_focus_within(true);
                }
            },
            onfocusout: move |_| {
                if auto_hide {
                    autohide.set_focus_within(false);
                }
            },
            onmounted: move |_| {
                let _ = document::eval(&format!(
                    r#"
                    (() => {{
                      const sidebar = document.getElementById('yggterm-sidebar');
                      if (!sidebar || sidebar.dataset.keyboardOwnerInstalled === 'true') {{
                        return;
                      }}
                      const claimOwner = (target) => {{
                        if (
                          target
                          && target.closest
                          && target.closest('[data-tree-rename-input="1"], input, textarea, [contenteditable="true"]')
                        ) {{
                          return;
                        }}
                        const row = target && target.closest ? target.closest('[data-sidebar-row-path]') : null;
                        const rowPath = row
                          ? String(row.getAttribute('data-sidebar-row-path') || '')
                          : '';
                        try {{
                          window.__yggtermSidebarKeyboardOwner = true;
                          window.__yggtermFocusedSidebarRowPath = rowPath;
                          window.__yggtermUiFocusClaimUntilMs = Math.max(
                            Number(window.__yggtermUiFocusClaimUntilMs || 0),
                            Date.now() + 1400
                          );
                        }} catch (_error) {{}}
                        try {{
                          const active = document.activeElement;
                          if (
                            active
                            && active.classList
                            && active.classList.contains('xterm-helper-textarea')
                            && typeof active.blur === 'function'
                          ) {{
                            active.blur();
                          }}
                        }} catch (_error) {{}}
                        const focusTarget = row || sidebar;
                        if (focusTarget && typeof focusTarget.focus === 'function') {{
                          try {{
                            focusTarget.focus({{ preventScroll: true }});
                          }} catch (_error) {{
                            try {{
                              focusTarget.focus();
                            }} catch (_error2) {{}}
                          }}
                        }}
                      }};
                      sidebar.addEventListener('mousedown', (event) => {{
                        claimOwner(event.target);
                      }}, true);
                      sidebar.addEventListener('contextmenu', (event) => {{
                        claimOwner(event.target);
                      }}, true);
                      if (!window.__yggtermSidebarDeleteListenerInstalled) {{
                        window.addEventListener('keydown', (event) => {{
                          if (!window.__yggtermSidebarKeyboardOwner) {{
                            return;
                          }}
                          if (String(event.key || '') !== 'Delete') {{
                            return;
                          }}
                          const target = event.target;
                          const active = document.activeElement;
                          const terminalOwnsKey = (node) => Boolean(
                            node
                            && (
                              (node.classList && node.classList.contains('xterm-helper-textarea'))
                              || (node.closest && node.closest('[id^="yggterm-terminal-"]'))
                            )
                          );
                          const editableOwnsKey = (node) => Boolean(
                            node
                            && (
                              node.isContentEditable
                              || ['input', 'textarea', 'select'].includes(String(node.tagName || '').toLowerCase())
                            )
                          );
                          if (
                            terminalOwnsKey(active)
                            || terminalOwnsKey(target)
                            || editableOwnsKey(active)
                            || editableOwnsKey(target)
                          ) {{
                            window.__yggtermSidebarFocusGeneration = Number(window.__yggtermSidebarFocusGeneration || 0) + 1;
                            window.__yggtermSidebarKeyboardOwner = false;
                            return;
                          }}
                          const buttonId = event.shiftKey
                            ? {TREE_HARD_DELETE_BUTTON_ID:?}
                            : {TREE_DELETE_BUTTON_ID:?};
                          const button = document.getElementById(buttonId);
                          if (!button) {{
                            return;
                          }}
                          event.preventDefault();
                          event.stopPropagation();
                          event.stopImmediatePropagation?.();
                          button.click();
                        }}, true);
                        window.__yggtermSidebarDeleteListenerInstalled = true;
                      }}
                      sidebar.dataset.keyboardOwnerInstalled = 'true';
                    }})();
                    "#,
                ));
            },
            onkeydown: move |evt| {
                let is_accel = evt.modifiers().contains(Modifiers::CONTROL)
                    || evt.modifiers().contains(Modifiers::META);
                if is_accel
                    && matches!(evt.key(), Key::Character(ref key) if key.eq_ignore_ascii_case("a"))
                {
                    evt.prevent_default();
                    on_select_all_rows.call(());
                    return;
                }
                if evt.key() == Key::Delete {
                    evt.prevent_default();
                    on_delete_selected_items.call(evt.modifiers().contains(Modifiers::SHIFT));
                    return;
                }
                // Arrow-key row navigation (§8). The selection follows the cursor,
                // so the focus ring and "here" both track it.
                match evt.key() {
                    Key::ArrowDown => {
                        evt.prevent_default();
                        on_navigate_rows.call((1, false));
                    }
                    Key::ArrowUp => {
                        evt.prevent_default();
                        on_navigate_rows.call((-1, false));
                    }
                    Key::Home => {
                        evt.prevent_default();
                        on_navigate_rows.call((-1, true));
                    }
                    Key::End => {
                        evt.prevent_default();
                        on_navigate_rows.call((1, true));
                    }
                    _ => {}
                }
            },
            // The CONTENT layer keeps the sidebar's full width at all times, so
            // the panel slides in as one piece instead of its rows re-wrapping
            // while the outer box animates from a 6px sensor to full width.
            // Same shape as the titlebar's fixed-height inner div.
            div {
            "data-sidebar-content": "1",
            style: content_style,
            if snapshot.search_active {
                div {
                    style: "padding:12px 12px 0 12px; display:flex; align-items:center; gap:8px;",
                    button {
                        title: "Previous matching session",
                        style: chip_style(snapshot.palette, false),
                        onclick: move |_| on_prev_search_row.call(()),
                        "↑"
                    }
                    button {
                        title: "Next matching session",
                        style: chip_style(snapshot.palette, false),
                        onclick: move |_| on_next_search_row.call(()),
                        "↓"
                    }
                    div {
                        style: format!("font-size:11px; color:{}; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", snapshot.palette.muted),
                        if let Some(ix) = snapshot.search_sidebar_match_index {
                            {format!("Sidebar {}/{}", ix + 1, snapshot.search_sidebar_matches.len())}
                        } else {
                            "Sidebar 0/0"
                        }
                    }
                }
            }
            if snapshot.sidebar_loading
                && !snapshot.show_loading_tree
                && snapshot.active_view_mode != WorkspaceViewMode::Terminal
            {
                div {
                    style: "padding:8px 12px 0 12px; display:flex; justify-content:flex-start;",
                    LoadingStateChip {
                        label: "Refreshing tree…".to_string(),
                        palette: snapshot.palette,
                    }
                }
            }
            style { "{TREE_SPINNER_CSS}{STATUS_DOT_BLINK_CSS}{SIDEBAR_LIVE_CLOSE_CSS}" }
            div {
                "data-sidebar-scroll": "1",
                style: "flex:1; min-height:0; overflow:auto; padding:12px 12px 12px 12px;",
                onmousemove: move |evt| {
                    let primary_down = evt.held_buttons().contains(MouseButton::Primary);
                    let coords = evt.client_coordinates();
                    if drag_active || primary_down {
                        on_drag_move.call((coords.x, coords.y));
                    }
                },
                onmouseup: move |_| {
                    if drag_active {
                        on_drop_into_row.call(());
                        on_end_drag.call(());
                    }
                },
                if snapshot.show_loading_tree && snapshot.rows.is_empty() {
                    SidebarLoadingState { palette: snapshot.palette }
                } else {
                    for (row, live_group_member) in visible_rows.into_iter() {
                    {
                        let select_row = row.clone();
                        let press_highlight_row = row.clone();
                        let context_row = row.clone();
                        let delete_row = row.clone();
                        let visible_label = sidebar_row_visible_label(&row);
                        let icon_kind = {
                            let base = tree_icon_kind(&row);
                            if base == "terminal"
                                && snapshot.live_sessions.iter().any(|s| {
                                    s.session_path == row.full_path
                                        && s.kind == SessionKind::ClaudeCode
                                })
                            {
                                "claude-code"
                            } else {
                                base
                            }
                        }
                        .to_string();
                        let busy_icon = sidebar_row_shows_busy_icon(&snapshot, &row);
                        let input_unanswered = sidebar_row_input_unanswered(&snapshot, &row);
                        let row_dragging = sidebar_row_dragging_for_projection(
                            snapshot.drag_paths.as_slice(),
                            &live_group_paths,
                            &row,
                            live_group_member,
                        );
                        let row_selected = sidebar_row_selected_for_projection(
                            snapshot.selected_tree_paths.as_slice(),
                            snapshot.selected_path.as_deref(),
                            &live_group_paths,
                            &row,
                            live_group_member,
                        );
                        let sidebar_row_key = format!(
                            "{}::{}::{}::{}",
                            row.full_path,
                            row.depth,
                            row.session_id.as_deref().unwrap_or(""),
                            if live_group_member { "live" } else { "tree" }
                        );
                        // A compound split row renders its own miniature-map
                        // widget ([[campaign-split-view-groups]]); every other
                        // row uses the normal SidebarRow.
                        let split_group_cells = split_group_cells_for_row(&snapshot, &row);
                        let split_context_row = row.clone();
                        rsx! {
                            if let Some((cells, axis_side_by_side)) = split_group_cells.clone() {
                                SplitGroupRow {
                                    key: "{sidebar_row_key}",
                                    axis_side_by_side,
                                    cells,
                                    palette: snapshot.palette,
                                    accent: snapshot.theme_accent.clone(),
                                    selected: row_selected,
                                    depth: row.depth,
                                    on_focus_pane: move |path: String| on_focus_split_pane.call(path),
                                    on_open_context_menu: move |coords: (f64, f64)| {
                                        on_open_context_menu.call(((*split_context_row).clone(), coords))
                                    },
                                }
                            } else {
                            SidebarRow {
                                key: "{sidebar_row_key}",
                                row: (*row).clone(),
                                visible_label: visible_label.clone(),
                                icon_kind: icon_kind.clone(),
                                busy_icon,
                                input_unanswered,
                                selected: row_selected,
                                drop_target: snapshot
                                    .drag_hover_target
                                    .as_ref()
                                    .filter(|target| target.path == row.full_path)
                                    .map(|target| target.placement),
                                dragging: row_dragging,
                                drag_active: !snapshot.drag_paths.is_empty(),
                                renaming: snapshot.tree_rename_path.as_deref() == Some(row.full_path.as_str())
                                    && rename_depth.is_none_or(|depth| depth == row.depth),
                                rename_focus_pending: snapshot.tree_rename_path.as_deref() == Some(row.full_path.as_str())
                                    && rename_depth.is_none_or(|depth| depth == row.depth)
                                    && !snapshot.tree_rename_input_focused_once,
                                rename_focused_once: snapshot.tree_rename_input_focused_once,
                                rename_value: snapshot.tree_rename_value.clone(),
                                show_live_close: live_group_member && row.kind == BrowserRowKind::Session,
                                palette: snapshot.palette,
                                web_profile: snapshot
                                    .web_surface_profiles
                                    .get(&row.full_path)
                                    .cloned()
                                    .unwrap_or_default(),
                                // One badge each, on the row it means something on:
                                // E on the "here" row (what ALT,E opens), J on the
                                // Live Sessions row (the list ALT,J walks).
                                row_menu_tip: if snapshot.here_row_path.as_deref() == Some(row.full_path.as_str()) {
                                    keytip_tip_attr(&snapshot, "session.menu")
                                } else {
                                    String::new()
                                },
                                jump_tip: if row.full_path == "__live_sessions__" {
                                    keytip_tip_attr(&snapshot, "session.jump")
                                } else {
                                    String::new()
                                },
                                on_select: move |mode: TreeSelectionMode| {
                                    on_select_row.call(((*select_row).clone(), mode));
                                },
                                on_press_highlight: move |mode: TreeSelectionMode| {
                                    on_press_highlight_row.call(((*press_highlight_row).clone(), mode));
                                },
                                on_set_expanded: {
                                    let row = row.clone();
                                    move |expanded: bool| on_set_row_expanded.call(((*row).clone(), expanded))
                                },
                                on_open_context_menu: move |coords: (f64, f64)| on_open_context_menu.call(((*context_row).clone(), coords)),
                                on_delete_row: move |_| on_delete_row.call((*delete_row).clone()),
                                on_begin_rename: {
                                    let row = row.clone();
                                    move |_| on_begin_rename.call((*row).clone())
                                },
                                on_regenerate_title: {
                                    let row = row.clone();
                                    move |_| on_regenerate_row_title.call((*row).clone())
                                },
                                on_update_rename: move |value: String| on_update_rename.call(value),
                                on_focus_rename: move |_| on_focus_rename.call(()),
                                on_commit_rename: {
                                    let row = row.clone();
                                    move |_| on_commit_rename.call((*row).clone())
                                },
                                on_cancel_rename: move |_| on_cancel_rename.call(()),
                                on_start_drag: {
                                    let row = row.clone();
                                    move |evt: MouseEvent| {
                                        let coords = evt.client_coordinates();
                                        on_start_drag.call(((*row).clone(), (coords.x, coords.y)))
                                    }
                                },
                                on_drag_move: {
                                    let row = row.clone();
                                    move |evt: MouseEvent| {
                                        let coords = evt.client_coordinates();
                                        on_start_drag.call(((*row).clone(), (coords.x, coords.y)))
                                    }
                                },
                                on_drag_hover: {
                                    let row = row.clone();
                                    move |(placement, evt): (DragDropPlacement, MouseEvent)| {
                                        let coords = evt.client_coordinates();
                                        on_drag_hover.call(((*row).clone(), (coords.x, coords.y), placement))
                                    }
                                },
                                on_drag_leave: {
                                    let row = row.clone();
                                    move |_| on_drag_leave.call((*row).clone())
                                },
                                on_drop_into_row: move |_| on_drop_into_row.call(()),
                                on_end_drag: move |_| on_end_drag.call(()),
                            }
                            }
                        }
                    }
                }
                }
                div {
                    "data-sidebar-resize-handle": "1",
                    style: sidebar_resize_handle_style(tree_edge),
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        on_start_sidebar_resize.call(evt.client_coordinates().x);
                    },
                    ondoubleclick: |evt| evt.stop_propagation(),
                }
            }
            }
        }
    }
}
fn sidebar_row_dragging_for_projection(
    drag_paths: &[String],
    live_group_paths: &HashSet<String>,
    row: &BrowserRow,
    live_group_member: bool,
) -> bool {
    let path_dragging = drag_paths.iter().any(|path| path == &row.full_path);
    if !path_dragging {
        return false;
    }
    !live_group_paths.contains(&row.full_path) || live_group_member
}

fn sidebar_row_selected_for_projection(
    selected_tree_paths: &[String],
    selected_path: Option<&str>,
    live_group_paths: &HashSet<String>,
    row: &BrowserRow,
    live_group_member: bool,
) -> bool {
    let path_selected = selected_tree_paths
        .iter()
        .any(|path| path == &row.full_path)
        || (selected_tree_paths.is_empty() && selected_path == Some(row.full_path.as_str()));
    if !path_selected {
        return false;
    }
    !live_group_paths.contains(&row.full_path) || live_group_member
}

fn sidebar_row_visible_label(row: &BrowserRow) -> String {
    machine_label_text(&row.label).unwrap_or_else(|| row.label.clone())
}
fn snapshot_terminal_mount_epoch(snapshot: &RenderSnapshot, session_path: &str) -> u64 {
    snapshot
        .terminal_mount_epochs
        .get(session_path)
        .copied()
        .unwrap_or(0)
}

fn sidebar_row_session_for_icon<'a>(
    snapshot: &'a RenderSnapshot,
    row: &BrowserRow,
) -> Option<&'a ManagedSessionView> {
    let row_path = normalize_live_session_path(&row.full_path);
    let row_session_id = row.session_id.as_deref();
    snapshot
        .live_sessions
        .iter()
        .chain(snapshot.retained_terminal_sessions.iter())
        .chain(snapshot.active_session.iter())
        .find(|session| {
            normalize_live_session_path(&session.session_path) == row_path
                || session.session_path == row.full_path
                || row_session_id.is_some_and(|session_id| session.id == session_id)
        })
}
fn session_sample_text_for_sidebar_icon(session: &ManagedSessionView) -> String {
    let terminal_tail = session
        .terminal_lines
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    if !terminal_tail.is_empty() {
        return terminal_tail
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
    }
    let rendered_tail = session
        .rendered_sections
        .iter()
        .rev()
        .flat_map(|section| section.lines.iter().rev())
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .take(6)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !rendered_tail.is_empty() {
        return rendered_tail
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
    }
    session.status_line.clone()
}
fn terminal_chunk_looks_idle_for_sidebar_icon(sample: &str) -> bool {
    // PERF (fan/CPU spin, live stack-sampled 2026-06-10): this runs per sidebar
    // row per RENDER, and its terminal_chunk_* recognizers each re-run
    // strip_terminal_control_sequences over the full sample (up to ~128KB of
    // codex frame) — the GUI main thread was pegged at ~100% inside this exact
    // call chain (sidebar_row_busy_state -> here -> strip). The sample for a
    // row only changes when new output arrives, so memoize the verdict by the
    // sample's hash. Main-thread only (render path) -> thread_local, capped.
    thread_local! {
        static SIDEBAR_IDLE_MEMO: std::cell::RefCell<HashMap<u64, bool>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        sample.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(hit) =
        SIDEBAR_IDLE_MEMO.with(|memo| memo.borrow().get(&key).copied())
    {
        return hit;
    }
    let verdict = terminal_chunk_looks_idle_for_sidebar_icon_uncached(sample);
    SIDEBAR_IDLE_MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.len() >= 512 {
            memo.clear();
        }
        memo.insert(key, verdict);
    });
    verdict
}

fn terminal_chunk_looks_idle_for_sidebar_icon_uncached(sample: &str) -> bool {
    let trimmed = sample.trim();
    if trimmed.is_empty() {
        return false;
    }
    let tail_line = trimmed
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(trimmed);
    let prompt_summary_prefix = tail_line
        .split(" >")
        .next()
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
        .unwrap_or(tail_line);
    terminal_chunk_has_prompt_output(trimmed)
        || terminal_chunk_has_codex_prompt_output(trimmed)
        || terminal_chunk_is_generic_codex_idle(trimmed)
        || terminal_chunk_has_prompt_output(tail_line)
        || terminal_chunk_has_codex_prompt_output(tail_line)
        || (prompt_summary_prefix != tail_line
            && (terminal_chunk_has_prompt_output(prompt_summary_prefix)
                || terminal_chunk_has_codex_prompt_output(prompt_summary_prefix)))
}
/// Detects an agent CLI actively processing a turn, for the sidebar working
/// indicator. CLI-agnostic: Codex renders `Working (Ns • esc to interrupt)`,
/// Claude Code renders `✻ <gerund>… (Ns · esc to interrupt)` — the shared,
/// unambiguous "I'm busy, press esc to stop" signal is `esc to interrupt`,
/// which neither CLI shows when idle. Codex-only background-task indicators
/// (`/stop to close`, `background terminal running`) are kept as a fallback.
fn terminal_chunk_has_agent_working_status_for_sidebar_icon(sample: &str) -> bool {
    // SSOT: the detection heuristic lives in yggterm-core so the sidebar
    // working-indicator and the daemon hot-update idle gate share one
    // definition of "agent is working" and cannot diverge.
    // PERF: memoized for the same reason as the idle recognizer above — this
    // runs per sidebar row per render over up-to-128KB samples.
    thread_local! {
        static SIDEBAR_WORKING_MEMO: std::cell::RefCell<HashMap<u64, bool>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        sample.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(hit) = SIDEBAR_WORKING_MEMO.with(|memo| memo.borrow().get(&key).copied()) {
        return hit;
    }
    let verdict = yggterm_core::screen_text_shows_agent_working(sample);
    SIDEBAR_WORKING_MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.len() >= 512 {
            memo.clear();
        }
        memo.insert(key, verdict);
    });
    verdict
}
fn terminal_lines_are_bootstrap_scaffold(lines: &[String]) -> bool {
    let visible = lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !visible.is_empty()
        && visible.iter().all(|line| {
            line.starts_with("$ exec ")
                || line.starts_with("Queue live shell session ")
                || line.starts_with("Launching live ")
                || line.starts_with("Target: ")
                || line.starts_with("Workspace: ")
                || line.starts_with("Command: ")
                || line.starts_with("Daemon runtime: ")
                || line.starts_with("Daemon PTY: ")
                || line.starts_with("Deploy state: ")
                || line.starts_with("Launch phase: ")
                || line == &"Terminal surface: embedded xterm.js"
        })
}
#[cfg(test)]
fn session_is_idle_for_sidebar_icon(session: &ManagedSessionView) -> bool {
    let sample = session_sample_text_for_sidebar_icon(session);
    terminal_chunk_looks_idle_for_sidebar_icon(&sample)
}
fn sidebar_row_has_optimistic_busy_hint(snapshot: &RenderSnapshot, row: &BrowserRow) -> bool {
    let row_path = normalize_live_session_path(&row.full_path);
    snapshot.optimistic_busy_paths.iter().any(|session_path| {
        normalize_live_session_path(session_path) == row_path || session_path == &row.full_path
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SidebarBusyState {
    visible: bool,
    reason: &'static str,
}
impl SidebarBusyState {
    const fn idle() -> Self {
        Self {
            visible: false,
            reason: "idle",
        }
    }
    const fn busy(reason: &'static str) -> Self {
        Self {
            visible: true,
            reason,
        }
    }
}
fn terminal_input_busy_hint_decision(data: &str, mut pending_line_has_text: bool) -> (bool, bool) {
    let mut should_hint = false;
    for ch in data.chars() {
        match ch {
            '\r' | '\n' => {
                should_hint |= pending_line_has_text;
                pending_line_has_text = false;
            }
            '\u{3}' | '\u{15}' => {
                pending_line_has_text = false;
            }
            '\u{8}' | '\u{7f}' => {}
            ch if !ch.is_control() && !ch.is_whitespace() => {
                pending_line_has_text = true;
            }
            _ => {}
        }
    }
    (should_hint, pending_line_has_text)
}
fn terminal_input_should_show_stateless_busy_hint(data: &str) -> bool {
    terminal_input_busy_hint_decision(data, false).0
}
fn terminal_probe_input_should_show_stateless_busy_hint(
    data: &str,
    press_enter: bool,
    press_tab: bool,
    press_ctrl_c: bool,
    press_ctrl_e: bool,
    press_ctrl_u: bool,
) -> bool {
    let mut input = String::new();
    if press_ctrl_c {
        input.push('\u{3}');
    }
    if press_ctrl_e {
        input.push('\u{5}');
    }
    if press_ctrl_u {
        input.push('\u{15}');
    }
    input.push_str(data);
    if press_tab {
        input.push('\t');
    }
    if press_enter {
        input.push('\r');
    }
    terminal_input_should_show_stateless_busy_hint(&input)
}
fn codex_completion_notification_should_fire(
    enabled: bool,
    busy_since_input: bool,
    busy_duration_ms: Option<u64>,
    already_notified: bool,
    runtime_running: bool,
    saw_generic_idle_output: bool,
    tail_generic_idle_output: bool,
    saw_generic_idle_footer_output: bool,
    tail_generic_idle_footer_output: bool,
    saw_prompt_output: bool,
    tail_prompt_only_output: bool,
) -> bool {
    enabled
        && busy_since_input
        && busy_duration_ms
            .is_some_and(|duration_ms| duration_ms >= CODEX_COMPLETION_NOTIFICATION_MIN_BUSY_MS)
        && !already_notified
        && !runtime_running
        && (saw_generic_idle_output
            || tail_generic_idle_output
            || saw_generic_idle_footer_output
            || tail_generic_idle_footer_output
            || saw_prompt_output
            || tail_prompt_only_output)
}
fn terminal_input_uses_optimistic_busy_hint(session_path: &str) -> bool {
    let session_path = normalize_live_session_path(session_path);
    session_path.starts_with("local://")
        || session_path.starts_with("ssh://")
        || session_path.starts_with("remote-session://")
        || session_path.starts_with("codex://")
        || session_path.starts_with("codex-litellm://")
}
fn sidebar_row_shows_busy_icon(snapshot: &RenderSnapshot, row: &BrowserRow) -> bool {
    sidebar_row_busy_state(snapshot, row).visible
}
/// Has this row been written to with nothing said back for long enough to be
/// worth a check?
///
/// ⛔ **Deliberately NOT part of [`sidebar_row_busy_state`].** Busy asks "is work
/// happening here"; this asks "is this row still listening". A deaf row answers
/// `false` to the first and `true` to the second, and folding them together
/// would render it as working — swapping one wrong answer for another.
///
/// ⚠ A TRIGGER, NEVER A VERDICT: echo-off and password prompts legitimately
/// consume input in silence. It marks the row as worth checking; `server app
/// terminal input-check` settles it by marker and echo.
fn sidebar_row_input_unanswered(snapshot: &RenderSnapshot, row: &BrowserRow) -> bool {
    if row.kind == BrowserRowKind::Group {
        // A group is COLLAPSED far more often than not, and a deaf row hidden
        // inside one is invisible for exactly as long as it stays folded —
        // which is the failure this whole signal exists to end, merely moved up
        // one level. The busy state has aggregated over descendants since issue
        // #3; attention has to, for the same reason and by the same walk.
        return sidebar_group_has_input_unanswered_descendant(snapshot, row)
            || sidebar_machine_row_has_input_unanswered_live_session(snapshot, row);
    }
    if row.kind != BrowserRowKind::Session {
        return false;
    }
    yggterm_core::input_unanswered_suggests_wedge(
        sidebar_row_session_for_icon(snapshot, row).and_then(|session| session.input_unanswered_ms),
    )
}

/// Any session in this group's subtree that has stopped answering — the
/// attention twin of [`sidebar_group_has_working_descendant`], walking the same
/// flat depth-ordered descendant span so the two can never disagree about what
/// "inside this group" means.
fn sidebar_group_has_input_unanswered_descendant(
    snapshot: &RenderSnapshot,
    group_row: &BrowserRow,
) -> bool {
    let Some(start) = snapshot.rows.iter().position(|candidate| {
        candidate.kind == BrowserRowKind::Group && candidate.full_path == group_row.full_path
    }) else {
        return false;
    };
    let group_depth = group_row.depth;
    snapshot.rows[start + 1..]
        .iter()
        .take_while(|descendant| descendant.depth > group_depth)
        .filter(|descendant| descendant.kind == BrowserRowKind::Session)
        .any(|descendant| sidebar_row_input_unanswered(snapshot, descendant))
}

/// A machine ROOT row whose machine hosts a live session that has stopped
/// answering. Live sessions hang off the flat "Live Sessions" group rather than
/// the machine's cwd subtree, so the descendant walk above cannot see them —
/// matched by host instead, exactly as the working twin does.
fn sidebar_machine_row_has_input_unanswered_live_session(
    snapshot: &RenderSnapshot,
    row: &BrowserRow,
) -> bool {
    let Some(machine_key) = row.full_path.strip_prefix("__remote_machine__/") else {
        return false;
    };
    snapshot.live_sessions.iter().any(|session| {
        yggterm_core::input_unanswered_suggests_wedge(session.input_unanswered_ms)
            && (session.ssh_target.as_deref() == Some(machine_key)
                || session.host_label == machine_key)
    })
}
/// Aggregate working state for a machine/cwd group row (issue #3): a group
/// blinks when ANY session in its subtree is working, so a COLLAPSED machine or
/// cwd still surfaces "work happening inside". Walks the flat, depth-ordered row
/// list for the group's contiguous descendants (depth > the group's depth) and
/// reuses the per-session busy SSOT. Cheap (one linear pass over the subtree);
/// no recursion (only Session descendants are queried, which never re-enter the
/// group branch). See DESIGN.md "Status indicator vocabulary".
/// A machine ROOT row (`__remote_machine__/<key>` or the LOCAL root `"local"`)
/// blinks when any LIVE session belonging to that machine is working (issue #3).
/// Live sessions hang off the flat "Live Sessions" group, NOT under the machine's
/// cwd-tree, so the descendant walk can't see them — match by host instead.
/// Restricted to the machine-root path so cwd SUBgroups under the machine never
/// inherit the whole machine's activity. Snapshot-only (no `ShellState`) so it
/// composes with `sidebar_row_busy_state`.
///
/// The LOCAL root has no `__remote_machine__/` prefix and its live shells live in
/// "Live Sessions" (not the local cwd subtree), so without this branch a working
/// local SHELL never surfaces on the local root — a divergence from remote
/// machines, which blink for any working session via host-match. Local sessions
/// are identified by locality (no ssh target / `local://…` family). Per the
/// local-machine indicator decision this is blink-only: the local machine is
/// always reachable, so it gets no persistent health dot, only this working blink.
fn sidebar_machine_row_has_working_live_session(
    snapshot: &RenderSnapshot,
    row: &BrowserRow,
) -> bool {
    if row.full_path == "local" {
        // Locality has TWO encodings on a live session and both must be
        // honored: `ssh_target` is None for a plain local shell, but a local
        // AGENT session (Codex/CC) persists with the canonical loopback target
        // "localhost" so restore works (see persist_live_sessions). The old
        // `is_none()` check silently excluded every local agent session, so a
        // COLLAPSED local root never blinked while one was working (2026-07-10
        // report) — expanded rows were fine because the session row itself
        // blinks. Reuse the server-side loopback SSOT instead of a second
        // string compare.
        return snapshot.live_sessions.iter().any(|session| {
            session.working == Some(true)
                && session
                    .ssh_target
                    .as_deref()
                    .is_none_or(yggterm_server::is_loopback_ssh_target)
                && is_local_live_session_path(&session.session_path)
        });
    }
    let Some(machine_key) = row.full_path.strip_prefix("__remote_machine__/") else {
        return false;
    };
    snapshot.live_sessions.iter().any(|session| {
        session.working == Some(true)
            && (session.ssh_target.as_deref() == Some(machine_key)
                || session.host_label == machine_key)
    })
}

fn sidebar_group_has_working_descendant(
    snapshot: &RenderSnapshot,
    group_row: &BrowserRow,
) -> bool {
    let Some(start) = snapshot.rows.iter().position(|candidate| {
        candidate.kind == BrowserRowKind::Group && candidate.full_path == group_row.full_path
    }) else {
        return false;
    };
    let group_depth = group_row.depth;
    snapshot.rows[start + 1..]
        .iter()
        .take_while(|descendant| descendant.depth > group_depth)
        .filter(|descendant| descendant.kind == BrowserRowKind::Session)
        .any(|descendant| sidebar_row_busy_state(snapshot, descendant).visible)
}

fn sidebar_row_busy_state(snapshot: &RenderSnapshot, row: &BrowserRow) -> SidebarBusyState {
    if row.kind == BrowserRowKind::Separator {
        return SidebarBusyState::idle();
    }
    if row.kind == BrowserRowKind::Group {
        // Issue #3: a group blinks when work is happening inside it — either a
        // session in its cwd subtree (Live Sessions / local folders) OR, for a
        // machine root row, any live session hosted on that machine.
        return if sidebar_group_has_working_descendant(snapshot, row)
            || sidebar_machine_row_has_working_live_session(snapshot, row)
        {
            SidebarBusyState::busy("group_descendant_working")
        } else {
            SidebarBusyState::idle()
        };
    }
    let Some(session) = sidebar_row_session_for_icon(snapshot, row) else {
        return SidebarBusyState::idle();
    };
    if session.kind == SessionKind::Document {
        return SidebarBusyState::idle();
    }
    // A session whose viewport is a WEB SURFACE: the foreground-process signal
    // is useless there — the browser app is "running" the whole time it is
    // open, which kept the row blinking from launch to quit. The row's light is
    // the PAGE's: blink while the active tab loads, steady once it is loaded.
    if let Some(loading) = snapshot
        .web_surface_loading
        .get(session.session_path.as_str())
    {
        return if *loading {
            SidebarBusyState::busy("web_surface_loading")
        } else {
            SidebarBusyState::idle()
        };
    }
    if sidebar_row_has_optimistic_busy_hint(snapshot, row) {
        return SidebarBusyState::busy("optimistic_terminal_input");
    }
    let sidebar_sample = session_sample_text_for_sidebar_icon(session);
    // ⛔ EVERY AGENT CLI, DERIVED FROM THE REGISTRY. This was a hand-written
    // `Codex | CodexLiteLlm | ClaudeCode`, and the seven CLIs registered after
    // it fell straight past into the screen-text heuristics below — so an
    // Antigravity row sat still while its own metadata rail read
    // `running · working` and its CLI printed `esc to cancel`. The daemon has
    // computed `working` from THIS descriptor's phrases for every agent kind
    // since the arm above it was derived (`session.kind.is_agent()` in
    // `overlay_terminal_runtime_snapshot_session`); only the reader was still
    // a list. Owner-reported 2026-08-21.
    if session.kind.is_agent() {
        // Agent CLI sessions: working has exactly ONE source of truth — the
        // DAEMON-authoritative `working` flag, computed from the session's LIVE
        // vt100 screen at snapshot time (esc-to-interrupt SSOT, shared codex/CC
        // shape). We blink ONLY on `Some(true)`. `Some(false)` (confirmed idle)
        // and `None` (the daemon holds no live screen — preserved/foreign-owned,
        // or an older daemon that doesn't report it) BOTH resolve to idle, so a
        // session can never get stuck blinking on a frozen last frame after its
        // turn ended — the previous code re-scraped the GUI's last-captured
        // screen tail here, which froze on the working footer for non-owned
        // sessions (the "blinking long after done" bug). The optimistic input
        // hint above stays: it is direct user intent, capped at
        // TERMINAL_BUSY_HINT_MS.
        // ⛔ BEFORE the working arm, not after: a row holding an owner question
        // is mid-turn, so `working` is TRUE and the dot would report ordinary
        // work on a session that is stopped and waiting for the human. The dot
        // stays lit — something IS pending — but with its own reason, so every
        // reader of this state can tell "the machine is busy" from "the machine
        // is waiting on you and eating what you type".
        if session.awaiting_user_choice {
            return SidebarBusyState::busy("awaiting_user_choice");
        }
        if session.working == Some(true) {
            return SidebarBusyState::busy("agent_working_daemon");
        }
        // A usage-limit wait is not idle: the CLI's auto-continue is armed and
        // the turn resumes when the window opens. The screen carries no
        // working phrase during the wait, so without this arm the dot went
        // dark at the exact moment the owner most wanted to see the row was
        // still in flight (queue: the limit-wait tri-state entry).
        if session.limit_wait {
            return SidebarBusyState::busy("limit_wait");
        }
        return SidebarBusyState::idle();
    }
    // Issue #1: a plain shell's working state has the SAME daemon-authoritative
    // SSOT as agents — the daemon sets `working` from the OS foreground-process
    // signal (a command actually running in the tty), which is correct even when
    // the prompt text "looks idle" (`pi@host:~$ sleep 90`). Trust it when the
    // daemon reports it (`Some` = owned); only fall back to the screen/foreground
    // heuristic below when it is `None` (not owned / an older daemon that does
    // not compute it). This stops the screen-text "looks idle" early-return from
    // preempting a genuinely-running command.
    if session.kind == SessionKind::Shell {
        match session.working {
            Some(true) => return SidebarBusyState::busy("shell_working_daemon"),
            Some(false) => return SidebarBusyState::idle(),
            None => {}
        }
    }
    let has_terminal_line_sample = session
        .terminal_lines
        .iter()
        .any(|line| !line.trim().is_empty())
        && !terminal_lines_are_bootstrap_scaffold(&session.terminal_lines);
    let is_idle = terminal_chunk_looks_idle_for_sidebar_icon(&sidebar_sample);
    let is_active_live_session = snapshot.active_session_path.as_deref()
        == Some(session.session_path.as_str())
        && matches!(
            session.source,
            SessionSource::LiveLocal | SessionSource::LiveSsh
        );
    if matches!(
        session.launch_phase,
        yggterm_server::TerminalLaunchPhase::Queued
            | yggterm_server::TerminalLaunchPhase::BridgePending
            | yggterm_server::TerminalLaunchPhase::RemoteBootstrap
    ) {
        if session.terminal_foreground_active == Some(true) {
            return SidebarBusyState::busy("daemon_foreground_active");
        }
        if has_terminal_line_sample && !is_idle && is_active_live_session {
            return SidebarBusyState::busy("active_bootstrap_terminal_output");
        }
        return SidebarBusyState::idle();
    }
    if is_active_live_session && is_idle {
        return SidebarBusyState::idle();
    }
    if session.terminal_foreground_active == Some(true) {
        return SidebarBusyState::busy("daemon_foreground_active");
    }
    if is_active_live_session
        && snapshot
            .active_summary
            .as_deref()
            .is_some_and(terminal_chunk_looks_idle_for_sidebar_icon)
    {
        return SidebarBusyState::idle();
    }
    if let Some(foreground_active) = session.terminal_foreground_active {
        if foreground_active {
            return SidebarBusyState::busy("daemon_foreground_active");
        }
        return SidebarBusyState::idle();
    }
    SidebarBusyState::idle()
}
fn tree_icon_kind(row: &BrowserRow) -> &'static str {
    match row.kind {
        BrowserRowKind::Separator => "separator",
        BrowserRowKind::Document => {
            if row.document_kind == Some(WorkspaceDocumentKind::TerminalRecipe) {
                "recipe"
            } else {
                "paper"
            }
        }
        BrowserRowKind::Session => {
            // SSOT for icon dispatch: when `session_kind` is set (rows built
            // from a ManagedSessionView), drive the icon from it. Path-prefix
            // branches below are fallback for synthesized rows.
            // See [[spec-unify-local-remote]].
            if let Some(kind) = row.session_kind {
                // ⚠ The agent arm is DERIVED. It used to be a hand-written
                // `Codex | CodexLiteLlm => "session"`, `ClaudeCode =>
                // "claude-code"` pair, and a seventh CLI would have fallen into
                // whichever arm someone remembered — silently wearing another
                // CLI's mark in the sidebar, the row JSON and every smoke test
                // that asserts on `icon_kind`.
                //
                // The two shipped strings are HISTORICAL and stay: `"session"`
                // for the codex family (it predates there being a second CLI)
                // and `"claude-code"`. Every new CLI reports its `slug`.
                // ⛔ The mapping itself lives in the registry beside the slugs it
                //    departs from — `row_icon_kind`. It was restated here, and the
                //    fleet's Python restated it a THIRD time as plain slug
                //    equality, which is how the codex family stopped being
                //    recognisable by its own mark.
                if let Some(icon) = yggterm_core::agent_cli::row_icon_kind(kind) {
                    return icon;
                }
                return match kind {
                    SessionKind::Shell | SessionKind::SshShell => "terminal",
                    SessionKind::Document => "paper",
                    // Unreachable: every non-agent kind is named above and every
                    // agent kind took the descriptor branch. A `match` that
                    // cannot see the future is exactly what this arm replaces.
                    _ => "terminal",
                };
            }
            if row.full_path.starts_with("codex-litellm://")
                || is_codex_litellm_storage_session_path(&row.full_path)
                || row.full_path.starts_with("codex://")
                || is_codex_storage_session_path(&row.full_path)
            {
                "session"
            } else if is_claude_code_session_path(&row.full_path) {
                "claude-code"
            } else if is_antigravity_session_path(&row.full_path) {
                "antigravity"
            } else if row.full_path.starts_with("remote-cc://") {
                "claude-code"
            } else if row.full_path.starts_with("remote-muse://") {
                "muse"
            } else if row.full_path.starts_with("remote-agy://") {
                "antigravity"
            // ⛔ NO hand arms for grok/kimi/opencode/qwen/pi: the drifted
            //    `remote-grok => "grok"` arm (registry says "grok-build") is
            //    exactly how a restated table lies — the registry fallback
            //    below answers from the ONE owner of the answer.
            } else if row.full_path.starts_with("local://") {
                "terminal"
            } else if row.full_path.starts_with("ssh://") {
                "terminal"
            } else if row.full_path.starts_with("remote-session://") {
                "session"
            } else {
                // Registry fallback BEFORE the terminal default: a scheme the
                // ladder above has no arm for (remote-opencode://, the newer
                // remote-qwen/remote-pi shapes, …) is still a REGISTERED agent
                // scheme, and a hand ladder that predates a CLI must not
                // re-clothe it as a shell. Measured 2026-09-01: opencode rows
                // wore the terminal icon because only their kind was missing.
                match yggterm_core::agent_scheme::session_kind_for_path(&row.full_path)
                    .and_then(yggterm_core::agent_cli::row_icon_kind)
                {
                    Some(icon) => icon,
                    None => "terminal",
                }
            }
        }
        BrowserRowKind::Group => {
            if row.full_path == "__live_sessions__" {
                "live-group"
            } else if row.full_path == "local" {
                "local-group"
            } else if row.full_path.starts_with("__remote_machine__/") {
                "remote-machine"
            } else {
                "folder"
            }
        }
    }
}
fn tree_icon_glyph(row: &BrowserRow) -> Option<&'static str> {
    match row.kind {
        BrowserRowKind::Separator => Some("—"),
        BrowserRowKind::Session => {
            // SSOT for glyph: consult session_kind first. See [[spec-unify-local-remote]].
            //
            // ⚠ An agent CLI's mark is DESCRIPTOR DATA (`icon_glyph`) — the CLI
            // declares its own identity once, beside its binary name and its
            // store. It used to be a `match` here, a second `match` in
            // `tree_icon_kind`, and a bespoke component reached by a THIRD
            // string comparison at the call site; three answers to one question,
            // kept in agreement only by two tests.
            if let Some(kind) = row.session_kind {
                if let Some(descriptor) = yggterm_core::agent_cli::agent_cli_descriptor(kind) {
                    return Some(descriptor.icon_glyph);
                }
                return Some(match kind {
                    SessionKind::Shell | SessionKind::SshShell => "$_",
                    SessionKind::Document => "$_",
                    _ => "$_",
                });
            }
            if is_claude_code_session_path(&row.full_path) {
                Some("*_")
            } else if row.full_path.starts_with("remote-cc://") {
                Some("*_")
            } else if row.full_path.starts_with("remote-muse://") {
                Some("M_")
            } else if row.full_path.starts_with("remote-agy://") {
                Some("A_")
            } else if row.full_path.starts_with("codex-litellm://")
                || is_codex_litellm_storage_session_path(&row.full_path)
                || row.full_path.starts_with("codex://")
                || is_codex_storage_session_path(&row.full_path)
                || row.full_path.starts_with("remote-session://")
            {
                Some(">_")
            } else {
                // Registry fallback before the shell default — the same law
                // as tree_icon_kind's ladder: a hand ladder that predates a
                // CLI must not answer for it.
                match yggterm_core::agent_scheme::session_kind_for_path(&row.full_path).and_then(
                    |kind| yggterm_core::agent_cli::agent_cli_descriptor(kind),
                ) {
                    Some(descriptor) => Some(descriptor.icon_glyph),
                    None => Some("$_"),
                }
            }
        }
        _ => None,
    }
}
#[component]
fn SidebarLoadingState(palette: Palette) -> Element {
    rsx! {
        style {
            "{TREE_LOADING_DOT_CSS}"
        }
        div {
            style: "display:flex; align-items:center; gap:10px; padding:12px 10px; border-radius:14px; background:rgba(95,168,255,0.08);",
            div {
                style: format!("font-size:12px; font-weight:800; color:{};", palette.accent),
                "Loading"
            }
            div {
                style: "display:flex; align-items:center; gap:4px;",
                for ix in 0..3 {
                    span {
                        class: "yggterm-loading-dot",
                        style: format!(
                            "width:6px; height:6px; border-radius:999px; background:{}; display:inline-block; \
                             animation:yggterm-tree-loading-dot 1.1s ease-in-out infinite; animation-delay:{}ms;",
                            palette.accent,
                            ix * 140
                        ),
                    }
                }
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarRowPrimaryClickZone {
    Label,
    ToggleSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarRowPrimaryClickAction {
    Select,
    ToggleExpanded,
}

fn sidebar_row_primary_click_action(
    row: &BrowserRow,
    zone: SidebarRowPrimaryClickZone,
) -> SidebarRowPrimaryClickAction {
    match (row.kind, zone) {
        (BrowserRowKind::Group, SidebarRowPrimaryClickZone::ToggleSurface) => {
            SidebarRowPrimaryClickAction::ToggleExpanded
        }
        _ => SidebarRowPrimaryClickAction::Select,
    }
}

/// One pane cell of a compound split row ([[campaign-split-view-groups]]).
#[derive(Clone, PartialEq)]
struct SplitPaneCell {
    path: String,
    label: String,
    focused: bool,
}

/// The compound sidebar row for a split group — a miniature map of the split
/// geometry ([[campaign-split-view-groups]]). Side-by-side splits show cells
/// separated by a `|`; stacked splits show two stacked lines. Clicking a cell
/// focuses that pane; hovering shows the full title. There is deliberately NO
/// `×` on the row: all structural ops (ungroup, close a pane, close all) live
/// behind the right-click menu, so a built workspace is hard to lose by
/// accident.
#[component]
fn SplitGroupRow(
    axis_side_by_side: bool,
    cells: Vec<SplitPaneCell>,
    palette: Palette,
    accent: String,
    selected: bool,
    depth: usize,
    on_focus_pane: EventHandler<String>,
    on_open_context_menu: EventHandler<(f64, f64)>,
) -> Element {
    let indent = 12 + depth * 14;
    let row_background = if selected { palette.accent_soft } else { "transparent" };
    let separator = if axis_side_by_side { "row" } else { "column" };
    let cell_min_height = if axis_side_by_side { 34 } else { 18 };
    let dot_style = live_session_keep_alive_dot_style(palette);
    rsx! {
        div {
            "data-split-group-row": "1",
            style: format!(
                "display:flex; align-items:stretch; gap:8px; padding:5px 8px 5px {indent}px; \
                 border-radius:9px; background:{row_background}; cursor:default; user-select:none;"
            ),
            oncontextmenu: move |evt: MouseEvent| {
                evt.prevent_default();
                let coords = evt.client_coordinates();
                on_open_context_menu.call((coords.x, coords.y));
            },
            // Always-green traffic signal: grouping IS the keep-alive declaration.
            div {
                style: format!("flex:0 0 auto; align-self:center; {dot_style}"),
                title: "Split group · kept alive",
            }
            // The mini-map: cells laid out along the split axis.
            div {
                style: format!(
                    "flex:1 1 auto; min-width:0; display:flex; flex-direction:{separator}; \
                     gap:3px; align-items:stretch;"
                ),
                for (index, cell) in cells.iter().cloned().enumerate() {
                    {
                        let border = if cell.focused {
                            format!("box-shadow: inset 0 0 0 1.5px {accent};")
                        } else {
                            format!("box-shadow: inset 0 0 0 1px {}55;", palette.muted)
                        };
                        let text_color = if cell.focused { palette.text } else { palette.muted };
                        let cell_path = cell.path.clone();
                        let cell_label = cell.label.clone();
                        rsx! {
                            if index > 0 && axis_side_by_side {
                                span {
                                    style: format!("flex:0 0 auto; align-self:center; color:{}; font-size:12px;", palette.muted),
                                    "|"
                                }
                            }
                            div {
                                key: "{cell_path}",
                                title: "{cell_label}",
                                style: format!(
                                    "flex:1 1 0; min-width:0; min-height:{cell_min_height}px; display:flex; align-items:center; \
                                     padding:2px 7px; border-radius:6px; cursor:pointer; overflow:hidden; \
                                     white-space:nowrap; text-overflow:ellipsis; font-size:12px; \
                                     color:{text_color}; {border}"
                                ),
                                onclick: move |evt: MouseEvent| {
                                    evt.stop_propagation();
                                    on_focus_pane.call(cell_path.clone());
                                },
                                span {
                                    style: "overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                                    "{cell_label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ─── THE SHARED ROW ENGINE ([[campaign-libyggterm]] Phase 1) ─────────────────
// One visual vocabulary for every session-style list row in the product:
// `[indent] [status-dot] [icon] [title(+subtitle)] [badge] [actions]`.
// Three consumers — the cwdtree sidebar rows (the richest; its numbers ARE the
// vocabulary), the ychrome WebTabs rail, and app-pane `list-row`s — draw from
// ONE set of metrics/style functions, so a fourth consumer cannot drift.
// DRY is renderer-level only (settled 2026-07-11): what a row MEANS (session,
// tab, note) stays with its owner; only the look is shared.

#[derive(Clone, Copy, PartialEq, Debug)]
enum SessionRowDensity {
    /// The left sidebar's density (cwdtree Live Sessions / folders).
    Sidebar,
    /// The right rail's density (WebTabs rows, app-pane list-rows).
    Rail,
}

struct SessionRowMetrics {
    indent_base_px: u32,
    indent_step_px: u32,
    pad_v_px: u32,
    pad_h_px: u32,
    radius_px: u32,
    gap_px: u32,
    font_px: f32,
    icon_box_px: u32,
    /// A SEPARATE status column, ahead of the icon box — or `None` when this
    /// density carries status ON the icon slot, as ONE leading mark column.
    ///
    /// The cwdtree has two leading marks at once (a live session's keep-alive
    /// dot BESIDE its kind icon), so it pays for two columns. A right-rail row
    /// never does: a folder has a glyph and no dot, a web tab has a loading dot
    /// and no glyph, and a contributed leaf that has both wants the dot to read
    /// as a badge ON its icon, not as a column of its own. Paying for the
    /// second column there spent 15px of a ~220px rail on a box that is empty
    /// in every row that draws (user report 2026-07-31: "significant waste of
    /// horizontal space on each row").
    status_column_px: Option<u32>,
}

const fn session_row_metrics(density: SessionRowDensity) -> SessionRowMetrics {
    match density {
        // The cwdtree main row's numbers (its outer container is a bespoke
        // two-line column — SidebarRow consumes the indent/dot/icon/label
        // pieces of the vocabulary rather than the container fn).
        SessionRowDensity::Sidebar => SessionRowMetrics {
            indent_base_px: 12,
            indent_step_px: 12,
            pad_v_px: 5,
            pad_h_px: 9,
            radius_px: 12,
            gap_px: 8,
            font_px: 12.0,
            icon_box_px: 20,
            status_column_px: Some(9),
        },
        // TYPOGRAPHY and the icon box are IDENTICAL to the sidebar's — a human
        // eye reads the left cwdtree and a right-rail file list as the same
        // vocabulary, and an 11px rail title next to the 12px tree was visibly
        // "off" (user-caught 2026-07-17). What the rail may differ in is
        // SPACING and the LEADING ANATOMY:
        //
        //   * `status_column_px: None` — one mark column, not two (see the
        //     field's own note). 15px back on every row.
        //   * `indent_step_px: 19` — two space-advances MORE than the tree's
        //     12 (the row's own 12px Inter measures 3.375px per space, so two
        //     of them is 6.75px → 19). The rail asked for it and the tree did
        //     not, because the tree spends a DIFFERENT icon on every level
        //     (machine → folder → session kind) while every rail row wears the
        //     same mark: with that redundancy gone, indent is the only thing
        //     left carrying depth, so it has to carry more.
        SessionRowDensity::Rail => SessionRowMetrics {
            indent_base_px: 8,
            indent_step_px: 19,
            pad_v_px: 5,
            pad_h_px: 8,
            radius_px: 8,
            gap_px: 6,
            font_px: 12.0,
            icon_box_px: 20,
            status_column_px: None,
        },
    }
}

/// The row's outer box: indent, padding, radius, selection tint, drag dim.
/// `selected_bg` is the palette's accent_soft (or a doc-theme equivalent) —
/// ONE selection tint per surface, never a per-consumer mix.
fn session_row_container_style(
    density: SessionRowDensity,
    depth: u32,
    selected: bool,
    dimmed: bool,
    clickable: bool,
    selected_bg: &str,
    text_color: &str,
) -> String {
    let m = session_row_metrics(density);
    let indent = m.indent_base_px + depth * m.indent_step_px;
    format!(
        "position:relative; display:flex; align-items:center; gap:{gap}px; box-sizing:border-box; min-width:0; overflow:hidden; \
         padding:{pv}px {ph}px {pv}px {indent}px; border-radius:{radius}px; font-size:{font}px; color:{text_color}; \
         background:{bg}; opacity:{opacity}; cursor:{cursor}; user-select:none; -webkit-user-select:none;",
        gap = m.gap_px,
        pv = m.pad_v_px,
        ph = m.pad_h_px,
        radius = m.radius_px,
        font = m.font_px,
        bg = if selected { selected_bg } else { "transparent" },
        opacity = if dimmed { "0.58" } else { "1" },
        cursor = if clickable { "pointer" } else { "default" },
    )
}

/// The fixed-width SEPARATE status column, for a density that has one (the
/// cwdtree). Laid out even when empty, so an appearing dot never shoves the
/// title sideways. A density with `status_column_px: None` draws no such column
/// — its status dot rides [`session_row_mark_column_style`] instead.
fn session_row_dot_rail_style(density: SessionRowDensity) -> String {
    let m = session_row_metrics(density);
    format!(
        "display:inline-flex; align-items:center; justify-content:center; \
         width:{w}px; min-width:{w}px; height:{h}px; flex:0 0 auto;",
        w = m.status_column_px.unwrap_or(0),
        h = m.icon_box_px,
    )
}

/// The ONE leading mark column, for a density with no separate status column.
/// Always laid out, icon or no icon, so every row of a list starts its title at
/// one x — the rule the two-column anatomy kept with an always-drawn dot rail.
/// `position:relative` because the status dot is positioned INSIDE it.
fn session_row_mark_column_style(density: SessionRowDensity, color: &str) -> String {
    let m = session_row_metrics(density);
    format!(
        "position:relative; display:inline-flex; align-items:center; justify-content:center; \
         width:{s}px; min-width:{s}px; height:{s}px; color:{color}; flex:0 0 auto;",
        s = m.icon_box_px,
    )
}

/// Where the status dot sits inside the mark column: CENTERED when the mark is
/// the dot itself (a web tab has no glyph), a corner BADGE when it shares the
/// column with an icon (a contributed leaf that is both dirty and a `.md`).
/// Absolute either way, so an appearing dot moves nothing at all — the promise
/// the always-laid-out dot rail was making, kept more strictly.
///
/// ⚠ ONE key set across both branches, values only. Dioxus applies `style`
/// property-by-property and never clears a key a later render drops.
fn session_row_status_badge_style(over_icon: bool) -> String {
    let (inset, margin) = if over_icon {
        ("auto 0px 0px auto", "0")
    } else {
        ("0px", "auto")
    };
    format!(
        "position:absolute; inset:{inset}; margin:{margin}; \
         display:inline-flex; align-items:center; justify-content:center; \
         width:auto; height:auto; pointer-events:none;"
    )
}

/// A sidebar row's two indents: the row's own left padding, and how far its
/// CONTENT is pushed inside that.
///
/// ⛔ **THE TRAFFIC LIGHTS ARE A COLUMN OF THEIR OWN, FAR LEFT, IN A FIXED
/// AREA.** Owner-directed, and it is a layout MODEL rather than a rule about
/// padding. A live row has two zones:
///
/// | zone | holds | behaviour |
/// |---|---|---|
/// | gutter | the status dot, nothing else | fixed width, flush to the row's own left edge, identical on every row at every depth |
/// | content | icon, title, trailing controls | starts after the gutter, and is the only thing nesting moves |
///
/// The zones are SIBLINGS in the markup, not a convention about who pads what,
/// so "something else sits before the dot and the column kinks" is not a state
/// this row can be in. His three reasons, and each one is a consequence:
/// **more room for titles** — the gutter is the row's own horizontal padding
/// rather than the old `base + step` leading run, so every live row's title
/// starts further left than it used to, including rows in no set at all;
/// **nesting costs no gutter** — depth is spent out of the content zone, so the
/// only thing that narrows is the title, from a wider start; and **it looks
/// better** — the dots form one unbroken vertical line, which is checkable: if
/// that line kinks at any row, the zone separation is not real.
///
/// Everywhere else (the cwd tree) the whole row indents as it always has: a
/// folder has no dot in that gutter, so there is nothing to hold still, and
/// stepping the row is what makes a tree readable.
fn sidebar_row_indents(depth: usize, in_live_region: bool) -> (u32, u32) {
    let metrics = session_row_metrics(SessionRowDensity::Sidebar);
    if in_live_region {
        // Flush to the row's own edge — the same padding its right side wears,
        // so the gutter is the row's margin and not a column of its own width.
        return (
            metrics.pad_h_px,
            depth.saturating_sub(1) as u32 * metrics.indent_step_px,
        );
    }
    (
        depth as u32 * metrics.indent_step_px + metrics.indent_base_px,
        0,
    )
}

fn session_row_icon_box_style(density: SessionRowDensity, color: &str) -> String {
    let m = session_row_metrics(density);
    format!(
        "display:inline-flex; align-items:center; justify-content:center; \
         width:{s}px; min-width:{s}px; height:{s}px; color:{color}; flex:0 0 auto;",
        s = m.icon_box_px,
    )
}

/// Title typography + ellipsis. Positioning (flex membership) stays with the
/// consumer — the sidebar's label lives inside a space-between cluster, the
/// rail's stretches.
fn session_row_label_style(density: SessionRowDensity, color: &str, bold: bool) -> String {
    let m = session_row_metrics(density);
    format!(
        "min-width:0; display:inline-block; font-size:{font}px; font-weight:{weight}; color:{color}; \
         white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
        font = m.font_px,
        weight = if bold { 600 } else { 500 },
    )
}

// The revealed verbs' chip-feather width used to live here. The chip is gone
// (the user rejected it — DESIGN.md, "NO BACKGROUND BEHIND THE VERBS"), and so
// is the constant: a number nothing reads is the seed the next chip grows from.

/// The in-flow cell the trailing verbs hang off.
///
/// This is the whole of "the label gets the width back", and it works by the
/// child being `display:none` at rest rather than by floating: a `display:none`
/// child gives the cell zero width, so the title track measures the full row
/// whenever no verbs are showing. Measured on the live cwdtree 2026-08-01: an
/// `opacity:0` ✕ still claimed 18px of layout plus its 6px gap on every
/// live-session row, so a truncated title stopped 24px short of the row it was
/// in — hiding is not enough, only leaving the flex line is.
///
/// Revealed, the cell DOES take its width and the title ellipsizes. That reflow
/// is deliberate: the alternative was floating the verbs on a frosted chip, and
/// the user rejected it because the title ran underneath and the ✕ read as a
/// smudge. See DESIGN.md, "NO BACKGROUND BEHIND THE VERBS".
///
/// It is an anchor rather than the row itself so the verbs land to the LEFT of
/// the always-visible `expander` — a hover must never cover the disclosure
/// chevron, which is permanent chrome, not a hover-revealed verb.
///
/// `row_gap_px` is the gap of the flex line the anchor is placed in, and it is
/// taken straight back out again as a negative margin. A zero-WIDTH item is not
/// a zero-COST item: flex `gap` applies between every pair of items, so an
/// anchor sitting between the title and the expander was still charging the
/// title one gap (measured live 2026-08-01: a rail title stopped at 2540 where
/// the app tab's — the one row with no verbs at all — reached 2546). Cancelling
/// it makes "the entire width" literally true, and it also keeps a group row's
/// title-to-chevron distance at ONE gap instead of two.
fn session_row_actions_anchor_style(_row_gap_px: u32) -> String {
    // A plain in-flow cell. It is zero-width while its child is `display:none`,
    // so a row with no revealed verbs gives the title everything; when the
    // child appears the cell grows and the title's `min-width:0` + ellipsis
    // does the truncating.
    "display:flex; align-items:center; flex:0 0 auto;".to_string()
}

/// The revealed verbs themselves: out of flow, pinned to the anchor's trailing
/// edge, feathered in from the left by a mask.
///
/// LAYOUT only. What the chip is MADE of — the `backdrop-filter` blur and the
/// wash of the surface's colour — lives in [`session_row_hover_css`], so it is
/// only paid for while the verbs are showing. The pair is deliberate:
/// yggterm's window is TRANSPARENT and the backdrop under a row is
/// `[data-yggterm-app-bg]`'s 135° gradient — measured live 2026-08-01, it runs
/// rgb(174,223,220) at the top of the sidebar to rgb(207,227,233) at the bottom
/// — so an opaque fade of any single palette colour would read as a bright
/// rectangle sliding down the list. The blur takes its colour from whatever is
/// actually behind (gradient, selection tint, a dark theme) and only smears the
/// glyphs under it; the wash finishes the job and is the graceful degradation
/// if a platform ever lacks `backdrop-filter`.
///
/// The mask is here rather than in the CSS because it is the SHAPE of the chip,
/// and it must match the `padding-left` that keeps the first glyph clear of the
/// feather — one number, one place.
///
/// `bleed_v_px`/`bleed_h_px` are the row's OWN padding, and the chip is pulled
/// out over them so it reaches the row's real edges. Without that it stops at
/// the content box and reads as a white sticker floating inside the row, with a
/// strip of the selection tint showing past its right edge (caught on the live
/// pixel, 2026-08-01). The row clips it back to its own rounded corners — every
/// row family sets `overflow:hidden` — so bleeding costs nothing and buys the
/// chip the row's own shape. The feathered left edge is the only edge that
/// should be visible at all.
fn session_row_actions_style(_bleed_v_px: u32, _bleed_h_px: u32) -> String {
    // `display` is owned by the reveal rule (none -> inline-flex); everything
    // here is the in-flow box it becomes. No background, no mask, no bleed —
    // the row's own surface shows through, which is the whole point.
    // 4px between verbs, not 2: two 18px marks a hair apart read as one smudged
    // control, which is half of what "the icons look illegible" meant.
    "align-items:center; justify-content:flex-end; gap:4px; margin-left:6px;".to_string()
}

/// A row's trailing verb button.
///
/// The metrics are a TOUCH TARGET, not a text box: an 18px square with the mark
/// centred in it, so two verbs on one row cannot collide and each is big enough
/// to hit. It used to be `padding:2px 4px` around whatever character the caller
/// passed, which put ychrome's `⧉ ⏱ ✎` shoulder to shoulder at 11px — the user's
/// report was that the vault rail's icons "look illegible" and want "a bit of
/// padding between them" (2026-08-04). The gap itself is the container's
/// ([`session_row_actions_style`]).
///
/// ⛔ Still NO BACKGROUND at rest (DESIGN.md ▸ Session-style rows: the frosted
/// chip is a settled prohibition). The square is invisible until the pointer is
/// on it — `session_row_hover_css` lights it.
fn session_row_action_button_style(color: &str) -> String {
    format!(
        "display:inline-flex; align-items:center; justify-content:center; \
         width:18px; height:18px; border:none; background:transparent; color:{color}; \
         cursor:pointer; font-size:11px; line-height:1; padding:0; border-radius:5px; \
         flex:0 0 auto; opacity:0.72;"
    )
}

/// A small trailing pill (Phase 5: ychrome profile badges live here).
fn session_row_badge_style(color: &str) -> String {
    format!(
        "flex:0 0 auto; font-size:9.5px; font-weight:700; padding:1px 6px; border-radius:999px; \
         background:color-mix(in srgb, {color} 18%, transparent); color:{color};"
    )
}

/// The shared row COMPONENT, for consumers whose interactivity fits the
/// standard shape (click to select, a few trailing actions): WebTabs rail
/// rows, app-pane list-rows. The cwdtree SidebarRow keeps its bespoke DOM
/// (drag/rename/keytips/click-zones) and consumes the same style functions —
/// one vocabulary either way. `depth` exists so a future `list-row` schema
/// `depth`/`group` needs no component change.
#[component]
fn SessionStyleRow(
    #[props(extends = div, extends = GlobalAttributes)] attributes: Vec<Attribute>,
    density: SessionRowDensity,
    #[props(default = 0)] depth: u32,
    #[props(default = false)] selected: bool,
    #[props(default = false)] dimmed: bool,
    text_color: String,
    selected_bg: String,
    label: String,
    #[props(default)] subtitle: Option<String>,
    #[props(default)] subtitle_color: Option<String>,
    #[props(default)] badge: Option<String>,
    #[props(default)] badge_color: Option<String>,
    #[props(default)] dot: Option<Element>,
    #[props(default)] icon: Option<Element>,
    /// Icon slot color. The cwdtree's rule: MUTED normally, full text color
    /// on the selected row — pass both and the row picks. Defaults to
    /// `text_color` when unset.
    #[props(default)] icon_color: Option<String>,
    /// The trailing DISCLOSURE control of a group row, and the cwd tree's own
    /// placement for it: after the badge, ALWAYS visible — unlike `actions`,
    /// which are hover-revealed verbs. A row's expand/collapse is not a verb
    /// you have to hover to discover.
    #[props(default)] expander: Option<Element>,
    #[props(default)] actions: Option<Element>,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
    #[props(default)] onmousedown: Option<EventHandler<MouseEvent>>,
    #[props(default)] onmouseenter: Option<EventHandler<MouseEvent>>,
    /// Double-click — the tree's own "rename this row" gesture, and declared
    /// here for the same reason `oncontextmenu` is.
    #[props(default)] ondoubleclick: Option<EventHandler<MouseEvent>>,
    /// Right-click. Declared like the other listeners rather than left to the
    /// attribute spread, because the spread carries ATTRIBUTES and a listener
    /// passed through it would silently never fire.
    #[props(default)] oncontextmenu: Option<EventHandler<MouseEvent>>,
) -> Element {
    let clickable = onclick.is_some();
    let metrics = session_row_metrics(density);
    let container = session_row_container_style(
        density,
        depth,
        selected,
        dimmed,
        clickable,
        &selected_bg,
        &text_color,
    );
    let subtitle_text = subtitle.filter(|text| !text.is_empty());
    let badge_text = badge.filter(|text| !text.is_empty());
    // Selection is the tint, NEVER a weight change — the cwdtree's rule, and
    // a bolded selected row read as a different font next to it.
    let title_style = session_row_label_style(density, &text_color, false);
    let icon_slot_color = icon_color.unwrap_or_else(|| text_color.clone());
    // Read BEFORE the rsx moves it: the chip's horizontal bleed depends on
    // whether the always-visible expander is behind it.
    let expander_present = expander.is_some();
    rsx! {
        div {
            "data-session-row": "1",
            "data-session-row-selected": if selected { "true" } else { "false" },
            style: "{container}",
            onclick: move |evt| {
                if let Some(handler) = &onclick {
                    handler.call(evt);
                }
            },
            onmousedown: move |evt| {
                if let Some(handler) = &onmousedown {
                    handler.call(evt);
                }
            },
            onmouseenter: move |evt| {
                if let Some(handler) = &onmouseenter {
                    handler.call(evt);
                }
            },
            ondoubleclick: move |evt| {
                if let Some(handler) = &ondoubleclick {
                    handler.call(evt);
                }
            },
            oncontextmenu: move |evt| {
                if let Some(handler) = &oncontextmenu {
                    handler.call(evt);
                }
            },
            ..attributes,
            // THE LEADING SLOT is ALWAYS laid out, mark or no mark (DESIGN.md
            // "Session-style rows"): a row whose status appears later — a yedit
            // note going dirty — must not shove its own title sideways, and two
            // rows of the same list must not start their titles at different x.
            //
            // How WIDE that slot is, is the density's answer. The cwdtree pays
            // for two columns because it shows two marks at once; a rail row
            // never does, so it pays for one and the dot rides the icon.
            if metrics.status_column_px.is_some() {
                span {
                    style: session_row_dot_rail_style(density),
                    if let Some(dot) = dot.clone() {
                        {dot}
                    }
                }
                if let Some(icon) = icon.clone() {
                    span {
                        style: session_row_icon_box_style(density, &icon_slot_color),
                        {icon}
                    }
                }
            } else {
                span {
                    "data-session-row-mark": "1",
                    style: session_row_mark_column_style(density, &icon_slot_color),
                    if let Some(icon) = icon.clone() {
                        {icon}
                    }
                    if let Some(dot) = dot.clone() {
                        span {
                            style: session_row_status_badge_style(icon.is_some()),
                            {dot}
                        }
                    }
                }
            }
            if let Some(subtitle_text) = subtitle_text {
                div {
                    style: "display:flex; flex-direction:column; gap:1px; min-width:0; flex:1 1 auto;",
                    div {
                        style: "{title_style}",
                        "{label}"
                    }
                    div {
                        style: format!(
                            "font-size:10px; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                            subtitle_color.as_deref().unwrap_or(&text_color),
                        ),
                        "{subtitle_text}"
                    }
                }
            } else {
                span {
                    style: "flex:1 1 auto; {title_style}",
                    "{label}"
                }
            }
            if let Some(badge_text) = badge_text {
                span {
                    style: session_row_badge_style(
                        badge_color.as_deref().unwrap_or(&text_color),
                    ),
                    "{badge_text}"
                }
            }
            if let Some(actions) = actions {
                // Revealed by hover / selection / keyboard focus
                // (session_row_hover_css) and IN FLOW. At rest the child is
                // `display:none`, so this cell is zero-width and the title gets
                // the whole row; revealed, the cell grows and the title
                // ellipsizes to fit.
                //
                // It USED to float out of flow on a frosted chip so the title
                // never reflowed. The user rejected that on sight, on every
                // cwdtree at once: the title ran under the chip and the verbs
                // read as a smudge rather than as buttons.
                //
                // Ahead of the `expander` on purpose: an expander is permanent
                // chrome (DESIGN.md — "always visible, unlike `actions`"), so
                // the verbs land to its LEFT and never cover it.
                span {
                    "data-session-row-actions-anchor": "1",
                    style: session_row_actions_anchor_style(metrics.gap_px),
                    span {
                        "data-session-row-actions": "1",
                        style: session_row_actions_style(
                            metrics.pad_v_px,
                            if expander_present { 0 } else { metrics.pad_h_px },
                        ),
                        {actions}
                    }
                }
            }
            if let Some(expander) = expander {
                span {
                    "data-session-row-expander": "1",
                    style: "display:inline-flex; align-items:center; gap:2px; flex:0 0 auto;",
                    {expander}
                }
            }
        }
    }
}

/// The tree's DISCLOSURE CHEVRON — the one glyph in the product that says "this
/// row has an inside". Down = open, right = closed. Drawn once here so the
/// cwdtree's folders, a contributed pane's groups and the WebTabs rail's
/// folders cannot drift into three different triangles; tweak it and every
/// surface inherits.
#[component]
fn RowDisclosureChevron(expanded: bool) -> Element {
    rsx! {
        svg {
            width: "10",
            height: "10",
            view_box: "0 0 12 12",
            fill: "none",
            xmlns: "http://www.w3.org/2000/svg",
            path {
                d: if expanded { "M3 4.75L6 7.75L9 4.75" } else { "M4.75 3L7.75 6L4.75 9" },
                stroke: "currentColor",
                stroke_width: "1.35",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
        }
    }
}

/// Focus an in-place rename field AND SELECT what is already in it.
///
/// A row born with a placeholder name — a folder's "New folder", a note's
/// "Untitled" — must take the user's first keystroke as a REPLACEMENT. Focus
/// alone leaves the caret at the end, so the user types into the placeholder
/// and then has to delete it, which is the "New folder text should be
/// selected" report. `set_focus` cannot express a selection, so this is the one
/// place the shell reaches for the DOM, and every rename field in the app goes
/// through it rather than each growing its own snippet.
fn select_rename_field(selector: &str) {
    let _ = document::eval(&format!(
        "const el = document.querySelector({selector}); \
         if (el) {{ el.focus(); el.select(); }}",
        selector = serde_json::Value::String(selector.to_string()),
    ));
}

/// The hit target a disclosure chevron sits in, inside a row's leading slot.
/// Transparent and borderless: the chevron IS the affordance.
fn row_disclosure_button_style(color: &str) -> String {
    format!(
        "display:inline-flex; align-items:center; justify-content:center; width:100%; height:100%; \
         border:none; background:transparent; padding:0; color:{color}; cursor:pointer;"
    )
}

/// A file-type badge for `list-row` icons of the form `file:<ext>` — a small
/// outlined rectangle carrying the extension text ("md", "txt"), or "·" when
/// the file has none. Generic vocabulary: any app can declare it; yggterm
/// knows nothing about notes. Sized to the shared icon box.
#[component]
fn FileBadgeIcon(ext: String) -> Element {
    let label = if ext.is_empty() {
        "·".to_string()
    } else {
        let mut text = ext.to_lowercase();
        text.truncate(4);
        text
    };
    rsx! {
        svg {
            width: "18",
            height: "16",
            view_box: "0 0 18 16",
            fill: "none",
            rect {
                x: "1",
                y: "1",
                width: "16",
                height: "14",
                rx: "3",
                stroke: "currentColor",
                stroke_width: "1.1",
            }
            text {
                x: "9",
                y: "8.5",
                text_anchor: "middle",
                dominant_baseline: "middle",
                fill: "currentColor",
                font_size: "6.5",
                font_weight: "600",
                font_family: "inherit",
                letter_spacing: "0.02em",
                "{label}"
            }
        }
    }
}

/// Render a `list-row` icon string: the `file:<ext>` vocabulary gets the
/// badge; anything else (an emoji glyph) renders as text.
fn app_pane_row_icon(icon: &str) -> Element {
    match icon.strip_prefix("file:") {
        // A file badge draws its OWN shape and carries its own ground; putting
        // a second disc behind it would be a badge inside a badge.
        Some(ext) => rsx! { FileBadgeIcon { ext: ext.to_string() } },
        // `icon:<name>` reaches the shell's own stroked set; anything else is
        // still drawn as the character the app sent.
        //
        // THE DISC IS ACCESSIBILITY, not decoration. A bare 13px stroke on the
        // page ground is a low-contrast hairline that has to be hunted for in a
        // list of forty rows. A filled disc gives the mark a consistent target
        // and a consistent contrast floor whatever the row is sitting on, and
        // the extra two pixels are what make a 1.25 stroke read as a shape
        // rather than a smudge. `geometricPrecision` stops the rasteriser
        // snapping those strokes to the pixel grid, which is what made curves
        // look faceted at this size.
        None => rsx! {
            span {
                style: "display:inline-flex; align-items:center; justify-content:center; \
                        width:26px; height:26px; flex:0 0 26px; border-radius:50%; \
                        background:rgba(127,127,127,0.16); \
                        shape-rendering:geometricPrecision;",
                {shell_glyph(icon, 15)}
            }
        },
    }
}

#[component]
fn SidebarRow(
    row: BrowserRow,
    visible_label: String,
    icon_kind: String,
    busy_icon: bool,
    /// This row has been written to and has said nothing back for longer than
    /// any normal round trip — DESIGN.md's amber ATTENTION state. ⚠ A trigger,
    /// never a verdict; `terminal input-check` settles it.
    input_unanswered: bool,
    selected: bool,
    drop_target: Option<DragDropPlacement>,
    dragging: bool,
    drag_active: bool,
    renaming: bool,
    rename_focus_pending: bool,
    rename_focused_once: bool,
    rename_value: String,
    show_live_close: bool,
    palette: Palette,
    /// The session's web-surface PROFILE when it is not the default — the
    /// shared row vocabulary's badge slot ([[campaign-libyggterm]] Phase 5:
    /// multiple profile browsers are categorization; the badge tells them
    /// apart). Empty = no badge.
    web_profile: String,
    /// `ALT,E`'s letter when THIS row is the "here" row the row menu would open on,
    /// else empty. A KeyTip is painted on the thing it acts on, and what `ALT,E`
    /// acts on is a row (spec §8: the layer's actions apply to the focused item).
    row_menu_tip: String,
    /// `ALT,J`'s letter on the Live Sessions row — the list jump mode walks — else
    /// empty.
    jump_tip: String,
    on_select: EventHandler<TreeSelectionMode>,
    // Fired on mouse-DOWN to paint the selection highlight immediately (instant
    // feedback) without opening/switching the session — the open still happens
    // on mouse-up via `on_select`. Eliminates the perceived "selection latency"
    // where the highlight only appeared on release. #14.
    on_press_highlight: EventHandler<TreeSelectionMode>,
    on_set_expanded: EventHandler<bool>,
    on_open_context_menu: EventHandler<(f64, f64)>,
    on_delete_row: EventHandler<MouseEvent>,
    on_begin_rename: EventHandler<MouseEvent>,
    on_regenerate_title: EventHandler<MouseEvent>,
    on_update_rename: EventHandler<String>,
    on_focus_rename: EventHandler<()>,
    on_commit_rename: EventHandler<()>,
    on_cancel_rename: EventHandler<()>,
    on_start_drag: EventHandler<MouseEvent>,
    on_drag_move: EventHandler<MouseEvent>,
    on_drag_hover: EventHandler<(DragDropPlacement, MouseEvent)>,
    on_drag_leave: EventHandler<MouseEvent>,
    on_drop_into_row: EventHandler<()>,
    on_end_drag: EventHandler<()>,
) -> Element {
    // Indent math from the SHARED row engine — the cwdtree is the vocabulary's
    // reference consumer, so its numbers and the engine's are one definition.
    let (indent, label_indent) = sidebar_row_indents(row.depth, show_live_close);
    let draggable = is_tree_drag_source_row(&row);
    let row_kind_label = format!("{:?}", row.kind);
    let drop_hovered = drop_target.is_some();
    let top_line = drop_target == Some(DragDropPlacement::Before);
    let bottom_line = drop_target == Some(DragDropPlacement::After);
    let fill_target = drop_target == Some(DragDropPlacement::Into);
    if row.kind == BrowserRowKind::Separator {
        let row_for_enter = row.clone();
        let row_for_move = row.clone();
        let focus_path = row.full_path.clone();
        return rsx! {
            div {
                id: "{sidebar_row_dom_id(&row.full_path)}",
                "data-sidebar-row-path": "{row.full_path}",
                // Unbounded list: navigated, not badged (§8). The row is a div
                // outside the walk's interactable selector, so it needs no
                // stamp; a subtree stamp here is forbidden (§12.1) — any
                // interactable CHILD carries its own per-element exemption.
                "data-sidebar-row-kind": "Separator",
                "data-sidebar-row-label": "{row.label}",
                "data-sidebar-row-depth": "{row.depth}",
                "data-sidebar-row-draggable": if draggable { "true" } else { "false" },
                tabindex: if selected { "0" } else { "-1" },
                "data-selected": if selected { "true" } else { "false" },
                "data-drop-target": match drop_target {
                    Some(DragDropPlacement::Before) => "before",
                    Some(DragDropPlacement::Into) => "into",
                    Some(DragDropPlacement::After) => "after",
                    None => "none",
                },
                style: format!(
                    "width:100%; display:flex; align-items:center; gap:10px; border:none; background:transparent; cursor:{}; \
                     padding:11px 9px 11px {}px; margin:0; opacity:{}; border-radius:12px; background:{}; \
                     box-sizing:border-box; min-width:0; overflow:hidden; user-select:none; -webkit-user-select:none; \
                     transition: transform 140ms ease, background 140ms ease, opacity 140ms ease, box-shadow 140ms ease; \
                     transform:translateY(0px); box-shadow:{}; position:relative;",
                    if dragging { "grabbing" } else { "pointer" },
                    indent
                    , if dragging { "0.58" } else { "1" },
                    if selected || fill_target { palette.accent_soft } else { "transparent" },
                    if top_line && drag_active {
                        format!("inset 0 2px 0 {}", palette.accent)
                    } else if bottom_line && drag_active {
                        format!("inset 0 -2px 0 {}", palette.accent)
                    } else {
                        "none".to_string()
                    },
                ),
                draggable: false,
                onmousedown: move |evt| {
                    claim_sidebar_focus_by_path(Some(&focus_path));
                    if draggable
                        && evt.trigger_button() == Some(MouseButton::Primary)
                        && !evt.modifiers().contains(Modifiers::SHIFT)
                        && !evt.modifiers().contains(Modifiers::CONTROL)
                        && !evt.modifiers().contains(Modifiers::META)
                    {
                        on_start_drag.call(evt);
                    }
                },
                ondoubleclick: move |evt| on_begin_rename.call(evt),
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    let coords = evt.client_coordinates();
                    on_open_context_menu.call((coords.x, coords.y));
                },
                onmouseleave: move |evt| {
                    on_drag_leave.call(evt);
                },
                onmouseenter: move |evt| {
                    if drag_active {
                        // A separator is never a live row and never holds one.
                        let placement = drag_drop_placement_from_pointer(
                            &row_for_enter,
                            evt.element_coordinates().y,
                            false,
                        );
                        on_drag_hover.call((placement, evt));
                    }
                },
                onmousemove: move |evt| {
                    if draggable && evt.held_buttons().contains(MouseButton::Primary) {
                        on_drag_move.call(evt.clone());
                    }
                    if drag_active {
                        let placement = drag_drop_placement_from_pointer(
                            &row_for_move,
                            evt.element_coordinates().y,
                            false,
                        );
                        on_drag_hover.call((placement, evt));
                    }
                },
                onmouseup: move |evt| {
                    evt.stop_propagation();
                    if drag_active {
                        on_drop_into_row.call(());
                    } else if evt.trigger_button() == Some(MouseButton::Primary) {
                        on_select.call(tree_selection_mode_for_modifiers(evt.modifiers()));
                    }
                    on_end_drag.call(());
                },
                div {
                    style: format!(
                        "flex:1; min-width:0; height:{}px; background:{}; opacity:{};",
                        if drop_hovered { 2 } else { 1 },
                        if drop_hovered || selected { palette.accent } else { palette.border },
                        if drop_hovered || selected { "0.96" } else { "0.72" }
                    ),
                }
                if renaming {
                        input {
                            "data-tree-rename-input": "1",
                            "data-tree-rename-row-path": "{row.full_path}",
                            "data-tree-rename-row-depth": "{row.depth}",
                            "data-tree-rename-focused-once": if rename_focused_once { "true" } else { "false" },
                            "data-tree-rename-focus-pending": if rename_focus_pending { "true" } else { "false" },
                            style: format!(
                                "width:140px; height:29px; border:none; border-radius:10px; background:rgba(255,255,255,0.92); \
                                 color:{}; font-size:12px; font-weight:600; padding:0 10px; box-shadow: inset 0 0 0 1px rgba(204,214,224,0.9);",
                                palette.text
                            ),
                            // ⛔ `initial_value`, NEVER `value`. See the twin at
                            // the other branch and `docs/agent-field-guide.md`:
                            // `value` is VOLATILE, `rename_value` arrives from the
                            // lagging snapshot, and the stale re-assert yanked the
                            // caret to the end after every keystroke.
                            initial_value: rename_value,
                            onmounted: move |evt| async move {
                                let _ = evt.set_focus(true).await;
                            },
                            oninput: move |evt| on_update_rename.call(evt.value()),
                            onfocus: move |_| on_focus_rename.call(()),
                            onblur: move |_| on_commit_rename.call(()),
                            onkeydown: move |evt| {
                                evt.stop_propagation();
                                if evt.key() == Key::Enter {
                                    evt.prevent_default();
                                    on_commit_rename.call(());
                                } else if evt.key() == Key::Escape {
                                    evt.prevent_default();
                                    on_cancel_rename.call(());
                                }
                            },
                            onclick: |evt| evt.stop_propagation(),
                            onmousedown: |evt| evt.stop_propagation(),
                            onmouseup: |evt| evt.stop_propagation(),
                            ondoubleclick: |evt| evt.stop_propagation(),
                            oncontextmenu: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                    }
                } else {
                    span {
                        style: format!(
                            "min-width:0; font-size:11.25px; font-weight:700; letter-spacing:0.04em; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            if drop_hovered || selected { palette.accent } else { palette.muted }
                        ),
                        "{row.label}"
                    }
                }
                div {
                    style: format!(
                        "flex:1; min-width:0; height:{}px; background:{}; opacity:{};",
                        if drop_hovered { 2 } else { 1 },
                        if drop_hovered || selected { palette.accent } else { palette.border },
                        if drop_hovered || selected { "0.96" } else { "0.72" }
                    ),
                }
            }
        };
    }
    let background = if drop_hovered {
        "rgba(95, 168, 255, 0.14)"
    } else if selected {
        palette.accent_soft
    } else if row.kind == BrowserRowKind::Group && row.depth == 0 {
        "transparent"
    } else {
        "transparent"
    };
    let icon_color = if row.kind == BrowserRowKind::Group && row.depth == 0 && row.expanded {
        palette.accent
    } else if selected {
        palette.text
    } else {
        palette.muted
    };
    let machine_health = machine_health_from_label(&row.label);
    let row_kept_alive = show_live_close && row.detail_label.starts_with("Kept alive");
    let label_color = if row.kind == BrowserRowKind::Group && row.depth == 0 && row.expanded {
        palette.accent
    } else if selected {
        palette.text
    } else if row.kind == BrowserRowKind::Group && row.depth > 0 {
        palette.muted
    } else {
        palette.text
    };
    let row_for_enter = row.clone();
    let row_for_move = row.clone();
    let row_for_root_mouseup = row.clone();
    let row_for_press_highlight = row.clone();
    let row_for_icon_mousedown = row.clone();
    let row_for_icon_mouseup = row.clone();
    let row_for_label_mouseup = row.clone();
    let focus_path = row.full_path.clone();
    let icon_focus_path = row.full_path.clone();
    let label_focus_path = row.full_path.clone();
    let row_is_group = row.kind == BrowserRowKind::Group;
    // A row set's head. `DESIGN.md` §"Row sets": the sidebar shows no noun at
    // all for a set, only a disclosure control on the head — so this is the ONE
    // thing the head looks different by.
    let row_is_row_set_head = row_heads_a_row_set(&row);
    let row_set_member_count = row.descendant_sessions.saturating_sub(1);
    let row_expanded = row.expanded;
    let row_toggle_target_expanded = !row.expanded;
    rsx! {
        div {
            id: "{sidebar_row_dom_id(&row.full_path)}",
            "data-sidebar-row-path": "{row.full_path}",
            // Unbounded list: navigated, not badged (§8). The ALT layer acts on
            // the FOCUSED row. The row is a div outside the walk's interactable
            // selector, so it needs no stamp; a subtree stamp here is forbidden
            // (§12.1) — the close/expander buttons carry their own per-element
            // "list-item" exemptions instead.
            "data-sidebar-row-kind": "{row_kind_label}",
            "data-sidebar-row-label": "{visible_label}",
            "data-sidebar-row-detail": "{row.detail_label}",
            "data-sidebar-row-depth": "{row.depth}",
            // The SHARED row marks. SidebarRow keeps its bespoke DOM (drag,
            // rename, keytips, click zones) but it is a session-style row like
            // any other, and wearing these is how it inherits the one reveal
            // rule (session_row_hover_css) instead of keeping a private copy —
            // the copy is how the tree and the rail disagreed about the ✕.
            "data-session-row": "1",
            "data-session-row-selected": if selected { "true" } else { "false" },
            "data-sidebar-row-draggable": if draggable { "true" } else { "false" },
            "data-sidebar-live-session-member": if show_live_close { "true" } else { "false" },
            tabindex: if selected { "0" } else { "-1" },
            "data-machine-health": if let Some(health) = machine_health {
                machine_health_attr_value(health)
            } else {
                ""
            },
            "data-selected": if selected { "true" } else { "false" },
            "data-drop-target": match drop_target {
                Some(DragDropPlacement::Before) => "before",
                Some(DragDropPlacement::Into) => "into",
                Some(DragDropPlacement::After) => "after",
                None => "none",
            },
            style: format!(
                "width:100%; display:flex; flex-direction:column; align-items:stretch; gap:1px; \
                 border:none; border-radius:12px; background:{}; padding:5px 9px 5px {}px; margin:0; opacity:{}; cursor:{}; \
                 box-sizing:border-box; min-width:0; overflow:hidden; user-select:none; -webkit-user-select:none; \
                 transition: transform 140ms ease, background 140ms ease, opacity 140ms ease, box-shadow 140ms ease; \
                 transform:translateY(0px); box-shadow:{}; position:relative;",
                if fill_target { "rgba(95, 168, 255, 0.14)" } else { background },
                indent,
                if dragging { "0.58" } else { "1" },
                if dragging { "grabbing" } else { "pointer" },
                if top_line && drag_active {
                    format!("inset 0 2px 0 {}", palette.accent)
                } else if bottom_line && drag_active {
                    format!("inset 0 -2px 0 {}", palette.accent)
                } else {
                    "none".to_string()
                },
            ),
            draggable: false,
            onmousedown: move |evt| {
                claim_sidebar_focus_by_path(Some(&focus_path));
                if evt.trigger_button() == Some(MouseButton::Primary)
                    && !evt.modifiers().contains(Modifiers::SHIFT)
                    && !evt.modifiers().contains(Modifiers::CONTROL)
                    && !evt.modifiers().contains(Modifiers::META)
                {
                    // #14: paint the highlight on press for instant feedback,
                    // but only for rows whose primary click selects (not groups
                    // that toggle-expand) and only when not already selected (so
                    // a press that begins a multi-row drag keeps the selection).
                    if !selected
                        && sidebar_row_primary_click_action(
                            &row_for_press_highlight,
                            SidebarRowPrimaryClickZone::ToggleSurface,
                        ) == SidebarRowPrimaryClickAction::Select
                    {
                        on_press_highlight.call(TreeSelectionMode::Replace);
                    }
                    if draggable {
                        on_start_drag.call(evt);
                    }
                }
            },
            ondoubleclick: move |evt| on_begin_rename.call(evt),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                evt.stop_propagation();
                let coords = evt.client_coordinates();
                on_open_context_menu.call((coords.x, coords.y));
            },
            onmouseleave: move |evt| {
                on_drag_leave.call(evt);
            },
            onmouseenter: move |evt| {
                if drag_active {
                    let placement =
                        drag_drop_placement_from_pointer(&row_for_enter, evt.element_coordinates().y, show_live_close);
                    on_drag_hover.call((placement, evt));
                }
            },
            onmousemove: move |evt| {
                if draggable && evt.held_buttons().contains(MouseButton::Primary) {
                    on_drag_move.call(evt.clone());
                }
                if drag_active {
                    let placement =
                        drag_drop_placement_from_pointer(&row_for_move, evt.element_coordinates().y, show_live_close);
                    on_drag_hover.call((placement, evt));
                }
            },
            onmouseup: move |evt| {
                evt.stop_propagation();
                if drag_active {
                    on_drop_into_row.call(());
                } else if evt.trigger_button() == Some(MouseButton::Primary) {
                    match sidebar_row_primary_click_action(
                        &row_for_root_mouseup,
                        SidebarRowPrimaryClickZone::ToggleSurface,
                    ) {
                        SidebarRowPrimaryClickAction::Select => {
                            on_select.call(tree_selection_mode_for_modifiers(evt.modifiers()));
                        }
                        SidebarRowPrimaryClickAction::ToggleExpanded => {
                            on_set_expanded.call(!row_for_root_mouseup.expanded);
                        }
                    }
                }
                on_end_drag.call(());
            },
            onkeydown: move |evt| {
                // Arrows open and shut anything that HOLDS rows — a folder, a
                // machine, or a row set's head. Enter stays a group's alone: on
                // a session row it belongs to opening the session.
                let holds_rows = row_is_group || row_is_row_set_head;
                if !holds_rows {
                    return;
                }
                if row_is_group && evt.key() == Key::Enter {
                    evt.prevent_default();
                    evt.stop_propagation();
                    on_select.call(TreeSelectionMode::Replace);
                } else if evt.key() == Key::ArrowRight {
                    evt.prevent_default();
                    evt.stop_propagation();
                    if !row_expanded {
                        on_set_expanded.call(true);
                    }
                } else if evt.key() == Key::ArrowLeft {
                    evt.prevent_default();
                    evt.stop_propagation();
                    if row_expanded {
                        on_set_expanded.call(false);
                    }
                }
            },
                // KeyTip markers for the two ROW-oriented commands. They are painted
                // on the row they act on, not on a proxy in the chrome: E on the row
                // the menu would open on, J on the Live Sessions row whose list jump
                // mode walks. Hidden spans — the badge painter measures their parent
                // (this row) and floats the letter there, so the row does not reflow
                // and the audit still sees the row as an exempt list-item.
                if !row_menu_tip.is_empty() {
                    span {
                        "data-keytip-node": keytip_node_id("session.menu"),
                        "data-keytip-tip": "{row_menu_tip}",
                        style: "display:none;",
                    }
                }
                if !jump_tip.is_empty() {
                    span {
                        "data-keytip-node": keytip_node_id("session.jump"),
                        "data-keytip-tip": "{jump_tip}",
                        style: "display:none;",
                    }
                }
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:6px;",
                    // ⛔ THE GUTTER — zone one, and a SIBLING of the content
                    // rather than its first child. That structure is the rule:
                    // a dot inside the content cluster moves whenever anything
                    // is inserted ahead of it or the cluster is indented, which
                    // is exactly how the first build of row sets put a header's
                    // dot to the right of its own members'. Out here it cannot.
                    if !show_live_close && row_is_group && input_unanswered {
                        // A GROUP holding a row that has stopped answering.
                        //
                        // ⛔ It goes in THE GUTTER, not beside the group's own
                        // dot, and the reason is a COLLISION rather than taste:
                        // a machine row's dot already spends this exact amber
                        // on `MachineHealth::Cached`
                        // (`machine_indicator_color_value`), so repainting it
                        // would make "this machine is a cached snapshot" and
                        // "something inside has gone deaf" the same pixel. The
                        // gutter is empty on every group row, is the column
                        // DESIGN.md reserves for "the status dot, nothing
                        // else", and lines this dot up with the session dots it
                        // is standing in for.
                        //
                        // STEADY, never blinking: blink means work is
                        // happening, and the whole claim here is that it has
                        // stopped.
                        span {
                            "data-sidebar-live-session-status-rail": "1",
                            style: session_row_dot_rail_style(SessionRowDensity::Sidebar),
                            span {
                                "data-sidebar-group-input-unanswered-dot": "1",
                                title: "A session inside has been typed to with no response — expand to find it",
                                style: live_session_status_dot_style_with_attention(palette, false, false, true),
                            }
                        }
                    }
                    if show_live_close {
                        span {
                            "data-sidebar-live-session-status-rail": "1",
                            style: session_row_dot_rail_style(SessionRowDensity::Sidebar),
                            // Status dot for EVERY live row (DESIGN.md "Status
                            // indicator vocabulary"): green = keep-alive,
                            // blue = lives with the GUI, blinking = working.
                            span {
                                "data-sidebar-live-session-status-dot": "1",
                                "data-sidebar-live-session-keep-alive": if row_kept_alive { "1" } else { "0" },
                                "data-sidebar-live-session-working": if busy_icon { "1" } else { "0" },
                                "data-sidebar-live-session-input-unanswered": if input_unanswered { "1" } else { "0" },
                                // The attention state OWNS the tooltip when it
                                // holds: the durability phrasings answer a
                                // question nobody is asking of a row that has
                                // stopped answering. It names the observation
                                // and the next step, and it does NOT say
                                // "wedged" — that is a verdict this signal is
                                // not entitled to make.
                                title: if input_unanswered {
                                    "Typed to, no response yet — may not be listening. Check with: server app terminal input-check"
                                } else {
                                    match (row_kept_alive, busy_icon) {
                                        (true, true) => "Keep-alive · working",
                                        (true, false) => "Keep alive",
                                        (false, true) => "Working",
                                        (false, false) => "Live (closes with the app)",
                                    }
                                },
                                style: live_session_status_dot_style_with_attention(palette, row_kept_alive, busy_icon, input_unanswered),
                            }
                        }
                    }
                    // `flex:1 1 auto` is the tree's half of "the title gets the
                    // width": with the ✕ out of flow there is nothing left to
                    // share the line with, so the label cluster takes all of it
                    // and truncates at the row's own edge instead of 24px short.
                    div {
                    style: "display:flex; align-items:center; gap:8px; min-width:0; flex:1 1 auto;",
                    div {
                        "data-tree-icon": "1",
                        "data-tree-icon-kind": icon_kind.as_str(),
                        "data-sidebar-row-toggle-target": if row_is_group { "icon" } else { "none" },
                        // The nesting lands HERE, inside the content zone — see
                        // `sidebar_row_indents`. `margin-left` rather than the
                        // row's padding is the whole point: everything from the
                        // icon rightwards steps, and the gutter, being a
                        // sibling of this cluster, cannot.
                        style: format!(
                            "{}margin-left:{}px;",
                            session_row_icon_box_style(SessionRowDensity::Sidebar, icon_color),
                            label_indent
                        ),
                        onmousedown: move |evt| {
                            if row_for_icon_mousedown.kind == BrowserRowKind::Group {
                                claim_sidebar_focus_by_path(Some(&icon_focus_path));
                                evt.prevent_default();
                                evt.stop_propagation();
                            }
                        },
                        onmouseup: move |evt| {
                            if row_for_icon_mouseup.kind != BrowserRowKind::Group {
                                return;
                            }
                            evt.prevent_default();
                            evt.stop_propagation();
                            if drag_active {
                                on_drop_into_row.call(());
                            } else if evt.trigger_button() == Some(MouseButton::Primary) {
                                on_set_expanded.call(!row_for_icon_mouseup.expanded);
                            }
                            on_end_drag.call(());
                        },
                        // Working is signalled by the BLINKING status dot
                        // (DESIGN.md "Status indicator vocabulary"); the row
                        // keeps its normal icon — the old busy circle icon is
                        // retired (user decision 2026-06-11).
                        if icon_kind == "claude-code" {
                            ClaudeCodeTreeIcon {}
                        } else {
                            TreeIcon { spec: tree_icon_spec(&row) }
                        }
                    }
                    if renaming {
                        div {
                            style: "flex:1; min-width:0; display:flex; align-items:center; gap:6px;",
                            input {
                                "data-tree-rename-input": "1",
                                "data-tree-rename-row-path": "{row.full_path}",
                                "data-tree-rename-row-depth": "{row.depth}",
                                "data-tree-rename-focused-once": if rename_focused_once { "true" } else { "false" },
                                "data-tree-rename-focus-pending": if rename_focus_pending { "true" } else { "false" },
                                style: format!(
                                    "flex:1; min-width:0; height:29px; border:none; border-radius:10px; background:rgba(255,255,255,0.92); \
                                     color:{}; font-size:12px; font-weight:600; padding:0 10px; box-shadow: inset 0 0 0 1px rgba(204,214,224,0.9);",
                                    palette.text
                                ),
                                // ⛔ `initial_value`, NEVER `value` — twin of the
                                // branch above; same volatile/stale-snapshot trap.
                                initial_value: rename_value,
                                onmounted: move |evt| async move {
                                    let _ = evt.set_focus(true).await;
                                },
                                oninput: move |evt| on_update_rename.call(evt.value()),
                                onfocus: move |_| on_focus_rename.call(()),
                                onblur: move |_| on_commit_rename.call(()),
                                onkeydown: move |evt| {
                                    evt.stop_propagation();
                                    if evt.key() == Key::Enter {
                                        evt.prevent_default();
                                        on_commit_rename.call(());
                                    } else if evt.key() == Key::Escape {
                                        evt.prevent_default();
                                        on_cancel_rename.call(());
                                    }
                                },
                                onclick: |evt| evt.stop_propagation(),
                                onmousedown: |evt| evt.stop_propagation(),
                                onmouseup: |evt| evt.stop_propagation(),
                                ondoubleclick: |evt| evt.stop_propagation(),
                                oncontextmenu: |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                },
                            }
                            if row.kind == BrowserRowKind::Session {
                                button {
                                    "data-sidebar-rename-ai-action": "title",
                                    title: "Use an AI-generated title",
                                    style: rename_ai_action_button_style(palette),
                                    onmousedown: |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                    },
                                    onclick: move |evt| on_regenerate_title.call(evt),
                                    AiSparkleIcon { size: 12 }
                                }
                            }
                        }
                    } else {
                        span {
                            "data-sidebar-row-label-target": "1",
                            style: session_row_label_style(
                                SessionRowDensity::Sidebar,
                                label_color,
                                row.kind == BrowserRowKind::Group && row.depth == 0,
                            ),
                            onmousedown: move |evt| {
                                claim_sidebar_focus_by_path(Some(&label_focus_path));
                                if draggable
                                    && evt.trigger_button() == Some(MouseButton::Primary)
                                    && !evt.modifiers().contains(Modifiers::SHIFT)
                                    && !evt.modifiers().contains(Modifiers::CONTROL)
                                    && !evt.modifiers().contains(Modifiers::META)
                                {
                                    on_start_drag.call(evt.clone());
                                }
                                evt.stop_propagation();
                            },
                            onmouseup: move |evt| {
                                evt.stop_propagation();
                                if drag_active {
                                    on_drop_into_row.call(());
                                } else if evt.trigger_button() == Some(MouseButton::Primary) {
                                    match sidebar_row_primary_click_action(
                                        &row_for_label_mouseup,
                                        SidebarRowPrimaryClickZone::Label,
                                    ) {
                                        SidebarRowPrimaryClickAction::Select => {
                                            on_select.call(tree_selection_mode_for_modifiers(evt.modifiers()));
                                        }
                                        SidebarRowPrimaryClickAction::ToggleExpanded => {
                                            on_set_expanded.call(!row_for_label_mouseup.expanded);
                                        }
                                    }
                                }
                                on_end_drag.call(());
                            },
                            "{visible_label}"
                        }
                    }
                    // The web-surface PROFILE badge (Phase 5): the shared row
                    // vocabulary's trailing pill. Only non-default profiles —
                    // the badge is what tells categorization browsers apart.
                    if !web_profile.is_empty() {
                        span {
                            "data-sidebar-web-profile": "{web_profile}",
                            title: "ychrome profile: {web_profile}",
                            style: session_row_badge_style(palette.accent),
                            "{web_profile}"
                        }
                    }
                    // Machine row status dot (issue #3): the OLD haloed
                    // "traffic-signal" indicator is replaced by the new flat-circle
                    // vocabulary (DESIGN.md) — color encodes reachability (green
                    // healthy / amber cached / red offline), and it BLINKS (the
                    // shared hard step-end pulse) when any session in the machine's
                    // subtree is working, exactly like a live-session row's dot.
                    if let Some(health) = machine_health {
                        span {
                            "data-machine-indicator": "1",
                            "data-machine-working": if busy_icon { "1" } else { "0" },
                            title: if busy_icon { "Working inside" } else { "" },
                            style: format!(
                                "display:inline-flex; width:7px; min-width:7px; height:7px; border-radius:999px; background:{};{}",
                                machine_indicator_color_value(health),
                                status_dot_blink_opacity_css(busy_icon)
                            ),
                        }
                    } else if row_is_group && busy_icon {
                        // Non-machine groups (cwd folders, the local root) have no
                        // health dot; surface their aggregate working state with the
                        // same blinking working dot the live rows use.
                        span {
                            "data-sidebar-group-working-dot": "1",
                            title: "Working inside",
                            style: live_session_status_dot_style(palette, false, true),
                        }
                    }
                }
                if show_live_close {
                    // IN FLOW on the shared anchor: the ✕ appears when the row
                    // is hovered, selected or holding the keyboard, takes its
                    // width from the title at that moment, and costs nothing at
                    // rest. Same reveal owner as every other row family — see
                    // the twin above for why the frosted chip this used to wear
                    // was rejected.
                    span {
                    "data-session-row-actions-anchor": "1",
                    // 6, because the cluster this anchor sits in is the row's
                    // own `gap:6px` line, not the shared metrics' 8.
                    style: session_row_actions_anchor_style(6),
                    span {
                    "data-session-row-actions": "1",
                    style: session_row_actions_style(5, 9),
                    // THE DISCLOSURE CONTROL ON A ROW SET'S HEAD — trailing,
                    // beside the ✕, and revealed by the same hover.
                    //
                    // ⛔ IT WAS LEADING FOR ONE BUILD AND THE OWNER CAUGHT IT
                    // IMMEDIATELY: a control inserted before the head's content
                    // pushes that row's dot, icon and label right, so a header
                    // sat FURTHER right than the rows nested under it, and the
                    // status dots stood in two different columns depending on
                    // whether a row happened to head a set. Sitting in the
                    // trailing actions costs the leading gutter nothing, which
                    // is what lets the dot column be identical at every depth.
                    if row_is_row_set_head {
                        button {
                            "data-sidebar-row-set-disclosure": "1",
                            "data-sidebar-row-set-expanded": if row_expanded { "true" } else { "false" },
                            "data-sidebar-row-set-members": "{row_set_member_count}",
                            // Per-element (§12.1), reason "list-item": one per
                            // head of an unbounded list (§8), exactly as the
                            // group expander is exempted.
                            "data-keytip-exempt": "list-item",
                            "data-sidebar-row-toggle-target": "expander",
                            title: if row_expanded { "Collapse this group of rows" } else { "Expand this group of rows" },
                            style: live_session_close_button_style(palette, selected),
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onmouseup: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: move |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                                on_set_expanded.call(row_toggle_target_expanded);
                            },
                            RowDisclosureChevron { expanded: row_expanded }
                        }
                    }
                    button {
                        "data-sidebar-live-session-close": "1",
                        // Per-element (§12.1), reason "list-item": a per-row
                        // affordance of an unbounded list (§8) — reached via the
                        // focused row's menu (ALT,E), never badged per row.
                        "data-keytip-exempt": "list-item",
                        title: "Close terminal",
                        style: live_session_close_button_style(palette, selected),
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        onclick: move |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                            on_delete_row.call(evt);
                        },
                        // Themed X close-icon (user decision 2026-06-26): an SVG
                        // stroke that inherits `currentColor` (the button color,
                        // theme-aware), not a filled "×" glyph pill. WHEN it is
                        // visible is session_row_hover_css's (shared with every
                        // session-style row); the hover "burn" tint is
                        // SIDEBAR_LIVE_CLOSE_CSS's.
                        svg {
                            width: "11",
                            height: "11",
                            view_box: "0 0 12 12",
                            fill: "none",
                            xmlns: "http://www.w3.org/2000/svg",
                            path {
                                d: "M3 3L9 9",
                                stroke: "currentColor",
                                stroke_width: "1.5",
                                stroke_linecap: "round",
                            }
                            path {
                                d: "M9 3L3 9",
                                stroke: "currentColor",
                                stroke_width: "1.5",
                                stroke_linecap: "round",
                            }
                        }
                    }
                    }
                    }
                } else if row.kind == BrowserRowKind::Group {
                    button {
                        "data-sidebar-group-expander": "1",
                        // Per-element (§12.1), reason "list-item": one per group
                        // row of an unbounded list (§8) — arrows expand/collapse
                        // the focused row, so badging every chevron would flood
                        // the overlay.
                        "data-keytip-exempt": "list-item",
                        "data-sidebar-row-toggle-target": "expander",
                        "data-sidebar-group-expanded": if row.expanded { "true" } else { "false" },
                        title: if row.expanded { "Collapse folder" } else { "Expand folder" },
                        style: format!(
                            "display:inline-flex; align-items:center; justify-content:center; gap:3px; min-width:30px; height:22px; \
                             border:none; border-radius:7px; background:transparent; color:{}; padding:0 3px; font-size:9.5px;",
                            palette.muted
                        ),
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        onmouseup: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        onclick: move |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                            on_set_expanded.call(row_toggle_target_expanded);
                        },
                        RowDisclosureChevron { expanded: row.expanded }
                        span {
                            style: "min-width:8px; text-align:right;",
                            "{row.descendant_sessions}"
                        }
                    }
                }
            }
            if row.kind == BrowserRowKind::Group
                && machine_health.is_none()
                && !row.detail_label.is_empty()
            {
                div {
                    style: format!(
                        "font-size:10px; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                        palette.muted
                    ),
                    "{row.detail_label}"
                }
            }
        }
    }
}
fn machine_health_from_label(label: &str) -> Option<MachineHealth> {
    if label.ends_with("[ok]") {
        Some(MachineHealth::Healthy)
    } else if label.ends_with("[cached]") {
        Some(MachineHealth::Cached)
    } else if label.ends_with("[offline]") {
        Some(MachineHealth::Offline)
    } else {
        None
    }
}
fn machine_label_text(label: &str) -> Option<String> {
    machine_health_from_label(label).map(|_| {
        label
            .rsplit_once(" [")
            .map(|(base, _)| base.to_string())
            .unwrap_or_else(|| label.to_string())
    })
}
#[component]
fn DragGhost(snapshot: SharedSnapshot) -> Element {
    let Some((x, y)) = snapshot.drag_pointer else {
        return rsx! {};
    };
    let dragged_rows = snapshot
        .rows
        .iter()
        .filter(|row| {
            snapshot
                .drag_paths
                .iter()
                .any(|path| path == &row.full_path)
        })
        .cloned()
        .collect::<Vec<_>>();
    if dragged_rows.is_empty() {
        return rsx! {};
    }
    let primary_label = dragged_rows
        .first()
        .map(|row| row.label.clone())
        .unwrap_or_else(|| "Move item".to_string());
    let extra_count = dragged_rows.len().saturating_sub(1);
    let drop_target_hint = snapshot.drag_hover_target.as_ref().map(|target| {
        let placement = match target.placement {
            DragDropPlacement::Before => "before",
            DragDropPlacement::Into => "inside",
            DragDropPlacement::After => "after",
        };
        let leaf = workspace_leaf_name(&target.path).unwrap_or_else(|| "item".to_string());
        format!("Drop {placement} {leaf}")
    });
    rsx! {
        DragGhostCard {
            x: x,
            y: y,
            primary_label: primary_label,
            extra_count: extra_count,
            target_hint: drop_target_hint,
            palette: DragGhostPalette {
                text: snapshot.palette.text,
                muted: snapshot.palette.muted,
                accent: snapshot.palette.accent,
                accent_soft: snapshot.palette.accent_soft,
            },
        }
    }
}
/// What the drag ghost says about where the release would land — the same
/// sentence the cwd tree's ghost has always shown ("Drop inside Work"), from
/// one owner so a row list and the tree cannot phrase it two ways.
fn row_drop_target_hint(target: &RowDropTarget) -> String {
    let placement = match target.placement {
        DragDropPlacement::Before => "before",
        DragDropPlacement::Into => "inside",
        DragDropPlacement::After => "after",
    };
    let name = if target.label.trim().is_empty() {
        target.row_id.as_str()
    } else {
        target.label.trim()
    };
    format!("Drop {placement} {name}")
}
#[component]
fn ClaudeCodeTreeIcon() -> Element {
    rsx! {
        svg {
            width: "19",
            height: "15",
            view_box: "0 0 19 15",
            fill: "none",
            xmlns: "http://www.w3.org/2000/svg",
            rect { x: "1.6", y: "1.7", width: "15.8", height: "11.6", rx: "2.2", stroke: "currentColor", stroke_width: "1.15" }
            text {
                x: "7.0",
                y: "10.2",
                text_anchor: "middle",
                fill: "currentColor",
                style: "font-family:'JetBrains Mono', ui-monospace, monospace; font-size:10px; font-weight:800; letter-spacing:0;",
                "*"
            }
            text {
                x: "14.0",
                y: "10.0",
                text_anchor: "middle",
                fill: "currentColor",
                style: "font-family:'JetBrains Mono', ui-monospace, monospace; font-size:7px; font-weight:800; letter-spacing:0;",
                "_"
            }
        }
    }
}
/// Resolved icon decision for a sidebar row — the small, cheap-to-compare key
/// that drives [`TreeIcon`].
///
/// PERF: `TreeIcon` is a memoized `#[component]`; passing the whole `BrowserRow`
/// made its prop-memo a full `BrowserRow` PartialEq (memcmp over many `String`s)
/// for every one of ~223 rows, every render, AND invalidated the icon whenever
/// any unrelated field changed (label/title/detail churn on working turns). The
/// icon only depends on six fields, so we collapse the decision into this Copy
/// enum once per row. The memo compare is now a trivial enum match and the icon
/// only re-renders when the glyph itself actually changes. See
/// [[finding-gui-latency-render-path-campaign]] (profiled hot stack:
/// `TreeIconProps::memoize -> <BrowserRow as PartialEq>::eq -> memcmp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeIconSpec {
    ClaudeCode,
    BoxedGlyph(&'static str),
    Document,
    TerminalRecipe,
    RootLiveSessions,
    RootGroup,
    FolderOpen,
    FolderClosed,
}

/// Single owner of the sidebar icon decision tree. Mirrors the branch order in
/// [`TreeIcon`] exactly — reads only `kind`, `session_kind`, `full_path`,
/// `document_kind`, `depth`, and `expanded`.
fn tree_icon_spec(row: &BrowserRow) -> TreeIconSpec {
    if row_session_kind(row) == Some(SessionKind::ClaudeCode) {
        return TreeIconSpec::ClaudeCode;
    }
    if let Some(glyph) = tree_icon_glyph(row) {
        return TreeIconSpec::BoxedGlyph(glyph);
    }
    if row.kind == BrowserRowKind::Document {
        if row.document_kind == Some(WorkspaceDocumentKind::TerminalRecipe) {
            return TreeIconSpec::TerminalRecipe;
        }
        return TreeIconSpec::Document;
    }
    if row.depth == 0 {
        if row.full_path == "__live_sessions__" {
            return TreeIconSpec::RootLiveSessions;
        }
        return TreeIconSpec::RootGroup;
    }
    if row.expanded {
        TreeIconSpec::FolderOpen
    } else {
        TreeIconSpec::FolderClosed
    }
}

/// The `<text>` style for a boxed glyph, sized so the mark fits the SAME rect
/// every session-kind icon draws.
///
/// The rect's inner width is 15.8 px and JetBrains Mono advances ≈0.6 em, so at
/// 7 px a character costs ≈4.2 px: two characters (`>_`, `$_`, `K_`, `π_`) sit
/// with ≈3.7 px of air each side, which is the shipped look. Three (`OC_`) would
/// leave 1.6 px and read visibly cramped beside its neighbours, so it steps down
/// a point instead.
///
/// ⛔ The answer is never "widen the rect": the rect is what makes every row's
/// icon one family, and a wider one on a single CLI is the thing a user's eye
/// catches immediately (`DESIGN.md` §sidebar iconography).
fn boxed_glyph_text_style(glyph: &str) -> &'static str {
    const SEVEN: &str = "font-family:'JetBrains Mono', ui-monospace, monospace; \
                         font-size:7px; font-weight:800; letter-spacing:0;";
    const SIX: &str = "font-family:'JetBrains Mono', ui-monospace, monospace; \
                       font-size:6px; font-weight:800; letter-spacing:0;";
    if glyph.chars().count() >= 3 { SIX } else { SEVEN }
}

#[component]
fn TreeIcon(spec: TreeIconSpec) -> Element {
    if spec == TreeIconSpec::ClaudeCode {
        return rsx! { ClaudeCodeTreeIcon {} };
    }
    if let TreeIconSpec::BoxedGlyph(glyph) = spec {
        let style = boxed_glyph_text_style(glyph);
        return rsx! {
            svg {
                width: "19",
                height: "15",
                view_box: "0 0 19 15",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect { x: "1.6", y: "1.7", width: "15.8", height: "11.6", rx: "2.2", stroke: "currentColor", stroke_width: "1.15" }
                text {
                    x: "9.5",
                    y: "9.8",
                    text_anchor: "middle",
                    fill: "currentColor",
                    style: "{style}",
                    "{glyph}"
                }
            }
        };
    }
    if spec == TreeIconSpec::Document || spec == TreeIconSpec::TerminalRecipe {
        if spec == TreeIconSpec::TerminalRecipe {
            return rsx! {
                svg {
                    width: "19",
                    height: "19",
                    view_box: "0 0 18 18",
                    fill: "none",
                    xmlns: "http://www.w3.org/2000/svg",
                    rect { x: "3.2", y: "3.2", width: "11.6", height: "11.6", rx: "2.2", stroke: "currentColor", stroke_width: "1.15" }
                    path { d: "M6 7.2L8 9L6 10.8", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                    path { d: "M9.5 10.8H12", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round" }
                }
            };
        }
        return rsx! {
            svg {
                width: "19",
                height: "19",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect { x: "4", y: "2.75", width: "10", height: "12.5", rx: "1.6", stroke: "currentColor", stroke_width: "1.15" }
                path { d: "M6.4 6.5H11.6", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
                path { d: "M6.4 9H11.6", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
                path { d: "M6.4 11.5H10.2", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
            }
        };
    }
    if spec == TreeIconSpec::RootLiveSessions || spec == TreeIconSpec::RootGroup {
        if spec == TreeIconSpec::RootLiveSessions {
            return rsx! {
                span {
                    style: "display:inline-flex; align-items:center; justify-content:center; font-size:12px; font-weight:700; line-height:1;",
                    "◉"
                }
            };
        }
        return rsx! {
            svg {
                width: "19",
                height: "19",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect {
                    x: "2.75",
                    y: "3.25",
                    width: "12.5",
                    height: "8.5",
                    rx: "1.4",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                }
                path {
                    d: "M6.2 14.1H11.8",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linecap: "round",
                }
                path {
                    d: "M9 11.95V14.05",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linecap: "round",
                }
            }
        };
    }
    rsx! { RowFolderIcon { expanded: spec == TreeIconSpec::FolderOpen } }
}

/// The tree's FOLDER GLYPH — the one icon in the product that says "this row
/// holds other rows", and whose FILL says whether it is open.
///
/// FILLED = expanded, OUTLINE = collapsed. That is the cwd tree's own answer
/// and it is now the only one: the WebTabs rail's folders and a contributed
/// pane's groups draw this same component, so a folder three pixels from a
/// cwd-tree folder cannot look like a different kind of thing (user-reported
/// 2026-07-31, "just like yggterm cwdtree").
///
/// The DISCLOSURE CHEVRON survives alongside it, also the cwd tree's answer: a
/// group row carries the folder glyph in its leading icon slot and a chevron in
/// its trailing expander. The fill is the state at rest; the chevron is the
/// control.
#[component]
fn RowFolderIcon(expanded: bool) -> Element {
    if expanded {
        rsx! {
            svg {
                width: "19",
                height: "19",
                view_box: "0 0 16 16",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M1.9 5.2C1.9 4.59249 2.39249 4.1 3 4.1H6.35L7.6 5.35H13C13.6075 5.35 14.1 5.84249 14.1 6.45V11.8C14.1 12.4075 13.6075 12.9 13 12.9H3C2.39249 12.9 1.9 12.4075 1.9 11.8V5.2Z",
                    fill: "currentColor",
                    fill_opacity: "0.84",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M2.4 6.25H14.05",
                    stroke: "currentColor",
                    stroke_width: "0.95",
                    stroke_opacity: "0.18",
                }
            }
        }
    } else {
        rsx! {
            svg {
                width: "19",
                height: "19",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M2.1 5.45C2.1 4.78726 2.63726 4.25 3.3 4.25H6.65L7.95 5.55H14.05C14.7127 5.55 15.25 6.08726 15.25 6.75V12.05C15.25 12.7127 14.7127 13.25 14.05 13.25H3.3C2.63726 13.25 2.1 12.7127 2.1 12.05V5.45Z",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M2.6 6.5H14.75",
                    stroke: "currentColor",
                    stroke_width: "1.0",
                    stroke_opacity: "0.42",
                }
            }
        }
    }
}
#[component]
fn BellIcon() -> Element {
    rsx! {
        svg {
            width: "15",
            height: "15",
            view_box: "0 0 16 16",
            fill: "none",
            xmlns: "http://www.w3.org/2000/svg",
            path {
                d: "M8 2.4C6.18 2.4 4.95 3.73 4.95 5.56V6.42C4.95 7.11 4.67 7.95 4.31 8.53L3.45 9.96C3.1 10.55 3.39 11.2 4.03 11.2H11.97C12.61 11.2 12.9 10.55 12.55 9.96L11.69 8.53C11.33 7.95 11.05 7.11 11.05 6.42V5.56C11.05 3.73 9.82 2.4 8 2.4Z",
                stroke: "currentColor",
                stroke_width: "1.15",
                stroke_linejoin: "round",
            }
            path {
                d: "M6.65 12.55C6.9 13.24 7.39 13.58 8 13.58C8.61 13.58 9.1 13.24 9.35 12.55",
                stroke: "currentColor",
                stroke_width: "1.15",
                stroke_linecap: "round",
            }
        }
    }
}
/// Width of the gap between split pane rects. The gutter is load-bearing, not
/// cosmetic: it is the ONLY strip where the divider line and the focus ring can
/// live once split members include web panes — a native webview paints above
/// all DOM inside its rect, so nothing drawn INSIDE a pane rect is guaranteed
/// visible. Half the gutter is carved off each pane at the seam.
const SPLIT_GUTTER_PX: u32 = 6;

/// CSS geometry (`position`/`left`/`top`/`width`/`height`) for pane `index` of
/// a 2-pane split ([[campaign-split-view-groups]] MVP). `ratio` is the fraction
/// the first pane occupies along the split axis; both panes shrink toward the
/// seam by half of SPLIT_GUTTER_PX. Index 0 is the first pane; any other index
/// is treated as the second — 2×2 (drop-onto-cell) is phase 2.
fn split_pane_rect_css(axis: SplitAxis, ratio: f32, index: usize) -> String {
    let ratio = ratio.clamp(0.05, 0.95);
    let first_pct = ratio * 100.0;
    let second_pct = 100.0 - first_pct;
    let half = SPLIT_GUTTER_PX / 2;
    match (axis, index) {
        (SplitAxis::SideBySide, 0) => {
            format!("position:absolute; left:0; top:0; width:calc({first_pct:.4}% - {half}px); height:100%;")
        }
        (SplitAxis::SideBySide, _) => {
            format!("position:absolute; left:calc({first_pct:.4}% + {half}px); top:0; width:calc({second_pct:.4}% - {half}px); height:100%;")
        }
        (SplitAxis::Stacked, 0) => {
            format!("position:absolute; left:0; top:0; width:100%; height:calc({first_pct:.4}% - {half}px);")
        }
        (SplitAxis::Stacked, _) => {
            format!("position:absolute; left:0; top:calc({first_pct:.4}% + {half}px); width:100%; height:calc({second_pct:.4}% - {half}px);")
        }
    }
}

/// The JS that drives a live divider drag ([[campaign-split-view-groups]]).
/// Doing the geometry in JS avoids the GUI needing the container's pixel size
/// and keeps the drag smooth: JS resizes the pane elements AND the divider in
/// place on every move (no per-frame Rust re-render / disk write), then posts
/// the final ratio ONCE on release so Rust commits+persists exactly once. Panes
/// carry `data-split-pane-index`; the container carries `data-split-layer`; the
/// divider carries `data-split-divider`.
fn split_divider_drag_script(axis: SplitAxis, ratio: f32) -> String {
    let vertical = matches!(axis, SplitAxis::Stacked);
    let half = SPLIT_GUTTER_PX / 2;
    format!(
        r#"
        (async () => {{
            const layer = document.querySelector('[data-split-layer]');
            if (!layer) {{ dioxus.send(-1); return; }}
            const first = layer.querySelector('[data-split-pane-index="0"]');
            const second = layer.querySelector('[data-split-pane-index="1"]');
            const divider = layer.querySelector('[data-split-divider]');
            const vertical = {vertical};
            let ratio = {ratio};
            const clamp = (v) => Math.max(0.15, Math.min(0.85, v));
            const apply = (r) => {{
                const pct = (r * 100).toFixed(3) + '%';
                const rest = ((1 - r) * 100).toFixed(3) + '%';
                const firstEdge = 'calc(' + pct + ' - {half}px)';
                const secondEdge = 'calc(' + pct + ' + {half}px)';
                const restSize = 'calc(' + rest + ' - {half}px)';
                if (first) {{
                    if (vertical) {{ first.style.height = firstEdge; first.style.top = '0px'; }}
                    else {{ first.style.width = firstEdge; first.style.left = '0px'; }}
                }}
                if (second) {{
                    if (vertical) {{ second.style.height = restSize; second.style.top = secondEdge; }}
                    else {{ second.style.width = restSize; second.style.left = secondEdge; }}
                }}
                if (divider) {{
                    const seam = 'calc(' + pct + ' - {half}px)';
                    if (vertical) {{ divider.style.top = seam; }}
                    else {{ divider.style.left = seam; }}
                }}
            }};
            const onMove = (ev) => {{
                const rect = layer.getBoundingClientRect();
                const size = vertical ? rect.height : rect.width;
                if (size <= 0) {{ return; }}
                const pos = vertical ? (ev.clientY - rect.top) : (ev.clientX - rect.left);
                ratio = clamp(pos / size);
                apply(ratio);
                if (ev.preventDefault) {{ ev.preventDefault(); }}
            }};
            const onUp = () => {{
                window.removeEventListener('pointermove', onMove, true);
                window.removeEventListener('pointerup', onUp, true);
                window.removeEventListener('pointercancel', onUp, true);
                dioxus.send(ratio);
            }};
            window.addEventListener('pointermove', onMove, true);
            window.addEventListener('pointerup', onUp, true);
            window.addEventListener('pointercancel', onUp, true);
        }})();
        "#
    )
}

