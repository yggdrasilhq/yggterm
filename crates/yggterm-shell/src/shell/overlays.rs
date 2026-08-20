/// Cursor v1 (`docs/agent-control-plane.md` slice 3): the pointers of agents
/// working the session the user is currently viewing, each a coloured arrow
/// tagged `agent-N`.
///
/// The whole of v1 is "show me where the other driver is". There is no
/// co-presence toggle, no ghost-cursor mimicry, and no visibility modes — those
/// were explicitly parked. The caller has already filtered to this session and
/// to live pointers, so this component only draws.
#[component]
fn AgentCursorOverlay(cursors: Vec<AgentPointer>) -> Element {
    rsx! {
        style { "{AGENT_CURSOR_CSS}" }
        div {
            "data-yggterm-agent-cursors": "1",
            // Above the chrome so the pointer is never buried, below the click
            // grid's 2147483000 so an agent's own aiming overlay still wins.
            // pointer-events:none — presence is information, never a hit target.
            style: "position:fixed; inset:0; z-index:210; pointer-events:none;",
            for cursor in cursors.iter() {
                div {
                    key: "agent-cursor-{cursor.index}",
                    // Every branch of this component emits the SAME style keys.
                    // Dioxus applies `style` property-by-property and never
                    // clears a key a later render drops, so a conditional key
                    // would linger as a ghost (see the sidebar overlay trap).
                    style: format!(
                        "position:absolute; left:{x}px; top:{y}px; display:flex; align-items:flex-start; \
                         gap:4px; transform:translate(-2px, -2px); will-change:opacity; \
                         animation:yggterm-agent-cursor-fade {ttl}ms linear forwards;",
                        x = cursor.x,
                        y = cursor.y,
                        ttl = AGENT_CURSOR_TTL_MS,
                    ),
                    // The arrow. An inline SVG rather than a glyph so the
                    // silhouette is identical on every platform's font stack.
                    svg {
                        width: "18",
                        height: "18",
                        view_box: "0 0 18 18",
                        path {
                            d: "M2 1 L2 14 L5.6 10.6 L8 15.6 L10.4 14.4 L8 9.6 L13 9.6 Z",
                            fill: "{cursor.color()}",
                            stroke: "rgba(0,0,0,0.55)",
                            stroke_width: "1",
                            stroke_linejoin: "round",
                        }
                    }
                    span {
                        style: format!(
                            "margin-top:11px; padding:1px 6px; border-radius:6px; font:700 11px \
                             ui-monospace, monospace; white-space:nowrap; color:#0b0b0b; \
                             background:{color}; box-shadow:0 1px 3px rgba(0,0,0,0.45);",
                            color = cursor.color(),
                        ),
                        "{cursor.tag()} {cursor.action}"
                    }
                }
            }
        }
    }
}

#[component]
fn ContextMenuOverlay(
    position: (f64, f64),
    window_size: (f64, f64),
    palette: Palette,
    /// The menu's contents — [`row_menu_items`], resolved once in the snapshot and
    /// shared with the ALT layer's `rowmenu` scope. This component draws exactly
    /// this list; it decides NOTHING about what the menu contains.
    items: Vec<RowMenuItem>,
    menu_title: String,
    /// The chrome region this menu must be drawn INSIDE, when that is narrower
    /// than the window.
    ///
    /// Passed by the mounts whose menus are raised from a band of chrome that
    /// neighbours a NATIVE web surface — the WebTabs rail's rows, the rail's
    /// profile badge, a contributed app-pane row — because in legacy stacking
    /// the page composites above any DOM that spills out of the band. `None` for
    /// the mounts that live inside DOM-owned regions (the cwd tree's row menu,
    /// the classic strip's dropdown, whose answer is the stash instead), which
    /// keep window clamping.
    ///
    /// The MOUNT decides, never a global sniff: only the mount knows which
    /// chrome raised the menu, and a sniff would silently re-band the tree's
    /// menu the day a web surface happens to be open.
    band: Option<ContextMenuBand>,
    /// The resolved KeyTip tree and the chord typed so far, so each item paints
    /// the same letter the chord walker would act on (one assignment, two views).
    keytip_tree: KeyTipTree,
    alt_overlay_active: bool,
    alt_overlay_sequence: String,
    /// §4 scope root, when this mount IS a top MODAL (`render_top_modal`): the
    /// classic strip's profile dropdown passes `"strip-dropdown"` so the
    /// overlay-open walk confines derivation to this menu while it is up.
    /// `None` (every other mount) stamps nothing that matches a modal kind.
    modal_root: Option<String>,
    on_close: EventHandler<MouseEvent>,
    /// Run the item with this id. One terminus for the mouse and the ALT layer
    /// alike ([`dispatch_row_menu_action`]) — neither can reach an action the
    /// other cannot.
    on_action: EventHandler<String>,
) -> Element {
    // ONE width, read by the placement and by the box that is drawn.
    let menu_width = context_menu_width(band);
    // Asked once for the whole list, so every row of one menu agrees.
    let menu_has_icons = context_menu_has_icons(&items);
    let placement = context_menu_placement(
        position,
        window_size,
        (menu_width, CONTEXT_MENU_HEIGHT_PX),
        band,
    );
    let placement_style = context_menu_position_style(placement);
    let menu_blur = overlay_backdrop_style("blur(20px) saturate(150%)");
    rsx! {
        div {
            // z-index ABOVE the auto-hidden sidebar overlay (`SIDEBAR_AUTOHIDE_Z_INDEX`
            // = 170) and the titlebar (180): a row context menu is raised FROM the
            // sidebar, so it must float over it. At the old 90 it rendered BEHIND a
            // revealed floating sidebar and was invisible/unclickable (user report
            // 2026-07-21).
            //
            // `pointer-events:auto` is what makes this a DISMISS surface rather
            // than decoration. While it was `none` the click-outside handler
            // below could never fire — the pointer fell straight through to
            // whatever was underneath — and the app's only real outside-click
            // dismissal was a terminal-host JS hack that fired for clicks inside
            // the terminal rect, closed only the cwd tree's menu, and went away
            // entirely while that host was blank or remounting.
            "data-yggterm-menu-backdrop": "1",
            style: "position:fixed; inset:0; z-index:200; background:transparent; pointer-events:auto;",
            // The whole press/release/click triple is consumed here, so the
            // first outside click is a dismissal and nothing else: hit testing
            // stops at this node, and a button underneath needs a mousedown AND
            // a mouseup on itself before it can raise a click — it gets neither.
            // `stop_propagation` then keeps the gesture off the shell ancestors
            // this overlay is nested inside.
            //
            // Dismissal rides `onclick`, not `onmousedown`. Closing on the press
            // unmounts this node mid-gesture; the release then lands elsewhere
            // and the browser raises `click` on the nearest common ancestor of
            // the two targets — a node that is not this backdrop and whose
            // handlers were never part of the gesture.
            onmousedown: move |evt: MouseEvent| {
                evt.stop_propagation();
            },
            onmouseup: move |evt: MouseEvent| {
                evt.stop_propagation();
            },
            onclick: move |evt: MouseEvent| {
                on_close.call(evt.clone());
                evt.stop_propagation();
            },
            // A right-click outside dismisses as well, and takes the native
            // WebKit menu with it — a menu that dies leaving another menu in its
            // place has not died.
            oncontextmenu: move |evt: MouseEvent| {
                evt.prevent_default();
                on_close.call(evt.clone());
                evt.stop_propagation();
            },
            div {
                "data-context-menu": "1",
                "data-yggterm-menu-surface": "1",
                "data-yggterm-modal-root": modal_root.clone().unwrap_or_default(),
                style: format!("{} pointer-events:auto;", context_menu_surface_style(palette, &placement_style, menu_blur, menu_width, placement.max_height)),
                onmousedown: |evt| evt.stop_propagation(),
                onmouseup: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                // The surface holds back the backdrop's right-click dismissal
                // too: a right-click ON the menu is inside it, and the native
                // menu has no business opening over the app's own.
                oncontextmenu: |evt: MouseEvent| {
                    evt.prevent_default();
                    evt.stop_propagation();
                },
                // A HEADING ONLY WHEN IT EARNS ITS PLACE. A menu raised ON a row
                // is drawn directly under that row, which is highlighted and
                // already says its own name — repeating it stacks the same words
                // twice (the user's screenshot) and costs a line of a box that
                // is only ~216px wide. An empty title is a mount saying "the
                // thing this acts on is right there"; the mounts that DO pass
                // one are saying something the row cannot — which page you are
                // on, how many rows are selected, or which surface this is.
                if !menu_title.trim().is_empty() {
                    div {
                        style: format!("padding:6px 12px 8px 12px; font-size:11px; font-weight:700; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.muted),
                        "{menu_title}"
                    }
                }
                for item in items.iter().cloned() {
                    if item.separator {
                        div {
                            key: "sep-{item.id}-{item.label}",
                            style: format!("height:1px; margin:6px 4px; background:{}; opacity:0.7;", palette.border),
                        }
                    } else {
                        button {
                            // The ICON COLUMN, drawn only for a menu that opted
                            // in (`context_menu_has_icons`). Every row of such a
                            // menu reserves the slot, including the rows with no
                            // mark, so the labels keep one left edge. The mark
                            // is stroked in `currentColor`, so it inherits the
                            // row's tone — destructive red, dimmed grey — rather
                            // than carrying colour of its own (DESIGN.md ▸ Tree
                            // behavior: restrained and mostly grayscale).
                            key: "item-{item.id}",
                            "data-context-menu-action": "{item.id}",
                            "data-context-menu-disabled": "{item.disabled}",
                            // WHY it is inert, or the whole label when the box
                            // had to ellipsize it. Never in the label itself.
                            title: item.tooltip(),
                            class: "yggterm-menu-item",
                            // ONE style owner (`context_menu_item_style`) routes
                            // every branch through the shared style engine, so
                            // all branches emit IDENTICAL keys (the Dioxus
                            // property-by-property trap: a dropped key never
                            // clears).
                            style: context_menu_item_style(palette, &item),
                            onmousedown: |evt| evt.stop_propagation(),
                            onclick: {
                                let item = item.clone();
                                let on_action = on_action;
                                move |evt: MouseEvent| {
                                    evt.stop_propagation();
                                    // A disabled item swallows the click and
                                    // leaves the menu open: the reason is in the
                                    // label, and dismissing on a refusal would
                                    // hide it the instant the user asked. The
                                    // dispatch owner is `context_menu_click_action`
                                    // — an inert item yields no id, so there is
                                    // nothing to call `on_action` with. The
                                    // keyboard's half of the same refusal is in
                                    // `build_keytip_scopes`, which never DECLARES
                                    // a disabled item.
                                    let Some(id) = context_menu_click_action(&item) else {
                                        return;
                                    };
                                    on_action.call(id);
                                }
                            },
                            span {
                                "data-keytip-node": keytip_node_id(&row_menu_node_key(&item.id)),
                                "data-keytip-tip": keytip_tip_for(
                                    &keytip_tree,
                                    alt_overlay_active,
                                    &alt_overlay_sequence,
                                    &row_menu_node_key(&item.id),
                                ),
                                style: "display:none;",
                            }
                            if menu_has_icons {
                                span {
                                    "data-context-menu-icon": item.icon.is_some(),
                                    style: "flex:0 0 auto; display:inline-flex; align-items:center; justify-content:center; width:16px; height:16px; margin-right:9px;",
                                    if let Some(icon) = item.icon {
                                        svg {
                                            width: "14",
                                            height: "14",
                                            view_box: "0 0 14 14",
                                            fill: "none",
                                            for d in icon.paths() {
                                                path {
                                                    key: "{d}",
                                                    d: "{d}",
                                                    stroke: "currentColor",
                                                    stroke_width: "1.25",
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            span {
                                style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis;",
                                "{item.display_label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}
fn selected_paths_are_live_sessions(
    session_paths: &[String],
    live_sessions: &[ManagedSessionView],
) -> bool {
    if session_paths.is_empty() {
        return false;
    }
    let live_paths = live_sessions
        .iter()
        .map(|session| session.session_path.as_str())
        .collect::<HashSet<_>>();
    session_paths
        .iter()
        .all(|path| live_paths.contains(path.as_str()))
}
fn delete_confirm_dialog_text(pending: &PendingDeleteDialog) -> DeleteConfirmDialogText {
    let item_count = pending_delete_display_count(pending);
    let deleting_ssh_targets = !pending.ssh_machine_keys.is_empty()
        && pending.document_paths.is_empty()
        && pending.group_paths.is_empty()
        && pending.session_paths.is_empty();
    let deleting_sessions = !pending.session_paths.is_empty()
        && pending.document_paths.is_empty()
        && pending.group_paths.is_empty()
        && pending.ssh_machine_keys.is_empty();
    if pending.hard_delete {
        return DeleteConfirmDialogText {
            title: "Delete Permanently?".to_string(),
            copy: if deleting_ssh_targets {
                "This will permanently remove the selected SSH targets from the sidebar."
            } else if deleting_sessions {
                "This will permanently remove stored session files when available and close live terminal runtimes for those sessions."
            } else {
                "This will permanently remove the selected items from the workspace tree."
            },
            action_label: "Delete Permanently".to_string(),
        };
    }
    if pending.live_session_bulk_close {
        return DeleteConfirmDialogText {
            title: "Close Live Sessions?".to_string(),
            copy: "This closes live terminal runtimes. Stored session metadata and transcripts remain. Close All Un Keep-Alive Sessions leaves Keep Alive sessions running.",
            action_label: "Close All Sessions".to_string(),
        };
    }
    if pending.live_session_close {
        return DeleteConfirmDialogText {
            title: if item_count == 1 {
                "Close Terminal?".to_string()
            } else {
                "Close Terminals?".to_string()
            },
            copy: "This closes the selected live terminal runtime. Stored session metadata and transcripts remain. Keep Alive preserves terminals only when the Yggterm window closes.",
            action_label: if item_count == 1 {
                "Close Terminal".to_string()
            } else {
                "Close Terminals".to_string()
            },
        };
    }
    DeleteConfirmDialogText {
        title: "Delete Selected Items?".to_string(),
        copy: if deleting_ssh_targets {
            "This will remove the selected SSH targets from the sidebar. Hold Shift while pressing Delete to skip this dialog."
        } else if deleting_sessions {
            "This will remove the selected session rows from the sidebar. Hold Shift while pressing Delete to permanently remove stored session files when available."
        } else {
            "This will remove the selected items from the workspace tree. Hold Shift while pressing Delete to skip this dialog."
        },
        action_label: "Delete".to_string(),
    }
}
fn pending_delete_display_count(pending: &PendingDeleteDialog) -> usize {
    let direct_count = pending.document_paths.len()
        + pending.group_paths.len()
        + pending.session_paths.len()
        + pending.ssh_machine_keys.len();
    direct_count.max(pending.labels.len())
}
fn delete_success_notification_text(
    pending: &PendingDeleteDialog,
    affected: usize,
) -> (&'static str, String) {
    if pending.live_session_close {
        return (
            if affected == 1 {
                "Terminal Closed"
            } else {
                "Terminals Closed"
            },
            if affected == 1 {
                "Closed the terminal runtime. Stored session metadata and transcripts remain."
                    .to_string()
            } else {
                format!(
                    "Closed {affected} live terminal runtimes. Stored session metadata and transcripts remain."
                )
            },
        );
    }
    ("Items Deleted", format!("Removed {affected} item(s)."))
}
#[component]
fn CopyEditOverlay(
    dialog: CopyEditDialog,
    palette: Palette,
    on_change: EventHandler<String>,
    on_cancel: EventHandler<MouseEvent>,
    on_save: EventHandler<MouseEvent>,
) -> Element {
    let is_summary = dialog.field == CopyEditField::Summary;
    let title = if is_summary {
        "Edit Summary"
    } else {
        "Edit Title"
    };
    let input_label = if is_summary { "Summary" } else { "Title" };
    let overlay_blur = overlay_backdrop_style("blur(18px) saturate(130%)");
    rsx! {
        div {
            "data-copy-edit-overlay": "1",
            // §4 scope root — see the delete overlay's stamp.
            "data-yggterm-modal-root": "copy-edit",
            style: format!(
                "position:fixed; inset:0; z-index:96; display:flex; align-items:center; justify-content:center; \
                 background:rgba(230,239,248,0.26); backdrop-filter:{}; -webkit-backdrop-filter:{};",
                overlay_blur,
                overlay_blur
            ),
            onclick: move |evt| on_cancel.call(evt),
            div {
                "data-copy-edit-dialog": "1",
                style: format!(
                    "width:min(520px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:14px; \
                     padding:20px; border-radius:16px; background:rgba(250,252,255,0.97); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; flex-direction:column; gap:5px; min-width:0;",
                    div {
                        "data-copy-edit-title": "1",
                        style: format!("font-size:17px; font-weight:800; color:{};", palette.text),
                        "{title}"
                    }
                    div {
                        style: format!("font-size:12px; line-height:1.5; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", palette.muted),
                        "{dialog.title}"
                    }
                }
                label {
                    style: "display:flex; flex-direction:column; gap:7px;",
                    span {
                        style: format!("font-size:11px; font-weight:800; color:{}; text-transform:uppercase; letter-spacing:0;", palette.muted),
                        "{input_label}"
                    }
                    if is_summary {
                        textarea {
                            "data-copy-edit-input": "summary",
                            // §12.4 clause 1 — the keyboard arrives in the
                            // field, not on Cancel: typing is why the dialog opened.
                            "data-yggterm-modal-autofocus": "1",
                            value: "{dialog.value}",
                            style: format!(
                                "min-height:132px; resize:vertical; border:none; border-radius:8px; padding:11px 12px; \
                                 background:{}; color:{}; box-shadow:inset 0 0 0 1px rgba(120,142,166,0.18); \
                                 font-size:13px; line-height:1.55; font-family:{}; outline:none;",
                                palette.panel_alt,
                                palette.text,
                                interface_font_family()
                            ),
                            oninput: move |evt| on_change.call(evt.value()),
                        }
                    } else {
                        input {
                            "data-copy-edit-input": "title",
                            // §12.4 clause 1 — the keyboard arrives in the
                            // field, not on Cancel: typing is why the dialog opened.
                            "data-yggterm-modal-autofocus": "1",
                            r#type: "text",
                            value: "{dialog.value}",
                            style: format!(
                                "height:38px; border:none; border-radius:8px; padding:0 12px; background:{}; color:{}; \
                                 box-shadow:inset 0 0 0 1px rgba(120,142,166,0.18); font-size:13px; font-weight:700; \
                                 font-family:{}; outline:none;",
                                palette.panel_alt,
                                palette.text,
                                interface_font_family()
                            ),
                            oninput: move |evt| on_change.call(evt.value()),
                        }
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px;",
                    button {
                        "data-copy-edit-cancel": "1",
                        style: cancel_confirm_button_style(palette),
                        onclick: move |evt| on_cancel.call(evt),
                        "Cancel"
                    }
                    button {
                        "data-copy-edit-save": "1",
                        style: delete_confirm_button_style(palette, false),
                        onclick: move |evt| on_save.call(evt),
                        "Save"
                    }
                }
            }
        }
    }
}
#[component]
/// Leaving vertical tabs while folders exist. The classic tab bar has no place
/// to draw a folder, so the user's organization is about to move behind an
/// overflow menu. Nothing is deleted — but a tab silently vanishing from the
/// strip reads as data loss, and that is the surprise this dialog exists to
/// prevent. It is raised over a native web surface, so `has_modal_over_viewport`
/// counts it: without that the surface would paint straight over the dialog.
#[component]
fn ClassicTabsSwitchOverlay(
    palette: Palette,
    folder_count: usize,
    filed_count: usize,
    on_cancel: EventHandler<MouseEvent>,
    on_confirm: EventHandler<MouseEvent>,
) -> Element {
    let overlay_blur = overlay_backdrop_style("blur(18px) saturate(130%)");
    let folders = if folder_count == 1 {
        "1 folder".to_string()
    } else {
        format!("{folder_count} folders")
    };
    let tabs = if filed_count == 1 {
        "1 tab".to_string()
    } else {
        format!("{filed_count} tabs")
    };
    rsx! {
        div {
            "data-classic-tabs-overlay": "1",
            // §4 scope root — see the delete overlay's stamp.
            "data-yggterm-modal-root": "classic-tabs-switch",
            style: format!(
                "position:fixed; inset:0; z-index:95; display:flex; align-items:center; justify-content:center; \
                 background:rgba(230,239,248,0.28); backdrop-filter:{}; -webkit-backdrop-filter:{};",
                overlay_blur, overlay_blur,
            ),
            onclick: move |evt| on_cancel.call(evt),
            div {
                "data-classic-tabs-dialog": "1",
                style: format!(
                    "width:min(460px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:14px; \
                     padding:22px; border-radius:18px; background:rgba(250,252,255,0.96); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family(),
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; flex-direction:column; gap:6px;",
                    div {
                        "data-classic-tabs-title": "1",
                        style: format!(
                            "font-size:18px; font-weight:700; letter-spacing:-0.01em; color:{};",
                            palette.text,
                        ),
                        "The tab bar cannot show folders"
                    }
                    div {
                        "data-classic-tabs-copy": "1",
                        style: format!("font-size:12px; line-height:1.6; color:{};", palette.muted),
                        "Classic tabs are a single strip, so no folder is populated into it. \
                         Only the root tabs stay in the bar; {tabs} in your {folders} move into a \
                         dropdown at the end of the strip, where the vertical-tabs control used to be. \
                         Nothing is closed or deleted, and switching back to vertical tabs restores the tree."
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px;",
                    button {
                        "data-classic-tabs-cancel": "1",
                        style: cancel_confirm_button_style(palette),
                        onclick: move |evt| on_cancel.call(evt),
                        "Keep vertical tabs"
                    }
                    button {
                        "data-classic-tabs-action": "1",
                        style: delete_confirm_button_style(palette, false),
                        onclick: move |evt| on_confirm.call(evt),
                        "Switch to classic"
                    }
                }
            }
        }
    }
}
#[component]
fn DeleteConfirmOverlay(
    pending: PendingDeleteDialog,
    palette: Palette,
    on_cancel: EventHandler<MouseEvent>,
    on_confirm: EventHandler<MouseEvent>,
    on_confirm_unkept: EventHandler<MouseEvent>,
) -> Element {
    let item_count = pending_delete_display_count(&pending);
    let preview = pending.labels.iter().take(4).cloned().collect::<Vec<_>>();
    let dialog_text = delete_confirm_dialog_text(&pending);
    let bulk_live_close = pending.live_session_bulk_close;
    let unkept_count = pending.live_session_unkept_paths.len();
    let unkept_disabled = unkept_count == 0;
    let overlay_blur = overlay_backdrop_style("blur(18px) saturate(130%)");
    rsx! {
        div {
            "data-delete-confirm-overlay": "1",
            // §4 scope root: while this is the top modal, the overlay-open walk
            // confines derivation to this subtree — badges INSIDE the modal.
            // Value matches the `data-yggterm-modal-open` marker's kind.
            "data-yggterm-modal-root": "delete",
            style: format!(
                "position:fixed; inset:0; z-index:95; display:flex; align-items:center; justify-content:center; \
                 background:rgba(230,239,248,0.28); backdrop-filter:{}; -webkit-backdrop-filter:{};",
                overlay_blur,
                overlay_blur
            ),
            onclick: move |evt| on_cancel.call(evt),
            div {
                "data-delete-confirm-dialog": "1",
                style: format!(
                    "width:min(460px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:14px; \
                     padding:22px; border-radius:18px; background:rgba(250,252,255,0.96); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; flex-direction:column; gap:6px;",
                    div {
                        "data-delete-confirm-title": "1",
                        style: format!(
                            "font-size:18px; font-weight:700; letter-spacing:-0.01em; color:{};",
                            palette.text
                        ),
                        "{dialog_text.title}"
                    }
                    div {
                        "data-delete-confirm-copy": "1",
                        style: format!("font-size:12px; line-height:1.6; color:{};", palette.muted),
                        "{dialog_text.copy}"
                    }
                }
                div {
                    style: "display:flex; flex-direction:column; gap:6px; max-height:180px; overflow:auto; padding-right:4px;",
                    for label in preview {
                        div {
                            style: format!("font-size:12px; line-height:1.5; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.text),
                            "• {label}"
                        }
                    }
                    if item_count > 4 {
                        div {
                            style: format!("font-size:11px; color:{};", palette.muted),
                            "+ {item_count - 4} more"
                        }
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px;",
                    button {
                        "data-delete-confirm-cancel": "1",
                        style: cancel_confirm_button_style(palette),
                        onclick: move |evt| on_cancel.call(evt),
                        "Cancel"
                    }
                    if bulk_live_close {
                        button {
                            "data-delete-confirm-unkept-action": "1",
                            disabled: unkept_disabled,
                            style: format!(
                                "{} opacity:{}; cursor:{};",
                                delete_confirm_button_style(palette, false),
                                if unkept_disabled { "0.45" } else { "1" },
                                if unkept_disabled { "not-allowed" } else { "pointer" }
                            ),
                            onclick: move |evt| {
                                if !unkept_disabled {
                                    on_confirm_unkept.call(evt);
                                }
                            },
                            "Close All Un Keep-Alive Sessions"
                        }
                    }
                    button {
                        "data-delete-confirm-action": "1",
                        style: delete_confirm_button_style(palette, pending.hard_delete),
                        onclick: move |evt| on_confirm.call(evt),
                        "{dialog_text.action_label}"
                    }
                }
            }
        }
    }
}
/// The WebAuthn presence dialog. A libyggterm app (ychrome) asks the user to
/// approve a passkey ceremony; the app carries only the site and account, never
/// a challenge or a key. Approve POSTs the grant to the app's control endpoint,
/// which then mints consent and signs; Decline POSTs a deny.
///
/// This is deliberately a plain modal, not a "type to confirm" gate: the strong
/// boundary is that the PAGE cannot reach the grant channel (it never learns the
/// request id, and the grant route is GUI→app over `ssh -L`), so a site can make
/// this appear but can never answer it. Approving is a deliberate operator
/// action at the GUI.
#[component]
fn Fido2PresenceOverlay(
    dialog: PendingFido2Dialog,
    palette: Palette,
    /// `Some(credential_id)` = the picked account (multi-account picker);
    /// `None` = a single-account Approve (the app signs the only match).
    on_approve: EventHandler<Option<String>>,
    on_decline: EventHandler<MouseEvent>,
) -> Element {
    let overlay_blur = overlay_backdrop_style("blur(18px) saturate(130%)");
    // A create() is always one account. A get() with several stored passkeys is a
    // PICKER — the user chooses which account to sign in as, the way the Bitwarden
    // extension offers a list.
    let is_picker = dialog.ceremony != "create" && dialog.accounts.len() > 1;
    let title = if dialog.ceremony == "create" {
        "Register a passkey?"
    } else if is_picker {
        "Choose a passkey"
    } else {
        "Sign in with a passkey?"
    };
    let verb = if dialog.ceremony == "create" { "Register" } else { "Approve" };
    let account = if dialog.account.is_empty() {
        "this account".to_string()
    } else {
        dialog.account.clone()
    };
    let accounts = dialog.accounts.clone();
    rsx! {
        div {
            "data-fido2-overlay": "1",
            // §4 scope root — see the delete overlay's stamp.
            "data-yggterm-modal-root": "fido2",
            style: format!(
                "position:fixed; inset:0; z-index:96; display:flex; align-items:center; justify-content:center; \
                 background:rgba(230,239,248,0.28); backdrop-filter:{}; -webkit-backdrop-filter:{};",
                overlay_blur,
                overlay_blur
            ),
            // A backdrop click declines — never approves.
            onclick: move |evt| on_decline.call(evt),
            div {
                "data-fido2-dialog": "1",
                style: format!(
                    "width:min(440px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:16px; \
                     padding:22px; border-radius:18px; background:rgba(250,252,255,0.96); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; align-items:center; gap:10px;",
                    div {
                        style: "font-size:22px; line-height:1;",
                        "\u{1f511}\u{fe0e}"
                    }
                    div {
                        "data-fido2-title": "1",
                        style: format!(
                            "font-size:18px; font-weight:700; letter-spacing:-0.01em; color:{};",
                            palette.text
                        ),
                        "{title}"
                    }
                }
                div {
                    style: "display:flex; flex-direction:column; gap:8px;",
                    // A single-account ceremony names the one account inline; a
                    // picker lists them as buttons below instead.
                    if !is_picker {
                        div {
                            style: format!("font-size:13px; line-height:1.5; color:{};", palette.text),
                            "{account}"
                        }
                    }
                    div {
                        "data-fido2-rp": "1",
                        style: format!(
                            "font-size:12px; line-height:1.5; color:{}; \
                             font-family:'JetBrains Mono', ui-monospace, monospace;",
                            palette.muted
                        ),
                        "{dialog.rp_id}"
                    }
                    if !dialog.origin.is_empty() && dialog.origin != dialog.rp_id {
                        div {
                            style: format!("font-size:11px; line-height:1.5; color:{};", palette.muted),
                            "requested by {dialog.origin}"
                        }
                    }
                }
                // The picker: one button per matched account. Clicking a row IS
                // the choice AND the consent for that credential.
                if is_picker {
                    div {
                        "data-fido2-accounts": "1",
                        style: "display:flex; flex-direction:column; gap:8px; max-height:260px; overflow:auto;",
                        for account in accounts.iter().cloned() {
                            button {
                                "data-fido2-account": "{account.credential_id}",
                                style: format!(
                                    "display:flex; align-items:center; gap:8px; width:100%; text-align:left; \
                                     padding:11px 13px; border-radius:12px; border:none; cursor:pointer; \
                                     background:{}; color:{}; font-size:13px; font-weight:600; font-family:{}; \
                                     box-shadow: inset 0 0 0 1px rgba(196,210,224,0.9);",
                                    chrome_chip_fill(palette, false),
                                    palette.text,
                                    interface_font_family()
                                ),
                                onclick: {
                                    let credential_id = account.credential_id.clone();
                                    move |_| on_approve.call(Some(credential_id.clone()))
                                },
                                span {
                                    style: "font-size:15px; line-height:1;",
                                    "\u{1f511}\u{fe0e}"
                                }
                                "{account.label}"
                            }
                        }
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px;",
                    button {
                        "data-fido2-decline": "1",
                        style: cancel_confirm_button_style(palette),
                        onclick: move |evt| on_decline.call(evt),
                        "Not now"
                    }
                    // The single-account Approve. A picker has no blanket Approve —
                    // the user must name an account.
                    if !is_picker {
                        button {
                            "data-fido2-approve": "1",
                            style: delete_confirm_button_style(palette, false),
                            onclick: move |_| on_approve.call(None),
                            "{verb}"
                        }
                    }
                }
            }
        }
    }
}
/// The camera/microphone prompt. A page called `getUserMedia()`; WebKitGTK is
/// blocked on the answer and every exit from this dialog settles it.
///
/// ⛔ Three deliberate choices, all of them safety:
///
/// * **The default action is not a grant.** The dismissal paths (Escape, the
///   backdrop, "Not now") all DENY, and Enter is swallowed by the dispatcher —
///   so no keystroke and no stray click can hand over a device.
/// * **The site is named in monospace, verbatim.** A prompt whose subject the
///   user cannot read is a prompt they cannot answer, and the whole class of
///   attack here is looking like a different site.
/// * **The wording names exactly what was asked for.** "Camera" when the page
///   asked for a camera; never a generic "media" that a microphone-only ask
///   could hide inside.
#[component]
fn MediaCapturePresenceOverlay(
    dialog: PendingMediaCaptureDialog,
    palette: Palette,
    on_answer: EventHandler<MediaCaptureAnswer>,
) -> Element {
    let overlay_blur = overlay_backdrop_style("blur(18px) saturate(130%)");
    let devices = media_capture_devices_phrase(dialog.audio, dialog.video);
    let glyph = if dialog.video {
        // A camera when video is involved; a microphone for an audio-only ask.
        "\u{1f4f7}\u{fe0e}"
    } else {
        "\u{1f3a4}\u{fe0e}"
    };
    // Nothing can be remembered for a document with no origin — say so rather
    // than offering a "block this site" that would silently do nothing.
    let remembers = dialog.origin.is_some();
    rsx! {
        div {
            "data-media-capture-overlay": "1",
            // §4 scope root — see the passkey overlay's stamp.
            "data-yggterm-modal-root": "media-capture",
            style: format!(
                "position:fixed; inset:0; z-index:97; display:flex; align-items:center; justify-content:center; \
                 background:rgba(230,239,248,0.28); backdrop-filter:{}; -webkit-backdrop-filter:{};",
                overlay_blur,
                overlay_blur
            ),
            // A backdrop click DENIES — never approves, and never just closes.
            onclick: move |_| on_answer.call(MediaCaptureAnswer::DenyOnce),
            div {
                "data-media-capture-dialog": "1",
                style: format!(
                    "width:min(440px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:16px; \
                     padding:22px; border-radius:18px; background:rgba(250,252,255,0.96); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; align-items:center; gap:10px;",
                    div { style: "font-size:22px; line-height:1;", "{glyph}" }
                    div {
                        "data-media-capture-title": "1",
                        style: format!(
                            "font-size:18px; font-weight:700; letter-spacing:-0.01em; color:{};",
                            palette.text
                        ),
                        "Use {devices}?"
                    }
                }
                div {
                    style: "display:flex; flex-direction:column; gap:8px;",
                    div {
                        "data-media-capture-origin": "1",
                        style: format!(
                            "font-size:12px; line-height:1.5; color:{}; \
                             font-family:'JetBrains Mono', ui-monospace, monospace; \
                             overflow-wrap:anywhere;",
                            palette.muted
                        ),
                        "{dialog.display}"
                    }
                    div {
                        style: format!("font-size:12px; line-height:1.5; color:{};", palette.muted),
                        if remembers {
                            "Allowing is remembered for this site until you revoke it in the browser's settings."
                        } else {
                            "This page has no site address, so nothing about it can be remembered — allowing applies to this request only."
                        }
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px; flex-wrap:wrap;",
                    // Remembered refusal, offered only where there is a site to
                    // remember it against.
                    if remembers {
                        button {
                            "data-media-capture-block": "1",
                            style: cancel_confirm_button_style(palette),
                            onclick: move |_| on_answer.call(MediaCaptureAnswer::BlockSite),
                            "Block this site"
                        }
                    }
                    button {
                        "data-media-capture-decline": "1",
                        style: cancel_confirm_button_style(palette),
                        onclick: move |_| on_answer.call(MediaCaptureAnswer::DenyOnce),
                        "Not now"
                    }
                    button {
                        "data-media-capture-allow": "1",
                        style: delete_confirm_button_style(palette, false),
                        onclick: move |_| on_answer.call(MediaCaptureAnswer::Allow),
                        "Allow"
                    }
                }
            }
        }
    }
}
/// The agent-CLI launch-flags modal.
///
/// ⛔ **GENERATED, one row per descriptor** — the law of
/// `docs/spec-agent-cli-extra-args-modal.md` §1. There is no per-CLI `rsx!` here
/// and there must never be: nine hand-written rows is what the titlebar `+` menu
/// is filed for, and adding the tenth CLI must remain a line in a table.
#[component]
fn LaunchFlagsOverlay(
    snapshot: SharedSnapshot,
    on_close: EventHandler<MouseEvent>,
    on_change: EventHandler<(String, String)>,
    on_reset: EventHandler<String>,
    on_focus_input: EventHandler<()>,
    on_blur_input: EventHandler<()>,
) -> Element {
    let overlay_wash = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgba(228,237,245,0.03)",
        UiTheme::ZedDark => "rgba(10,14,18,0.05)",
    };
    let editor_surface = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgb(248,252,255)",
        UiTheme::ZedDark => "rgb(28,34,41)",
    };
    let editor_shadow = match snapshot.settings.theme {
        UiTheme::ZedLight => {
            "0 0 0 1px rgba(215,229,243,0.96), 0 0 0 10px rgba(129,188,255,0.18), 0 26px 60px rgba(55,83,112,0.20), inset 0 0 0 1px rgba(214,223,232,0.92)"
        }
        UiTheme::ZedDark => {
            "0 0 0 1px rgba(59,87,112,0.90), 0 0 0 10px rgba(124,200,255,0.16), 0 26px 60px rgba(0,0,0,0.42), inset 0 0 0 1px rgba(68,84,99,0.94)"
        }
    };
    let palette = snapshot.palette;
    let stored = snapshot.settings.agent_cli_extra_args.clone();
    let rows = launch_flags_rows().collect::<Vec<_>>();
    rsx! {
        div {
            "data-launch-flags-overlay": "1",
            "data-yggterm-modal-root": "launch-flags",
            style: format!(
                "position:fixed; inset:0; z-index:98; display:flex; align-items:center; justify-content:center; background:{};",
                overlay_wash
            ),
            onmousedown: move |evt| on_close.call(evt),
            onclick: move |evt| on_close.call(evt),
            div {
                "data-launch-flags-shell": "1",
                style: format!(
                    "width:min(660px, calc(100vw - 44px)); max-height:calc(100vh - 56px); overflow:auto; \
                     display:flex; flex-direction:column; gap:12px; padding:16px; \
                     border-radius:22px; background:{}; color:{}; box-shadow:{}; font-family:{};",
                    editor_surface,
                    palette.text,
                    editor_shadow,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                    div {
                        style: "display:flex; flex-direction:column; gap:3px;",
                        div {
                            style: format!("font-size:15px; font-weight:800; letter-spacing:-0.01em; color:{};", palette.text),
                            "Agent CLI launch flags"
                        }
                        div {
                            style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                            "Each CLI expresses its permission checks in its own vocabulary. Pick a tier or type your own — typing always wins."
                        }
                    }
                    button {
                        "data-launch-flags-close": "1",
                        style: format!(
                            "border:none; background:transparent; color:{}; font-size:16px; font-weight:700; cursor:pointer;",
                            palette.muted
                        ),
                        onclick: move |evt| on_close.call(evt),
                        "✕"
                    }
                }
                for descriptor in rows {
                    LaunchFlagsRow {
                        key: "{descriptor.slug}",
                        palette,
                        slug: descriptor.slug.to_string(),
                        display_name: descriptor.display_name.to_string(),
                        icon_glyph: descriptor.icon_glyph.to_string(),
                        brand_color: descriptor.brand_color.to_string(),
                        value: launch_flags_box_value(descriptor, &stored),
                        customised: stored.contains_key(descriptor.extra_args_slug),
                        presets: descriptor.permission_presets,
                        provenance_note: launch_flags_provenance_note(descriptor),
                        disabled: matches!(
                            descriptor.permission_provenance,
                            yggterm_core::agent_cli::PermissionProvenance::Unmeasured(_)
                        ),
                        on_change,
                        on_reset,
                        on_focus_input: on_focus_input.clone(),
                        on_blur_input: on_blur_input.clone(),
                    }
                }
            }
        }
    }
}

/// The marker a row wears for where its flags came from — spec §5.
///
/// Provenance is part of the UI, not a footnote: a row read off a running binary
/// and a row taken from a vendor doc must not look the same.
fn launch_flags_provenance_note(
    descriptor: &yggterm_core::agent_cli::AgentCliDescriptor,
) -> String {
    match descriptor.permission_provenance {
        yggterm_core::agent_cli::PermissionProvenance::Measured => String::new(),
        yggterm_core::agent_cli::PermissionProvenance::Documented => {
            "documented, not verified here".to_string()
        }
        yggterm_core::agent_cli::PermissionProvenance::Unmeasured(reason) => reason.to_string(),
    }
}

#[component]
#[allow(clippy::too_many_arguments)]
fn LaunchFlagsRow(
    palette: Palette,
    slug: String,
    display_name: String,
    icon_glyph: String,
    brand_color: String,
    value: String,
    customised: bool,
    presets: &'static [yggterm_core::agent_cli::PermissionPreset],
    provenance_note: String,
    disabled: bool,
    on_change: EventHandler<(String, String)>,
    on_reset: EventHandler<String>,
    on_focus_input: EventHandler<()>,
    on_blur_input: EventHandler<()>,
) -> Element {
    // Which tier the current text corresponds to, by exact match. Anything else
    // is Custom — the free text is authoritative and a preset button never
    // silently rewrites it.
    let active_preset = presets
        .iter()
        .find(|preset| preset.args.trim() == value.trim())
        .map(|preset| preset.id);
    let explanation = active_preset
        .and_then(|id| presets.iter().find(|preset| preset.id == id))
        .map(|preset| preset.explanation.to_string())
        .unwrap_or_else(|| {
            if disabled {
                provenance_note.clone()
            } else {
                "Custom flags — not one of this CLI's named tiers.".to_string()
            }
        });
    let row_slug = slug.clone();
    let reset_slug = slug.clone();
    rsx! {
        div {
            "data-launch-flags-row": "{slug}",
            style: format!(
                "display:flex; flex-direction:column; gap:7px; padding:10px; border-radius:14px; \
                 background:{}; box-shadow: inset 0 0 0 1px {};",
                if palette_is_dark(palette) { "rgba(255,255,255,0.04)" } else { "rgba(255,255,255,0.22)" },
                if palette_is_dark(palette) { "rgba(141,160,178,0.16)" } else { "rgba(198,212,224,0.32)" }
            ),
            div {
                style: "display:flex; align-items:center; gap:8px;",
                span {
                    style: format!(
                        "display:inline-flex; align-items:center; justify-content:center; width:22px; height:18px; \
                         border-radius:5px; background:{}; color:#ffffff; font-family:'JetBrains Mono', ui-monospace, monospace; \
                         font-size:8px; font-weight:800;",
                        brand_color
                    ),
                    "{icon_glyph}"
                }
                span {
                    style: format!("font-size:12px; font-weight:800; color:{};", palette.text),
                    "{display_name}"
                }
                if !provenance_note.is_empty() {
                    span {
                        style: format!(
                            "font-size:9px; font-weight:700; padding:1px 6px; border-radius:999px; color:{}; \
                             box-shadow: inset 0 0 0 1px {};",
                            palette.muted,
                            if palette_is_dark(palette) { "rgba(141,160,178,0.30)" } else { "rgba(198,212,224,0.70)" }
                        ),
                        if disabled { "unmeasured" } else { "documented" }
                    }
                }
                span { style: "flex:1;" }
                if customised {
                    button {
                        "data-launch-flags-reset": "{slug}",
                        style: format!(
                            "border:none; background:transparent; color:{}; font-size:10px; font-weight:700; cursor:pointer;",
                            palette.muted
                        ),
                        onclick: move |_| on_reset.call(reset_slug.clone()),
                        "Reset"
                    }
                }
            }
            if !presets.is_empty() {
                div {
                    style: "display:flex; flex-wrap:wrap; gap:6px;",
                    for preset in presets.iter() {
                        button {
                            key: "{preset.id}",
                            "data-launch-flags-preset": "{slug}/{preset.id}",
                            disabled,
                            style: launch_flags_preset_style(palette, active_preset == Some(preset.id), disabled),
                            onclick: {
                                let slug = row_slug.clone();
                                let args = preset.args.to_string();
                                move |_| on_change.call((slug.clone(), args.clone()))
                            },
                            "{preset.label}"
                        }
                    }
                }
            }
            input {
                "data-launch-flags-input": "{slug}",
                r#type: "text",
                value: "{value}",
                disabled,
                placeholder: "no flags",
                style: format!(
                    "height:30px; padding:0 9px; border:none; border-radius:9px; background:{}; color:{}; \
                     box-shadow: inset 0 0 0 1px {}; font-family:'JetBrains Mono', ui-monospace, monospace; font-size:11px;",
                    if palette_is_dark(palette) { "rgba(13,18,24,0.72)" } else { "rgba(255,255,255,0.90)" },
                    palette.text,
                    if palette_is_dark(palette) { "rgba(93,116,134,0.44)" } else { "rgba(208,219,229,0.85)" }
                ),
                onfocus: move |_| on_focus_input.call(()),
                onblur: move |_| on_blur_input.call(()),
                oninput: {
                    let slug = slug.clone();
                    move |evt: FormEvent| on_change.call((slug.clone(), evt.value()))
                },
            }
            div {
                style: format!("font-size:10px; line-height:1.45; color:{};", palette.muted),
                "{explanation}"
            }
        }
    }
}

fn launch_flags_preset_style(palette: Palette, active: bool, disabled: bool) -> String {
    let background = if disabled {
        "transparent".to_string()
    } else if active {
        palette.accent.to_string()
    } else if palette_is_dark(palette) {
        "rgba(21,28,35,0.94)".to_string()
    } else {
        "rgba(255,255,255,0.86)".to_string()
    };
    let color = if active && !disabled {
        "#ffffff"
    } else {
        palette.muted
    };
    format!(
        "height:24px; padding:0 10px; border:none; border-radius:999px; background:{background}; color:{color}; \
         font-size:10px; font-weight:700; cursor:{};",
        if disabled { "not-allowed" } else { "pointer" }
    )
}

#[component]
fn ThemeEditorOverlay(
    snapshot: SharedSnapshot,
    on_close: EventHandler<MouseEvent>,
    on_reset: EventHandler<MouseEvent>,
    on_seed: EventHandler<MouseEvent>,
    on_set_ui_theme: EventHandler<UiTheme>,
    on_add_stop: EventHandler<MouseEvent>,
    on_remove_stop: EventHandler<MouseEvent>,
    on_pick_stop: EventHandler<usize>,
    on_begin_drag_stop: EventHandler<usize>,
    on_drag_stop: EventHandler<(f32, f32)>,
    on_end_drag_stop: EventHandler<MouseEvent>,
    on_double_click_pad: EventHandler<(f32, f32)>,
    on_update_stop_color: EventHandler<String>,
    on_pick_swatch: EventHandler<String>,
    on_set_brightness: EventHandler<f32>,
    on_set_alpha: EventHandler<f32>,
    on_set_grain: EventHandler<f32>,
) -> Element {
    let _ = (&on_set_alpha, &on_set_grain);
    let selected_stop = snapshot
        .theme_editor_selected_stop
        .and_then(|index| snapshot.theme_editor_draft.colors.get(index).cloned());
    let preview_surface =
        preview_surface_css(snapshot.settings.theme, &snapshot.theme_editor_draft);
    let brightness_percent = (snapshot.theme_editor_draft.brightness * 100.0).round() as i32;
    let accent = snapshot.theme_accent.clone();
    let preview_has_stops = !snapshot.theme_editor_draft.colors.is_empty();
    let overlay_wash = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgba(228,237,245,0.03)",
        UiTheme::ZedDark => "rgba(10,14,18,0.05)",
    };
    let editor_surface = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgb(248,252,255)",
        UiTheme::ZedDark => "rgb(28,34,41)",
    };
    let editor_shadow = match snapshot.settings.theme {
        UiTheme::ZedLight => {
            "0 0 0 1px rgba(215,229,243,0.96), 0 0 0 10px rgba(129,188,255,0.18), 0 26px 60px rgba(55,83,112,0.20), inset 0 0 0 1px rgba(214,223,232,0.92)"
        }
        UiTheme::ZedDark => {
            "0 0 0 1px rgba(59,87,112,0.90), 0 0 0 10px rgba(124,200,255,0.16), 0 26px 60px rgba(0,0,0,0.42), inset 0 0 0 1px rgba(68,84,99,0.94)"
        }
    };
    rsx! {
        div {
            "data-theme-editor-overlay": "1",
            // §4 walk root — see the delete overlay's stamp.
            "data-yggterm-modal-root": "theme-editor",
            style: format!(
                "position:fixed; inset:0; z-index:98; display:flex; align-items:center; justify-content:center; background:{};",
                overlay_wash
            ),
            onmousedown: move |evt| on_close.call(evt),
            onclick: move |evt| on_close.call(evt),
            div {
                "data-theme-editor-shell": "1",
                style: format!(
                    "width:min(640px, calc(100vw - 44px)); max-height:calc(100vh - 56px); overflow:auto; \
                     display:flex; flex-direction:column; gap:14px; padding:14px; \
                     border-radius:22px; background:{}; color:{}; \
                     box-shadow:{}; \
                     font-family:{};",
                    editor_surface,
                    snapshot.palette.text,
                    editor_shadow,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        div {
                            style: format!(
                                "width:11px; height:11px; border-radius:999px; background:{}; box-shadow:0 0 0 4px rgba(128,175,212,0.12);",
                                accent
                            ),
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:3px;",
                            div {
                                style: format!("font-size:15px; font-weight:800; letter-spacing:-0.01em; color:{};", snapshot.palette.text),
                                "Edit Theme"
                            }
                            div {
                                style: format!("font-size:11px; line-height:1.45; color:{};", snapshot.palette.muted),
                                "Shape the shell gradient and brightness for Yggui. Changes apply immediately, and closing the editor saves them to ~/.yggterm/settings.json."
                            }
                        }
                    }
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        div {
                            // §12.4 clause 3 — a group is ONE tab stop: Tab lands
                            // on the SELECTED segment (roving tabindex, rendered
                            // from the same state the styling reads) and ←/→ move
                            // between them. Also the dialog's autofocus head: the
                            // keyboard should arrive at the choice, not at the ✕
                            // that merely happens to come next in the DOM.
                            "data-keynav-group": "theme-mode",
                            "data-yggterm-modal-autofocus": "1",
                            style: segmented_control_track_style(snapshot.palette),
                            button {
                                "data-keynav-item": "light",
                                tabindex: if snapshot.settings.theme == UiTheme::ZedLight { "0" } else { "-1" },
                                style: segmented_control_segment_style(snapshot.palette, snapshot.settings.theme == UiTheme::ZedLight, true, false),
                                onclick: move |_| on_set_ui_theme.call(UiTheme::ZedLight),
                                "Light"
                            }
                            button {
                                "data-keynav-item": "dark",
                                tabindex: if snapshot.settings.theme == UiTheme::ZedDark { "0" } else { "-1" },
                                style: segmented_control_segment_style(snapshot.palette, snapshot.settings.theme == UiTheme::ZedDark, true, false),
                                onclick: move |_| on_set_ui_theme.call(UiTheme::ZedDark),
                                "Dark"
                            }
                        }
                        button {
                            style: icon_button_style(snapshot.palette),
                            onclick: move |evt| on_close.call(evt),
                            "✕"
                        }
                    }
                }
                div {
                    style: "display:flex; flex-wrap:wrap; gap:14px; align-items:stretch;",
                    div {
                        style: format!(
                            "flex:1 1 {}px; min-width:min(100%, {}px); max-width:{}px; display:flex; flex-direction:column; gap:10px;",
                            THEME_EDITOR_PAD_SIZE as i32,
                            THEME_EDITOR_PAD_SIZE as i32,
                            THEME_EDITOR_PAD_SIZE as i32
                        ),
                        div {
                            style: format!(
                                "position:relative; width:{}px; min-width:{}px; height:{}px; border-radius:20px; overflow:hidden; \
                                 background:{}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.56), 0 18px 38px rgba(84,113,137,0.12);",
                                THEME_EDITOR_PAD_SIZE as i32,
                                THEME_EDITOR_PAD_SIZE as i32,
                                THEME_EDITOR_PAD_SIZE as i32,
                                preview_surface
                            ),
                            // ⭐ THE GRID IS MAGNETIC, AND ALT LETS GO OF IT.
                            // The pad has always PAINTED a grid; until now it did
                            // not act on one, so every line on it was decoration
                            // and a stop could only ever be eyeballed onto it.
                            // Holding Alt suspends the magnetism for the placement
                            // that genuinely wants to sit between two lines.
                            onmousemove: move |evt| {
                                let point = evt.element_coordinates();
                                let snapping = !evt.modifiers().contains(Modifiers::ALT);
                                on_drag_stop.call((
                                    normalize_theme_editor_axis(snap_theme_editor_axis_px(
                                        point.x, snapping,
                                    )),
                                    normalize_theme_editor_axis(snap_theme_editor_axis_px(
                                        point.y, snapping,
                                    )),
                                ));
                            },
                            onmouseup: move |evt| on_end_drag_stop.call(evt),
                            ondoubleclick: move |evt| {
                                let point = evt.element_coordinates();
                                let snapping = !evt.modifiers().contains(Modifiers::ALT);
                                on_double_click_pad.call((
                                    normalize_theme_editor_axis(snap_theme_editor_axis_px(
                                        point.x, snapping,
                                    )),
                                    normalize_theme_editor_axis(snap_theme_editor_axis_px(
                                        point.y, snapping,
                                    )),
                                ));
                            },
                            div {
                                style: "position:absolute; inset:0; background-image: linear-gradient(rgba(144,173,199,0.18) 1px, transparent 1px), linear-gradient(90deg, rgba(144,173,199,0.18) 1px, transparent 1px); background-size: 24px 24px; opacity:0.78; pointer-events:none;",
                            }
                            div {
                                style: "position:absolute; inset:0; background-image: linear-gradient(rgba(255,255,255,0.24) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.24) 1px, transparent 1px); background-size: 96px 96px; opacity:0.52; pointer-events:none;",
                            }
                            if !preview_has_stops {
                                div {
                                    style: format!(
                                        "position:absolute; inset:0; display:flex; align-items:center; justify-content:center; padding:18px; \
                                         text-align:center; font-size:12px; font-weight:700; line-height:1.6; color:{};",
                                        snapshot.palette.text
                                    ),
                                    "Double-click to add a color"
                                }
                            }
                            for (index, stop) in snapshot.theme_editor_draft.colors.iter().enumerate() {
                                button {
                                    key: "theme-stop-{index}",
                                    // ⭐ THE HANDLE SAYS WHEN THE GRID HAS HOLD OF IT.
                                    // A magnetic grid the eye cannot read is worse
                                    // than none: the point moves by an amount the
                                    // hand did not ask for and nothing explains why.
                                    // A stop resting on the grid wears a thin accent
                                    // halo; one placed freely does not. Derived from
                                    // the stop's coordinates (DESIGN.md ▸ Gradient pad),
                                    // so it reads the same on a theme loaded from disk
                                    // as on one just dragged.
                                    style: format!(
                                        "position:absolute; left:calc({:.2}% - 11px); top:calc({:.2}% - 11px); width:22px; height:22px; \
                                         border-radius:999px; border:{}; background:{}; box-shadow:{};",
                                        stop.x * 100.0,
                                        stop.y * 100.0,
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            format!("3px solid {}", accent)
                                        } else {
                                            "2px solid rgba(255,255,255,0.86)".to_string()
                                        },
                                        stop.color,
                                        if theme_editor_axis_is_on_grid(stop.x)
                                            && theme_editor_axis_is_on_grid(stop.y)
                                        {
                                            format!(
                                                "0 0 0 4px {}, 0 10px 22px rgba(42,67,88,0.16)",
                                                theme_editor_snap_halo(&accent)
                                            )
                                        } else {
                                            "0 10px 22px rgba(42,67,88,0.16)".to_string()
                                        }
                                    ),
                                    onmousedown: move |evt| {
                                        evt.stop_propagation();
                                        on_begin_drag_stop.call(index);
                                    },
                                    onclick: move |_| on_pick_stop.call(index),
                                }
                            }
                        }
                        div {
                            style: "display:flex; align-items:center; justify-content:space-between; gap:12px; padding:8px 10px; border-radius:14px; background:rgba(247,250,253,0.9); box-shadow:inset 0 0 0 1px rgba(214,223,232,0.92);",
                            div {
                                style: "display:flex; flex-direction:column; gap:3px;",
                                div {
                                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                    "Gradient Stops"
                                }
                                div {
                                    style: format!("font-size:11px; color:{};", snapshot.palette.muted),
                                    if snapshot.theme_editor_draft.colors.is_empty() {
                                        "Start with one color, then add more only when the gradient needs them."
                                    } else {
                                        "{snapshot.theme_editor_draft.colors.len()} stop(s) in this gradient"
                                    }
                                }
                            }
                            div {
                                style: "display:flex; align-items:center; gap:8px;",
                                ThemeDialogButton {
                                    data_action: None,
                                    primary: false,
                                    enabled: true,
                                    palette: snapshot.palette,
                                    accent: accent.clone(),
                                    onclick: move |evt| on_add_stop.call(evt),
                                    prefix: Some("+".to_string()),
                                    "Add Stop"
                                }
                                ThemeDialogButton {
                                    data_action: None,
                                    primary: false,
                                    enabled: snapshot.theme_editor_selected_stop.is_some(),
                                    palette: snapshot.palette,
                                    accent: accent.clone(),
                                    onclick: move |evt| on_remove_stop.call(evt),
                                    prefix: Some("−".to_string()),
                                    "Remove"
                                }
                            }
                        }
                    }
                    div {
                        style: "flex:1 1 266px; display:flex; flex-direction:column; gap:12px; min-width:min(100%, 252px);",
                        div {
                            // §12.4 clause 3 — the stop chips are one group: Tab
                            // lands on the SELECTED chip, arrows walk the stops.
                            "data-keynav-group": "theme-stops",
                            style: "display:flex; flex-wrap:wrap; gap:8px;",
                            for (index, stop) in snapshot.theme_editor_draft.colors.iter().enumerate() {
                                button {
                                    key: "theme-chip-{index}",
                                    "data-keynav-item": "stop-{index}",
                                    // Roving tabindex from the SAME selection the
                                    // styling below reads — one source, so the
                                    // ring can never land on an unselected chip.
                                    tabindex: if snapshot.theme_editor_selected_stop == Some(index)
                                        || (snapshot.theme_editor_selected_stop.is_none() && index == 0) { "0" } else { "-1" },
                                    style: format!(
                                        "display:flex; align-items:center; gap:8px; height:32px; padding:0 10px; border:none; border-radius:999px; \
                                         background:{}; color:{}; box-shadow:{};",
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            "rgba(255,255,255,0.96)"
                                        } else {
                                            "rgba(246,249,252,0.84)"
                                        },
                                        snapshot.palette.text,
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            format!("inset 0 0 0 2px {}", accent)
                                        } else {
                                            "inset 0 0 0 1px rgba(214,223,232,0.92)".to_string()
                                        }
                                    ),
                                    onclick: move |_| on_pick_stop.call(index),
                                    span {
                                        style: format!("width:14px; height:14px; border-radius:999px; background:{}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.88);", stop.color),
                                    }
                                    span {
                                        style: format!("font-size:11px; font-weight:700; color:{};", snapshot.palette.text),
                                        "Color {index + 1}"
                                    }
                                }
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px;",
                            div {
                                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                "Color Library"
                            }
                            div {
                                // §12.4 clause 3 — the library is one group; no
                                // swatch is "selected", so the first is the stop.
                                "data-keynav-group": "theme-library",
                                style: "display:flex; flex-wrap:wrap; gap:8px;",
                                for (swatch_index, swatch) in THEME_EDITOR_SWATCHES.iter().enumerate() {
                                    button {
                                        key: "theme-swatch-{swatch}",
                                        "data-keynav-item": "swatch-{swatch_index}",
                                        tabindex: if swatch_index == 0 { "0" } else { "-1" },
                                        style: format!(
                                            "width:24px; height:24px; border-radius:999px; border:2px solid rgba(255,255,255,0.92); background:{}; box-shadow:0 8px 16px rgba(45,67,88,0.12);",
                                            swatch
                                        ),
                                        onclick: move |_| on_pick_swatch.call(swatch.to_string()),
                                    }
                                }
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px;",
                            div {
                                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                "Selected Color"
                            }
                            input {
                                r#type: "color",
                                value: selected_stop.as_ref().map(|stop| stop.color.clone()).unwrap_or_else(|| accent.clone()),
                                style: "width:100%; height:42px; border:none; border-radius:12px; background:transparent;",
                                oninput: move |evt| on_update_stop_color.call(evt.value()),
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px; padding:10px 12px; border-radius:14px; background:rgba(247,250,253,0.9); box-shadow:inset 0 0 0 1px rgba(214,223,232,0.92);",
                            div {
                                style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                                div {
                                    style: format!("font-size:11px; font-weight:800; letter-spacing:0.02em; color:{};", snapshot.palette.text),
                                    "Brightness"
                                }
                                input {
                                    r#type: "number",
                                    min: "38",
                                    max: "72",
                                    value: "{brightness_percent}",
                                    style: "width:64px; height:28px; padding:0 8px; border:none; border-radius:9px; background:rgba(255,255,255,0.96); box-shadow:inset 0 0 0 1px rgba(214,223,232,0.92); font-size:12px; font-weight:700; text-align:right;",
                                    oninput: move |evt| {
                                        let value = evt.value().parse::<f32>().unwrap_or(56.0) / 100.0;
                                        on_set_brightness.call(value);
                                    },
                                }
                            }
                            input {
                                r#type: "range",
                                "data-theme-editor-brightness-input": "1",
                                min: "38",
                                max: "72",
                                value: "{brightness_percent}",
                                style: "width:100%; accent-color:#7cc8ff;",
                                oninput: move |evt| {
                                    let value = evt.value().parse::<f32>().unwrap_or(56.0) / 100.0;
                                    on_set_brightness.call(value);
                                },
                            }
                        }
                    }
                }
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                    div {
                        style: format!("font-size:11px; line-height:1.5; color:{};", snapshot.palette.muted),
                        if preview_has_stops {
                            "Double-click the pad to add another color, drag the dots to reshape the gradient, then close when it looks right."
                        } else {
                            "Start empty or use the starter palette, then drag colors until the shell feels right."
                        }
                    }
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        if !preview_has_stops {
                            ThemeDialogButton {
                                data_action: Some("seed".to_string()),
                                primary: false,
                                enabled: true,
                                palette: snapshot.palette,
                                accent: accent.clone(),
                                onclick: move |evt| on_seed.call(evt),
                                prefix: None,
                                "Use Starter"
                            }
                        }
                        ThemeDialogButton {
                            data_action: Some("reset".to_string()),
                            primary: false,
                            enabled: true,
                            palette: snapshot.palette,
                            accent: accent.clone(),
                            onclick: move |evt| on_reset.call(evt),
                            prefix: None,
                            "Reset to Defaults"
                        }
                    }
                }
            }
        }
    }
}
#[component]
fn ThemeDialogButton(
    data_action: Option<String>,
    primary: bool,
    enabled: bool,
    palette: Palette,
    accent: String,
    prefix: Option<String>,
    onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    let style = if primary {
        primary_action_style(palette)
    } else {
        theme_editor_action_button_style(palette, &accent, enabled, false)
    };
    let action_attr = data_action.unwrap_or_default();
    rsx! {
        button {
            "data-theme-editor-action": "{action_attr}",
            disabled: !enabled,
            style: style,
            onclick: move |evt| {
                if enabled {
                    onclick.call(evt);
                }
            },
            if let Some(prefix) = prefix {
                span {
                    style: format!(
                        "font-size:14px; font-weight:800; color:{};",
                        if enabled { accent.clone() } else { palette.muted.to_string() }
                    ),
                    "{prefix}"
                }
            }
            {children}
        }
    }
}
fn normalize_theme_editor_axis(value: f64) -> f32 {
    ((value / THEME_EDITOR_PAD_SIZE).clamp(0.0, 1.0)) as f32
}
/// The gradient pad's minor gridline pitch, in pad pixels. It is the SAME number
/// the pad paints its 24px grid with — one owner, so a point can never snap to a
/// line the eye cannot see. (The 96px major grid is a visual accent over these,
/// not a second set of targets.)
const THEME_EDITOR_GRID_PX: f64 = 24.0;
/// How close a point must come before the grid takes it. Arc's pad is MAGNETIC,
/// not quantised: a quantised pad cannot express a stop at 37% and so takes the
/// pen out of the designer's hand, while a magnetic one snaps the placements that
/// wanted to be exact and leaves the rest alone. 7px on a 24px pitch means the
/// outer ~70% of every cell is still free travel.
const THEME_EDITOR_SNAP_RADIUS_PX: f64 = 7.0;
/// Pull one axis of a gradient stop onto the visible grid, in PAD PIXELS.
///
/// Snap targets are the minor gridlines AND both pad edges. The edges must be
/// named explicitly: the pad is 286px and the pitch is 24px, so the far edge is
/// not a multiple of the pitch and the nearest-line arithmetic alone can never
/// land on it — a stop dragged into the far corner would come to rest a few
/// pixels short of it, which is precisely the placement a designer most wants
/// to be exact.
fn snap_theme_editor_axis_px(value: f64, snapping: bool) -> f64 {
    if !snapping {
        return value;
    }
    let mut best = value;
    let mut best_distance = THEME_EDITOR_SNAP_RADIUS_PX;
    let last_line = (THEME_EDITOR_PAD_SIZE / THEME_EDITOR_GRID_PX).floor() as i32;
    let candidates = (0..=last_line)
        .map(|line| f64::from(line) * THEME_EDITOR_GRID_PX)
        .chain([0.0, THEME_EDITOR_PAD_SIZE]);
    for candidate in candidates {
        let distance = (value - candidate).abs();
        if distance <= best_distance {
            best_distance = distance;
            best = candidate;
        }
    }
    best
}
/// The snap halo's colour: the theme accent at low alpha.
///
/// ⛔ AN ALPHA SUFFIX IS NOT UNIVERSALLY APPENDABLE. `#rrggbb` takes a two-digit
/// alpha and becomes `#rrggbbaa`, but the accent is a theme-supplied string and a
/// theme may hand over `rgb(…)`, `oklch(…)` or a named colour — appending to any
/// of those yields a colour the engine silently drops, taking the whole
/// `box-shadow` declaration with it and leaving the snap state invisible with
/// nothing in the log. Only the shape that can carry alpha gets it; anything else
/// falls back to a neutral halo that always renders.
fn theme_editor_snap_halo(accent: &str) -> String {
    let hex = accent.trim();
    if hex.len() == 7
        && hex.starts_with('#')
        && hex[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return format!("{hex}2e");
    }
    "rgba(120,142,166,0.30)".to_string()
}
/// True when a stop already sits on the grid, so the handle can SAY it is
/// snapped. Derived from the stop's own coordinates rather than remembered from
/// the drag that placed it — a remembered flag is a second encoding of the same
/// fact and drifts the moment a theme is loaded from disk rather than dragged.
fn theme_editor_axis_is_on_grid(normalized: f32) -> bool {
    // ⛔ NOT `snap(px) == px`. The snapper RETURNS ITS INPUT UNCHANGED when the
    // point is out of magnet range, so comparing against it reports every
    // free-standing point as on-grid — the answer is "true" for the mid-cell
    // placement this is meant to distinguish. Ask the gridlines directly.
    let px = f64::from(normalized) * THEME_EDITOR_PAD_SIZE;
    let last_line = (THEME_EDITOR_PAD_SIZE / THEME_EDITOR_GRID_PX).floor() as i32;
    (0..=last_line)
        .map(|line| f64::from(line) * THEME_EDITOR_GRID_PX)
        .chain([0.0, THEME_EDITOR_PAD_SIZE])
        .any(|candidate| (px - candidate).abs() < 0.5)
}
#[component]
fn SettingsField(
    field_key: String,
    label: String,
    value: String,
    placeholder: String,
    secret: bool,
    autofocus: bool,
    palette: Palette,
    on_focus_input: EventHandler<String>,
    on_blur_input: EventHandler<()>,
    on_change: EventHandler<String>,
) -> Element {
    let focus_key_on_mousedown = field_key.clone();
    let focus_key_on_click = field_key.clone();
    let focus_key_on_focus = field_key.clone();
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:4px;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "{label}"
            }
            input {
                "data-settings-field-key": "{field_key}",
                "data-yggui-field": "true",
                r#type: if secret { "password" } else { "text" },
                value: "{value}",
                placeholder: "{placeholder}",
                style: settings_input_style(palette),
                onmounted: move |evt| async move {
                    if autofocus {
                        let _ = evt.set_focus(true).await;
                    }
                },
                onmousedown: move |evt| {
                    evt.stop_propagation();
                    on_focus_input.call(focus_key_on_mousedown.clone());
                },
                onclick: move |evt| {
                    evt.stop_propagation();
                    on_focus_input.call(focus_key_on_click.clone());
                },
                onfocus: move |_| on_focus_input.call(focus_key_on_focus.clone()),
                onblur: move |_| on_blur_input.call(()),
                onkeydown: move |evt| {
                    evt.stop_propagation();
                },
                oninput: move |evt| on_change.call(evt.value()),
            }
        }
    }
}
/// The one-line summary the settings rail shows in place of the flag values.
///
/// It counts CLIs that OWN a box (`extra_args_slug == slug`), so `codex-anything`
/// — which shares codex's — is not counted twice, and "customised" means the
/// user has stored something, not that a default tier exists.
fn launch_flags_rail_summary(stored: &std::collections::BTreeMap<String, String>) -> String {
    let total = launch_flags_rows().count();
    let customised = launch_flags_rows()
        .filter(|descriptor| stored.contains_key(descriptor.extra_args_slug))
        .count();
    format!("{total} CLIs · {customised} customised")
}

/// The one-line summary the settings rail shows beside the CLI-install button.
///
/// It reports THIS machine only, and says so, because that is the only host the
/// GUI can probe without reaching over ssh — and a summary that silently
/// averaged several machines would hide the exact fault this surface exists to
/// show, where one host carries every CLI and the host beside it carries none.
fn cli_install_rail_summary(stored_consent: &str) -> String {
    use yggterm_core::cli_install::{local_machine_status, InstallConsent};
    let status = local_machine_status("this machine");
    let total = status.rows.len();
    let present = status.present_count();
    match InstallConsent::from_wire(stored_consent) {
        InstallConsent::Undecided if present < total => format!("{present}/{total} here · not asked"),
        InstallConsent::Declined => format!("{present}/{total} here · declined"),
        _ => format!("{present}/{total} on this machine"),
    }
}

/// The CLIs that get a ROW in the launch-flags modal, in registry order.
///
/// ⛔ Derived, never hand-listed: `docs/spec-agent-cli-extra-args-modal.md` §1
/// makes this the place the "a CLI is DATA" law would otherwise stop being true,
/// and the titlebar `+` menu is the standing proof of what hand-rolling costs.
/// A CLI that does not own its box (it reads another's) is not a row — one CLI,
/// one box.
fn launch_flags_rows()
-> impl Iterator<Item = &'static yggterm_core::agent_cli::AgentCliDescriptor> {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .filter(|descriptor| descriptor.extra_args_slug == descriptor.slug)
}

/// What one row's box should show: the stored value, or the descriptor's default
/// tier when the user has never set one.
///
/// ⚠ An entry present but EMPTY is a user who cleared the box, and it must stay
/// cleared — `Some("")` is not `None`. That distinction is the whole reason the
/// store keys a map instead of defaulting a `String` field.
fn launch_flags_box_value(
    descriptor: &yggterm_core::agent_cli::AgentCliDescriptor,
    stored: &std::collections::BTreeMap<String, String>,
) -> String {
    if let Some(value) = stored.get(descriptor.extra_args_slug) {
        return value.clone();
    }
    descriptor
        .default_permission_preset()
        .map(|preset| preset.args.to_string())
        .unwrap_or_default()
}

#[component]
fn LaunchFlagsSettingsSection(
    palette: Palette,
    summary: String,
    on_open: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Agent CLI launch flags"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.muted),
                    "{summary}"
                }
            }
            button {
                "data-launch-flags-open-button": "1",
                style: format!(
                    "display:flex; align-items:center; justify-content:space-between; height:34px; padding:0 12px; \
                     border:none; border-radius:11px; background:{}; color:{}; \
                     box-shadow: inset 0 0 0 1px {}; font-size:12px; font-weight:700;",
                    if palette_is_dark(palette) {
                        "rgba(21,28,35,0.94)"
                    } else {
                        "rgba(255,255,255,0.86)"
                    },
                    palette.text,
                    if palette_is_dark(palette) {
                        "rgba(93,116,134,0.56)"
                    } else {
                        "rgba(208,219,229,0.85)"
                    }
                ),
                onclick: move |evt| on_open.call(evt),
                span { "Configure" }
                span { style: format!("color:{};", palette.muted), "↗" }
            }
            div {
                style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                "Flags appended to every new and resumed session of that CLI. Each CLI spells its permission checks differently."
            }
        }
    }
}

/// The machine list the CLI-installation modal draws, local first.
///
/// ⛔ **Only the local machine is PROBED.** The GUI can read its own `PATH`;
/// every other machine is reachable only over ssh, which is the provisioner's
/// path and not the renderer's. Remote hosts are therefore reported as
/// `Unknown`, which the modal renders as "not probed" — an honest blank rather
/// than an `Absent` that would make the primary button offer installs it cannot
/// perform against hosts it has not contacted.
fn cli_install_machines(
    snapshot: &SharedSnapshot,
) -> Vec<yggterm_core::cli_install::MachineCliStatus> {
    use yggterm_core::cli_install::{local_machine_status, CliPresence, MachineCliStatus};
    let mut machines = vec![local_machine_status("This machine")];
    machines.extend(snapshot.remote_machines.iter().map(|machine| {
        MachineCliStatus::build(
            machine.machine_key.clone(),
            machine.label.clone(),
            |_| CliPresence::Unknown,
        )
    }));
    machines
}

/// The CLI-installation modal: which agent CLIs are on which machine, what is
/// missing, and the licence acknowledgement that lets yggterm fetch them.
///
/// ⚖ **Why the consent banner is part of THIS surface and not a settings
/// toggle.** yggterm installs other people's programs — some by package
/// manager, one by piping a vendor's install script. The acknowledgement has to
/// sit where the user can see WHAT would be installed and WHERE, or it is
/// consent to an abstraction. Everything below the banner is that "what and
/// where", which is why the banner cannot be lifted out into a checkbox.
#[component]
fn CliInstallOverlay(
    palette: Palette,
    theme: UiTheme,
    machines: Vec<yggterm_core::cli_install::MachineCliStatus>,
    consent: yggterm_core::cli_install::InstallConsent,
    pending: bool,
    on_grant: EventHandler<MouseEvent>,
    on_decline: EventHandler<MouseEvent>,
    on_install_all: EventHandler<MouseEvent>,
    on_close: EventHandler<MouseEvent>,
) -> Element {
    use yggterm_core::cli_install::{plan_install_count, recommended_plans, ArrivalPlan, CliPresence};

    let overlay_wash = match theme {
        UiTheme::ZedLight => "rgba(228,237,245,0.03)",
        UiTheme::ZedDark => "rgba(10,14,18,0.05)",
    };
    let editor_surface = match theme {
        UiTheme::ZedLight => "rgb(248,252,255)",
        UiTheme::ZedDark => "rgb(28,34,41)",
    };
    let editor_shadow = match theme {
        UiTheme::ZedLight => {
            "0 0 0 1px rgba(215,229,243,0.96), 0 0 0 10px rgba(129,188,255,0.18), 0 26px 60px rgba(55,83,112,0.20), inset 0 0 0 1px rgba(214,223,232,0.92)"
        }
        UiTheme::ZedDark => {
            "0 0 0 1px rgba(59,87,112,0.90), 0 0 0 10px rgba(124,200,255,0.16), 0 26px 60px rgba(0,0,0,0.42), inset 0 0 0 1px rgba(68,84,99,0.94)"
        }
    };
    let plans = recommended_plans(&machines, consent);
    let pending_count = plan_install_count(&plans);
    // Counted independently of consent: the user must be able to SEE how much
    // work there is before deciding whether to authorise it. Gating this number
    // on consent would show "0 missing" to exactly the person being asked.
    let actionable_total: usize = machines
        .iter()
        .map(|machine| machine.installable().count())
        .sum();

    rsx! {
        div {
            "data-cli-install-overlay": "1",
            "data-yggterm-modal-root": "cli-install",
            style: format!(
                "position:fixed; inset:0; z-index:98; display:flex; align-items:center; justify-content:center; background:{};",
                overlay_wash
            ),
            onmousedown: move |evt| on_close.call(evt),
            onclick: move |evt| on_close.call(evt),
            div {
                "data-cli-install-shell": "1",
                style: format!(
                    "width:min(720px, calc(100vw - 44px)); max-height:calc(100vh - 56px); overflow:auto; \
                     display:flex; flex-direction:column; gap:12px; padding:16px; \
                     border-radius:22px; background:{}; color:{}; box-shadow:{}; font-family:{};",
                    editor_surface,
                    palette.text,
                    editor_shadow,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),

                div {
                    style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                    div {
                        style: "display:flex; flex-direction:column; gap:3px;",
                        div {
                            style: format!("font-size:15px; font-weight:800; letter-spacing:-0.01em; color:{};", palette.text),
                            "Agent CLI installation"
                        }
                        div {
                            style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                            "Every machine yggterm can reach should carry every agent CLI, so a session opens wherever you click. Pick what you want where."
                        }
                    }
                    button {
                        "data-cli-install-close": "1",
                        style: format!(
                            "border:none; background:transparent; color:{}; font-size:16px; font-weight:700; cursor:pointer;",
                            palette.muted
                        ),
                        onclick: move |evt| on_close.call(evt),
                        "✕"
                    }
                }

                if consent.should_offer() {
                    div {
                        "data-cli-install-consent": "1",
                        style: format!(
                            "display:flex; flex-direction:column; gap:8px; padding:12px; border-radius:14px; \
                             background:{}; box-shadow: inset 0 0 0 1px {};",
                            if palette_is_dark(palette) { "rgba(28,36,45,0.86)" } else { "rgba(248,250,252,0.94)" },
                            if palette_is_dark(palette) { "rgba(120,146,168,0.42)" } else { "rgba(206,217,228,0.9)" }
                        ),
                        div {
                            style: format!("font-size:12px; font-weight:800; color:{};", palette.text),
                            "These are third-party programs"
                        }
                        div {
                            style: format!("font-size:11px; line-height:1.5; color:{};", palette.muted),
                            "Each agent CLI is published by its own vendor under its own licence and terms, separate from yggterm. \
                             Installing one here fetches it from that vendor — by package manager, or by running the vendor's own \
                             install script — into your user account on the machine you choose. yggterm does not redistribute them \
                             and grants you no rights to them. Nothing is fetched until you agree."
                        }
                        div {
                            style: "display:flex; gap:8px; align-items:center;",
                            button {
                                "data-cli-install-grant": "1",
                                style: format!(
                                    "border:none; border-radius:10px; height:30px; padding:0 14px; font-size:12px; \
                                     font-weight:800; cursor:pointer; background:{}; color:#fff;",
                                    palette.accent
                                ),
                                onclick: move |evt| on_grant.call(evt),
                                "I agree — yggterm may install them"
                            }
                            button {
                                "data-cli-install-decline": "1",
                                style: format!(
                                    "border:none; border-radius:10px; height:30px; padding:0 14px; font-size:12px; \
                                     font-weight:700; cursor:pointer; background:transparent; color:{}; \
                                     box-shadow: inset 0 0 0 1px {};",
                                    palette.muted,
                                    if palette_is_dark(palette) { "rgba(120,146,168,0.42)" } else { "rgba(206,217,228,0.9)" }
                                ),
                                onclick: move |evt| on_decline.call(evt),
                                "Not now"
                            }
                        }
                    }
                }

                if consent == yggterm_core::cli_install::InstallConsent::Declined {
                    div {
                        "data-cli-install-declined": "1",
                        style: format!("font-size:11px; line-height:1.5; color:{};", palette.muted),
                        "You chose not to let yggterm install these. The diagnosis below still works — install anything you want by hand, or change your mind with the button at the bottom."
                    }
                }

                for machine in machines.iter() {
                    div {
                        key: "{machine.machine_key}",
                        "data-cli-install-machine": "{machine.machine_key}",
                        style: format!(
                            "display:flex; flex-direction:column; gap:6px; padding:10px 12px; border-radius:14px; \
                             box-shadow: inset 0 0 0 1px {};",
                            if palette_is_dark(palette) { "rgba(93,116,134,0.4)" } else { "rgba(214,224,234,0.9)" }
                        ),
                        div {
                            style: "display:flex; align-items:baseline; justify-content:space-between; gap:8px;",
                            div {
                                style: format!("font-size:12px; font-weight:800; color:{};", palette.text),
                                "{machine.display_label}"
                            }
                            div {
                                style: format!("font-size:10px; font-weight:700; color:{};", palette.muted),
                                "{machine.summary()}"
                            }
                        }
                        div {
                            style: "display:flex; flex-wrap:wrap; gap:6px;",
                            for row in machine.rows.iter() {
                                div {
                                    key: "{row.slug}",
                                    "data-cli-install-row": "{row.slug}",
                                    "data-cli-install-presence": match &row.presence {
                                        CliPresence::Present { .. } => "present",
                                        CliPresence::Absent => "absent",
                                        CliPresence::UnsupportedHere => "unsupported",
                                        CliPresence::Unknown => "unknown",
                                    },
                                    style: format!(
                                        "display:flex; align-items:center; gap:6px; padding:4px 9px; border-radius:9px; \
                                         font-size:11px; font-weight:700; background:{}; color:{};",
                                        if row.presence.is_present() {
                                            if palette_is_dark(palette) { "rgba(16,72,52,0.55)" } else { "rgba(222,246,235,0.95)" }
                                        } else if palette_is_dark(palette) {
                                            "rgba(38,30,20,0.62)"
                                        } else {
                                            "rgba(253,243,224,0.95)"
                                        },
                                        palette.text
                                    ),
                                    span { "{row.display_name}" }
                                    span {
                                        style: format!("font-size:10px; font-weight:700; color:{};", palette.muted),
                                        match (&row.presence, row.arrival) {
                                            (CliPresence::Present { version: Some(v) }, _) => v.clone(),
                                            (CliPresence::Present { version: None }, _) => "installed".to_string(),
                                            (CliPresence::UnsupportedHere, _) => "not on this platform".to_string(),
                                            (CliPresence::Unknown, _) => "not probed".to_string(),
                                            (CliPresence::Absent, ArrivalPlan::Unattended) => "missing".to_string(),
                                            (CliPresence::Absent, ArrivalPlan::NeedsHuman) => "install by hand".to_string(),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px; padding-top:2px;",
                    div {
                        style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                        if actionable_total == 0 {
                            "Nothing to install on the machines yggterm has probed."
                        } else {
                            "Recommended: install every CLI on every machine, so a session opens wherever you click."
                        }
                    }
                    button {
                        "data-cli-install-run": "1",
                        disabled: pending || pending_count == 0,
                        style: format!(
                            "border:none; border-radius:10px; height:32px; padding:0 14px; font-size:12px; font-weight:800; \
                             cursor:{}; background:{}; color:#fff; opacity:{};",
                            if pending || pending_count == 0 { "default" } else { "pointer" },
                            palette.accent,
                            if pending || pending_count == 0 { "0.5" } else { "1" }
                        ),
                        onclick: move |evt| on_install_all.call(evt),
                        if pending {
                            "Installing…"
                        } else if pending_count == 0 {
                            "Install all recommended"
                        } else {
                            "Install all recommended ({pending_count})"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CliInstallSettingsSection(
    palette: Palette,
    summary: String,
    on_open: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Agent CLI installation"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.muted),
                    "{summary}"
                }
            }
            button {
                "data-cli-install-open-button": "1",
                style: format!(
                    "display:flex; align-items:center; justify-content:space-between; height:34px; padding:0 12px; \
                     border:none; border-radius:11px; background:{}; color:{}; \
                     box-shadow: inset 0 0 0 1px {}; font-size:12px; font-weight:700;",
                    if palette_is_dark(palette) {
                        "rgba(21,28,35,0.94)"
                    } else {
                        "rgba(255,255,255,0.86)"
                    },
                    palette.text,
                    if palette_is_dark(palette) {
                        "rgba(93,116,134,0.56)"
                    } else {
                        "rgba(208,219,229,0.85)"
                    }
                ),
                onclick: move |evt| on_open.call(evt),
                span { "Diagnose" }
                span { style: format!("color:{};", palette.muted), "↗" }
            }
            div {
                style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                "Which agent CLIs are on which machine, and what is missing. These are third-party programs under their own licences — yggterm fetches them only after you say so."
            }
        }
    }
}

#[component]
fn ThemeSettingsSection(
    palette: Palette,
    selected_theme: UiTheme,
    accent: String,
    custom_stop_count: usize,
    /// The ALT+ KeyTip letters for the theme options while the Settings scope is
    /// open (empty otherwise) — the §4 "ALT,G then a letter" theme switch.
    light_tip: String,
    dark_tip: String,
    on_select: EventHandler<UiTheme>,
    on_open_editor: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Theme"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", accent),
                    if custom_stop_count == 0 { "System Gradient" } else { "Custom Gradient" }
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:8px; padding:9px; border-radius:14px; \
                     background:{}; box-shadow: inset 0 0 0 1px {};",
                    if palette_is_dark(palette) {
                        "rgba(255,255,255,0.04)"
                    } else {
                        "rgba(255,255,255,0.22)"
                    },
                    if palette_is_dark(palette) {
                        "rgba(141,160,178,0.16)"
                    } else {
                        "rgba(198,212,224,0.32)"
                    }
                ),
                div {
                    style: segmented_control_track_style(palette),
                    button {
                        style: segmented_control_segment_style(palette, selected_theme == UiTheme::ZedLight, true, false),
                        onclick: move |_| on_select.call(UiTheme::ZedLight),
                        span {
                            "data-keytip-node": "settings/theme.light",
                            "data-keytip-tip": "{light_tip}",
                            style: "display:none;",
                        }
                        "Light"
                    }
                    button {
                        style: segmented_control_segment_style(palette, selected_theme == UiTheme::ZedDark, true, false),
                        onclick: move |_| on_select.call(UiTheme::ZedDark),
                        span {
                            "data-keytip-node": "settings/theme.dark",
                            "data-keytip-tip": "{dark_tip}",
                            style: "display:none;",
                        }
                        "Dark"
                    }
                }
                button {
                    "data-theme-editor-open-button": "1",
                    style: format!(
                        "display:flex; align-items:center; justify-content:space-between; height:34px; padding:0 12px; \
                         border:none; border-radius:11px; background:{}; color:{}; \
                         box-shadow: inset 0 0 0 1px {}; font-size:12px; font-weight:700;",
                        if palette_is_dark(palette) {
                            "rgba(21,28,35,0.94)"
                        } else {
                            "rgba(255,255,255,0.86)"
                        },
                        palette.text,
                        if palette_is_dark(palette) {
                            "rgba(93,116,134,0.56)"
                        } else {
                            "rgba(208,219,229,0.85)"
                        }
                    ),
                    onclick: move |evt| on_open_editor.call(evt),
                    span { "Edit Theme" }
                    span { style: format!("color:{};", accent), "↗" }
                }
            }
        }
    }
}
#[component]
fn KeytipsSettingsSection(
    palette: Palette,
    accent: String,
    customized: bool,
    on_open_editor: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "ALT+ Keys"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", accent),
                    if customized { "Customized" } else { "Excel preset" }
                }
            }
            div {
                style: settings_section_card_style(palette),
                button {
                    "data-keytips-editor-open-button": "1",
                    style: format!(
                        "display:flex; align-items:center; justify-content:space-between; height:34px; padding:0 12px; \
                         border:none; border-radius:11px; background:{}; color:{}; \
                         box-shadow: inset 0 0 0 1px {}; font-size:12px; font-weight:700; cursor:pointer;",
                        if palette_is_dark(palette) {
                            "rgba(21,28,35,0.94)"
                        } else {
                            "rgba(255,255,255,0.86)"
                        },
                        palette.text,
                        if palette_is_dark(palette) {
                            "rgba(93,116,134,0.56)"
                        } else {
                            "rgba(208,219,229,0.85)"
                        }
                    ),
                    onclick: move |evt| on_open_editor.call(evt),
                    span { "Explore & edit KeyTips" }
                    span { style: format!("color:{};", accent), "↗" }
                }
                div {
                    style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                    "Tap ALT to show the KeyTips overlay, then press a highlighted letter. Rebind or reset the shortcuts here."
                }
            }
        }
    }
}
#[component]
fn ChromeBehaviorSettingsSection(
    palette: Palette,
    auto_hide_titlebar: bool,
    chrome_mirrored: bool,
    on_change: EventHandler<bool>,
    on_change_mirror: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "Window Chrome"
            }
            div {
                style: settings_section_card_style(palette),
                InlineSettingsToggleRow {
                    field_key: SETTINGS_FIRST_FIELD_KEY.to_string(),
                    label: "Auto-hide Titlebar".to_string(),
                    description: "Collapse the chrome to a top-edge hover strip and pin it while search or titlebar menus are active.".to_string(),
                    enabled: auto_hide_titlebar,
                    palette,
                    on_change,
                }
                InlineSettingsToggleRow {
                    field_key: SETTINGS_MIRROR_CHROME_FIELD_KEY.to_string(),
                    label: "Mirror Chrome".to_string(),
                    description: "Reflect the window about its centre: the session tree, its ☰ toggle, the view toggle and + move right; the rail and its buttons move left. The search box stays put.".to_string(),
                    enabled: chrome_mirrored,
                    palette,
                    on_change: on_change_mirror,
                }
            }
        }
    }
}
#[component]
fn InlineSettingsToggleRow(
    field_key: String,
    label: String,
    description: String,
    enabled: bool,
    palette: Palette,
    on_change: EventHandler<bool>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            "data-settings-field-key": "{field_key}",
            "data-settings-toggle-key": "{field_key}",
            "data-settings-toggle-enabled": if enabled { "true" } else { "false" },
            aria_pressed: if enabled { "true" } else { "false" },
            style: inline_toggle_row_button_style(palette, enabled),
            onclick: move |_| on_change.call(!enabled),
            div {
                style: "display:flex; flex-direction:column; gap:3px; min-width:0; flex:1 1 auto; pointer-events:none;",
                div {
                    style: format!("font-size:11px; font-weight:700; color:{};", palette.text),
                    "{label}"
                }
                div {
                    style: format!("font-size:10px; line-height:1.45; color:{}; text-wrap:pretty;", palette.muted),
                    "{description}"
                }
            }
            div {
                style: inline_toggle_affordance_style(enabled),
                span {
                    "data-settings-toggle-state": "1",
                    style: format!(
                        "font-size:10px; font-weight:700; color:{}; pointer-events:none;",
                        if enabled { palette.accent } else { palette.muted }
                    ),
                    if enabled { "On" } else { "Off" }
                }
                div {
                    style: inline_toggle_track_style(palette, enabled),
                    div {
                        style: inline_toggle_thumb_style(enabled),
                    }
                }
            }
        }
    }
}
#[component]
fn NotificationSettingsSection(
    palette: Palette,
    selected: NotificationDeliveryMode,
    sound_enabled: bool,
    on_select: EventHandler<NotificationDeliveryMode>,
    on_change: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Notifications"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.accent),
                    "In-App Recommended"
                }
            }
            div {
                style: settings_section_card_style(palette),
                div {
                    style: segmented_control_track_style(palette),
                    button {
                        style: segmented_control_segment_style(palette, selected == NotificationDeliveryMode::InApp, true, false),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::InApp),
                        "App"
                    }
                    button {
                        style: segmented_control_segment_style(palette, selected == NotificationDeliveryMode::Both, true, false),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::Both),
                        "Both"
                    }
                    button {
                        style: segmented_control_segment_style(palette, selected == NotificationDeliveryMode::System, true, false),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::System),
                        "System"
                    }
                }
                InlineSettingsToggleRow {
                    field_key: "notification-sound".to_string(),
                    label: "Sound".to_string(),
                    description: "Play a local notification sound when the shell surfaces in-app delivery.".to_string(),
                    enabled: sound_enabled,
                    palette,
                    on_change,
                }
            }
        }
    }
}
#[component]
fn TelemetrySettingsSection(
    palette: Palette,
    enabled: bool,
    db_path: String,
    on_change: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Terminal Telemetry"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.accent),
                    "Recommended"
                }
            }
            div {
                style: settings_section_card_style(palette),
                InlineSettingsToggleRow {
                    field_key: "terminal-telemetry".to_string(),
                    label: "Diagnostics".to_string(),
                    description: "Record terminal readiness, reconnect, input, and render fault events to local SQLite.".to_string(),
                    enabled,
                    palette,
                    on_change,
                }
                div {
                    style: format!(
                        "font-family:'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
                         font-size:9.5px; line-height:1.35; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                        palette.muted
                    ),
                    "{db_path}"
                }
            }
        }
    }
}
#[component]
fn PerfProfilingSettingsSection(
    palette: Palette,
    enabled: bool,
    on_change: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Performance Profiling"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.muted),
                    "Developer"
                }
            }
            div {
                style: settings_section_card_style(palette),
                InlineSettingsToggleRow {
                    field_key: "perf-profiling".to_string(),
                    label: "Profiling".to_string(),
                    description: "Time hot paths (terminal attach, persist, snapshot, requests) to perf-telemetry.jsonl. Inspect with `yggterm-headless server perf-summary`.".to_string(),
                    enabled,
                    palette,
                    on_change,
                }
                div {
                    style: format!(
                        "font-family:'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
                         font-size:9.5px; line-height:1.35; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                        palette.muted
                    ),
                    "~/.yggterm/perf-telemetry.jsonl"
                }
            }
        }
    }
}
#[component]
fn ZoomSettingRow(
    field_key: String,
    label: String,
    percent: i32,
    palette: Palette,
    on_focus_input: EventHandler<String>,
    on_blur_input: EventHandler<()>,
    on_decrease: EventHandler<MouseEvent>,
    on_increase: EventHandler<MouseEvent>,
    on_set_percent: EventHandler<i32>,
) -> Element {
    let focus_key_on_mousedown = field_key.clone();
    let focus_key_on_click = field_key.clone();
    let focus_key_on_focus = field_key.clone();
    let mut draft_percent = use_signal(|| percent.to_string());
    let mut focused = use_signal(|| false);
    // While the field is focused the user is mid-edit, so honor their draft;
    // otherwise mirror the canonical `percent` prop directly. Deriving the
    // display from focus state — rather than a use_effect that copies `percent`
    // into `draft` — is what makes a +/- step (or a keyboard zoom shortcut)
    // show up: those re-render with a fresh `percent` but touch neither
    // `focused` nor `draft`, so the old effect (which captured `percent` by
    // value and only re-fired on focus/draft reads) never updated the number
    // or the width-tracking pill. Reading `percent` here re-derives on every
    // render, so both stay in lockstep with the setting.
    let display_percent = if focused() {
        draft_percent()
    } else {
        percent.to_string()
    };
    // The number's pill snugly tracks its content: the field grows as more digits
    // are typed / stepped up and shrinks back down for smaller values
    // (user-reported 2026-06-28). Width is digit-count driven so "50" reads
    // narrower than "100" reads narrower than "1000".
    let zoom_input_digits = display_percent
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
        .max(1);
    let zoom_input_width_px = 22 + zoom_input_digits as i32 * 12;
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:4px;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "{label}"
            }
            div {
                style: format!(
                    "display:flex; align-items:center; justify-content:space-between; height:30px; padding:0 6px; \
                     border:none; border-radius:10px; background:{}; box-shadow: inset 0 0 0 1px {};",
                    if palette_is_dark(palette) {
                        "rgba(255,255,255,0.05)"
                    } else {
                        "rgba(255,255,255,0.34)"
                    },
                    if palette_is_dark(palette) {
                        "rgba(141,160,178,0.18)"
                    } else {
                        "rgba(198,212,224,0.34)"
                    }
                ),
                button {
                    style: zoom_button_style(palette),
                    onclick: move |evt| on_decrease.call(evt),
                    "−"
                }
                input {
                    "data-settings-field-key": "{field_key}",
                    "data-settings-zoom-input": "1",
                    r#type: "text",
                    inputmode: "numeric",
                    pattern: "[0-9]*",
                    value: "{display_percent}",
                    style: format!(
                        "min-width:0; width:{zoom_input_width_px}px; height:24px; border:none; border-radius:8px; \
                         background:{}; color:{}; outline:none; text-align:center; \
                         font-size:12px; font-weight:750; box-shadow: inset 0 0 0 1px {}; \
                         appearance:textfield; -moz-appearance:textfield; transition:width 120ms ease;",
                        if palette_is_dark(palette) {
                            "rgba(10,14,20,0.60)"
                        } else {
                            "rgba(255,255,255,0.66)"
                        },
                        palette.text,
                        if palette_is_dark(palette) {
                            "rgba(93,116,134,0.44)"
                        } else {
                            "rgba(198,212,224,0.48)"
                        }
                    ),
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        on_focus_input.call(focus_key_on_mousedown.clone());
                    },
                    onclick: move |evt| {
                        evt.stop_propagation();
                        on_focus_input.call(focus_key_on_click.clone());
                    },
                    onfocus: move |_| {
                        // Seed the draft from the canonical value: while unfocused the
                        // display mirrors `percent`, so editing must begin there (the
                        // draft may hold a stale value from a prior edit or never have
                        // caught a +/- step).
                        draft_percent.set(percent.to_string());
                        focused.set(true);
                        on_focus_input.call(focus_key_on_focus.clone());
                    },
                    onblur: move |_| {
                        focused.set(false);
                        let next = normalize_zoom_percent_text(&draft_percent(), percent);
                        draft_percent.set(next.to_string());
                        on_set_percent.call(next);
                        on_blur_input.call(());
                    },
                    onkeydown: move |evt| {
                        evt.stop_propagation();
                        match evt.key() {
                            Key::Enter => {
                                evt.prevent_default();
                                let next = normalize_zoom_percent_text(&draft_percent(), percent);
                                draft_percent.set(next.to_string());
                                on_set_percent.call(next);
                            }
                            Key::Character(ref chars) if !chars.chars().all(|ch| ch.is_ascii_digit()) => {
                                evt.prevent_default();
                            }
                            _ => {}
                        }
                    },
                    oninput: move |evt| {
                        draft_percent.set(sanitize_zoom_percent_text(&evt.value()));
                    },
                }
                button {
                    style: zoom_button_style(palette),
                    onclick: move |evt| on_increase.call(evt),
                    "+"
                }
            }
        }
    }
}
#[component]
fn TerminalThemeSettingRow(
    palette: Palette,
    light_value: String,
    dark_value: String,
    light_options: Vec<String>,
    dark_options: Vec<String>,
    on_change: EventHandler<(UiTheme, String)>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:4px; position:relative;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "Terminal Theme"
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px;",
                TerminalThemeSelectRow {
                    palette: palette,
                    icon: "☀".to_string(),
                    label: "Light".to_string(),
                    value: light_value,
                    options: light_options,
                    mode: UiTheme::ZedLight,
                    on_change: on_change,
                }
                TerminalThemeSelectRow {
                    palette: palette,
                    icon: "☾".to_string(),
                    label: "Dark".to_string(),
                    value: dark_value,
                    options: dark_options,
                    mode: UiTheme::ZedDark,
                    on_change: on_change,
                }
            }
        }
    }
}
#[component]
fn TerminalThemeSelectRow(
    palette: Palette,
    icon: String,
    label: String,
    value: String,
    options: Vec<String>,
    mode: UiTheme,
    on_change: EventHandler<(UiTheme, String)>,
) -> Element {
    let mut menu_open = use_signal(|| false);
    let mut filter_query = use_signal(String::new);
    let control_background = if palette_is_dark(palette) {
        "rgba(10,14,20,0.98)"
    } else {
        "rgba(255,255,255,0.94)"
    };
    let control_text = if palette_is_dark(palette) {
        "#f6fbff"
    } else {
        "#1f2b35"
    };
    let control_border = if palette_is_dark(palette) {
        "rgba(214,229,242,0.38)"
    } else {
        "rgba(201,214,226,0.56)"
    };
    let menu_background = if palette_is_dark(palette) {
        "rgba(13,19,27,0.99)"
    } else {
        "rgba(255,255,255,0.98)"
    };
    let mode_key = format!("{:?}", mode);
    let filter_value = filter_query();
    let filtered_options = filter_terminal_theme_options(&options, &filter_value);
    let empty_filter = filter_value.trim().is_empty();
    let option_count = filtered_options.len();
    let mut open_button_filter = filter_query;
    let mut open_button_menu = menu_open;
    let options_for_enter = options.clone();
    rsx! {
        div {
            style: format!(
                "display:grid; grid-template-columns:auto minmax(0,1fr); align-items:start; gap:10px; \
                 min-width:0; min-height:34px; padding:0;",
            ),
            div {
                style: format!(
                    "display:inline-flex; align-items:center; gap:6px; min-width:58px; font-size:11px; font-weight:700; color:{};",
                    palette.text
                ),
                span {
                    style: format!(
                        "display:inline-flex; width:18px; height:18px; align-items:center; justify-content:center; \
                         border-radius:999px; background:{}; color:{}; font-size:11px; box-shadow: inset 0 0 0 1px {};",
                        if palette_is_dark(palette) {
                            "rgba(255,255,255,0.08)"
                        } else {
                            "rgba(255,255,255,0.72)"
                        },
                        if palette_is_dark(palette) {
                            "#f3f8fd"
                        } else {
                            "#1f2b35"
                        },
                        if palette_is_dark(palette) {
                            "rgba(214,229,242,0.18)"
                        } else {
                            "rgba(201,214,226,0.56)"
                        }
                    ),
                    "{icon}"
                }
                span {
                    "{label}"
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:4px; min-width:0;",
                button {
                    r#type: "button",
                    "data-terminal-theme-button": "1",
                    "data-terminal-theme-mode": "{mode_key}",
                    style: format!(
                        "width:100%; height:34px; border:none; border-radius:10px; padding:0 9px 0 12px; \
                         display:flex; align-items:center; justify-content:space-between; gap:8px; min-width:0; \
                         background:{}; color:{}; box-shadow: inset 0 0 0 1px {}; \
                         font-size:12px; font-weight:700; text-align:left;",
                        control_background,
                        control_text,
                        control_border
                    ),
                    onclick: move |_| {
                        let next = !open_button_menu();
                        open_button_menu.set(next);
                        if next {
                            open_button_filter.set(String::new());
                        }
                    },
                    onkeydown: move |evt| {
                        evt.stop_propagation();
                        match evt.key() {
                            Key::Enter => {
                                evt.prevent_default();
                                menu_open.set(true);
                                filter_query.set(String::new());
                            }
                            Key::Character(ref chars) if chars == " " => {
                                evt.prevent_default();
                                menu_open.set(true);
                                filter_query.set(String::new());
                            }
                            Key::Character(ref chars) if chars.chars().all(|ch| !ch.is_control()) => {
                                evt.prevent_default();
                                menu_open.set(true);
                                filter_query.set(chars.to_string());
                            }
                            _ => {}
                        }
                    },
                    span {
                        style: "min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                        "{value}"
                    }
                    span {
                        style: format!("flex:0 0 auto; color:{};", palette.muted),
                        "⌄"
                    }
                }
                if menu_open() {
                    div {
                        "data-terminal-theme-menu": "1",
                        "data-terminal-theme-mode": "{mode_key}",
                        "data-terminal-theme-filter": "{filter_value}",
                        "data-terminal-theme-option-count": "{option_count}",
                        style: format!(
                            "display:flex; flex-direction:column; gap:4px; max-height:min(236px, 54vh); overflow:auto; \
                             border-radius:10px; padding:5px; background:{}; box-shadow: inset 0 0 0 1px {}; scroll-margin-block:16px;",
                            menu_background,
                            control_border
                        ),
                        onmounted: {
                            let mode_key = mode_key.clone();
                            move |evt| {
                                let mode_key = mode_key.clone();
                                async move {
                                    let _ = evt.set_focus(true).await;
                                    let _ = document::eval(&format!(
                                        r#"
                                    setTimeout(() => {{
                                        const menu = document.querySelector('[data-terminal-theme-menu="1"][data-terminal-theme-mode="{mode_key}"]');
                                        if (!menu) return;
                                        menu.scrollIntoView({{ block: 'nearest', inline: 'nearest', behavior: 'smooth' }});
                                        const input = menu.querySelector('[data-terminal-theme-filter-input="1"]');
                                        if (input && typeof input.focus === 'function') {{
                                            input.focus({{ preventScroll: true }});
                                            if (typeof input.select === 'function') {{
                                                input.select();
                                            }}
                                        }}
                                    }}, 0);
                                    "#
                                    ));
                                }
                            }
                        },
                        input {
                            "data-terminal-theme-filter-input": "1",
                            "data-terminal-theme-mode": "{mode_key}",
                            r#type: "text",
                            value: "{filter_value}",
                            placeholder: "Filter themes",
                            style: format!(
                                "flex:0 0 auto; height:28px; min-height:28px; padding:0 9px; border:none; border-radius:8px; background:{}; color:{}; \
                                 outline:none; box-shadow: inset 0 0 0 1px {}; font-size:12px; font-weight:650;",
                                control_background,
                                control_text,
                                control_border
                            ),
                            onmousedown: |evt| evt.stop_propagation(),
                            onclick: |evt| evt.stop_propagation(),
                            onkeydown: move |evt| {
                                evt.stop_propagation();
                                if evt.key() == Key::Escape {
                                    evt.prevent_default();
                                    menu_open.set(false);
                                } else if evt.key() == Key::Enter {
                                    evt.prevent_default();
                                    if let Some(first) = filter_terminal_theme_options(&options_for_enter, &filter_query()).into_iter().next() {
                                        on_change.call((mode, first));
                                        menu_open.set(false);
                                    }
                                }
                            },
                            oninput: move |evt| filter_query.set(evt.value()),
                        }
                        if option_count == 0 {
                            div {
                                "data-terminal-theme-empty": "1",
                                style: format!("padding:7px 8px; color:{}; font-size:12px; font-weight:600;", palette.muted),
                                "No matching themes"
                            }
                        }
                        for option in filtered_options {
                            {
                                let selected = option == value;
                                let option_for_click = option.clone();
                                rsx! {
                                    button {
                                        key: "{label}:{option}",
                                        r#type: "button",
                                        style: format!(
                                            "width:100%; min-height:28px; border:none; border-radius:7px; padding:0 8px; \
                                             text-align:left; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; \
                                             background:{}; color:{}; font-size:12px; font-weight:{};",
                                            if selected {
                                                palette.accent_soft
                                            } else {
                                                "transparent"
                                            },
                                            control_text,
                                            if selected { 700 } else { 600 }
                                        ),
                                        onclick: move |_| {
                                            on_change.call((mode, option_for_click.clone()));
                                            menu_open.set(false);
                                            filter_query.set(String::new());
                                        },
                                        onkeydown: move |evt| evt.stop_propagation(),
                                        if selected && empty_filter {
                                            span {
                                                "data-terminal-theme-selected-option": "1",
                                                "{option}"
                                            }
                                        } else {
                                            "{option}"
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
fn terminal_theme_value_for_settings(value: &str, theme: UiTheme) -> String {
    let value = value.trim();
    if value.is_empty() {
        default_terminal_theme_name(theme).to_string()
    } else {
        value.to_string()
    }
}
fn filter_terminal_theme_options(options: &[String], query: &str) -> Vec<String> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return options.to_vec();
    }
    options
        .iter()
        .filter(|option| option.to_ascii_lowercase().contains(&query))
        .cloned()
        .collect()
}
#[component]
fn MetadataGroup(title: String, entries: Vec<SessionMetadataEntry>, palette: Palette) -> Element {
    // Every group collapses. The rail stacks Session + Runtime + History + Client + Daemon,
    // and the Daemon group in particular now carries per-blocker detail — enough that a
    // reader hunting one fact had to scroll past everything else. Collapse state is local
    // to the group and survives re-renders (hook state is keyed by position), so a user who
    // folds "History" away keeps it folded while they switch sessions.
    let mut expanded = use_signal(|| true);
    let is_expanded = expanded();
    let entry_count = entries.len();
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px; padding-bottom:4px;",
            div {
                style: "display:flex; align-items:center; gap:6px; cursor:pointer; user-select:none;",
                "data-metadata-group-toggle": "{title}",
                "data-metadata-group-expanded": if is_expanded { "1" } else { "0" },
                onclick: move |_| expanded.toggle(),
                span {
                    style: format!(
                        "font-size:9px; line-height:1; color:{}; transform:rotate({}deg); \
                         transition:transform 120ms ease;",
                        palette.muted,
                        if is_expanded { 90 } else { 0 },
                    ),
                    "▶"
                }
                RailSectionTitle { title: title.clone(), muted_color: palette.muted.to_string() }
                // Collapsed, the group must still say how much it is hiding — a bare title
                // with no affordance reads as an empty section, not a folded one.
                if !is_expanded {
                    span {
                        style: format!("font-size:10px; font-weight:600; color:{};", palette.muted),
                        "{entry_count}"
                    }
                }
            }
            if is_expanded {
            for entry in entries.into_iter() {
                div {
                    style: "display:flex; flex-direction:column; gap:2px;",
                    span {
                        style: format!(
                            "font-size:11px; font-weight:600; color:{}; text-rendering:optimizeLegibility; \
                             -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            palette.muted
                        ),
                        "{entry.label}"
                    }
                    span {
                        style: format!(
                            "font-size:12px; font-weight:500; color:{}; white-space:pre-wrap; line-height:1.5; \
                             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            palette.text
                        ),
                        "{entry.value}"
                    }
                }
            }
            }
        }
    }
}
#[component]
fn InstallUpdateRow(
    update_call_to_action: UpdateCallToAction,
    version: String,
    palette: Palette,
    on_trigger_update: EventHandler<MouseEvent>,
) -> Element {
    let is_busy =
        update_call_to_action.mode == "checking" || update_call_to_action.mode == "updating";
    let button_background = if is_busy {
        palette.accent_soft
    } else if update_call_to_action.mode == "restart" {
        palette.accent
    } else {
        palette.panel
    };
    let button_text = if update_call_to_action.mode == "restart" {
        "#f6fbff"
    } else {
        palette.text
    };
    rsx! {
        div {
            "data-install-update-row": "1",
            style: format!(
                "display:flex; flex-direction:column; gap:10px; padding:12px; border-radius:14px; \
                 background:{}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.42);",
                palette.panel_alt
            ),
            div {
                style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                div {
                    style: "display:flex; flex-direction:column; gap:2px; min-width:0;",
                    span {
                        style: format!("font-size:11px; font-weight:600; color:{};", palette.muted),
                        "Version"
                    }
                    span {
                        style: format!("font-size:12px; font-weight:600; color:{};", palette.text),
                        "{version}"
                    }
                }
                button {
                    "data-install-update-button": "1",
                    "data-install-update-mode": "{update_call_to_action.mode}",
                    disabled: update_call_to_action.disabled,
                    style: format!(
                        "display:inline-flex; align-items:center; justify-content:center; gap:6px; min-height:30px; padding:0 12px; \
                         border:none; border-radius:10px; background:{}; color:{}; font-size:11px; font-weight:700; \
                         box-shadow: inset 0 0 0 1px {}; cursor:{}; white-space:nowrap; opacity:{};",
                        button_background,
                        button_text,
                        chrome_chip_border(palette),
                        if update_call_to_action.disabled { "default" } else { "pointer" },
                        if update_call_to_action.disabled { "0.92" } else { "1" },
                    ),
                    onclick: move |evt| on_trigger_update.call(evt),
                    if update_call_to_action.mode == "updating" {
                        span { "{update_call_to_action.label}" }
                        span {
                            style: "display:inline-flex; align-items:center; gap:1px;",
                            for dot_ix in 0..3 {
                                span {
                                    key: "{dot_ix}",
                                    style: format!(
                                        "display:inline-block; width:4px; animation:yggterm-update-ellipsis-pulse 1s ease-in-out infinite; animation-delay:{}ms;",
                                        dot_ix * 120
                                    ),
                                    "."
                                }
                            }
                        }
                        if let Some(percent) = update_call_to_action.progress_percent {
                            span { "{percent}%" }
                        }
                    } else {
                        "{update_call_to_action.label}"
                    }
                }
            }
            div {
                "data-install-update-detail": "1",
                style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                "{update_call_to_action.detail}"
            }
        }
    }
}
fn palette(theme: UiTheme) -> Palette {
    match theme {
        UiTheme::ZedLight => Palette {
            shell: "#f3f7fa",
            titlebar: "transparent",
            sidebar: "transparent",
            sidebar_hover: "rgba(134,186,202,0.14)",
            panel: "#ffffff",
            panel_alt: "rgba(255,255,255,0.18)",
            border: "#dfe5ea",
            text: "#24303a",
            muted: "#6f7c86",
            accent: "#2f7cf6",
            accent_soft: "rgba(114,190,215,0.18)",
            gradient: "linear-gradient(180deg, rgb(232, 243, 248) 0%, rgb(232, 244, 238) 48%, rgb(237, 240, 244) 100%)",
            close_hover: "#e81123",
            control_hover: "rgba(36,48,58,0.10)",
            shadow: "0 14px 30px rgba(72,102,118,0.10)",
            panel_shadow: "0 10px 24px rgba(69,108,136,0.10)",
        },
        UiTheme::ZedDark => Palette {
            shell: "#272e34",
            titlebar: "transparent",
            sidebar: "transparent",
            sidebar_hover: "rgba(124,200,255,0.12)",
            panel: "#161c22",
            panel_alt: "rgba(255,255,255,0.05)",
            border: "#2d3946",
            text: "#dde8f3",
            muted: "#c9d5e0",
            accent: "#7cc8ff",
            accent_soft: "rgba(124,200,255,0.16)",
            gradient: "linear-gradient(180deg, rgb(56, 79, 91) 0%, rgb(55, 88, 79) 54%, rgb(39, 46, 52) 100%)",
            close_hover: "#e81123",
            control_hover: "rgba(255,255,255,0.10)",
            shadow: "0 16px 38px rgba(0,0,0,0.24)",
            panel_shadow: "0 12px 28px rgba(0,0,0,0.18)",
        },
    }
}
fn linux_compositor_blur_active() -> bool {
    #[cfg(target_os = "linux")]
    {
        if linux_gtk_backend_is_x11() {
            LINUX_COMPOSITOR_BLUR_ACTIVE.load(Ordering::SeqCst)
                && LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.load(Ordering::SeqCst)
        } else {
            LINUX_COMPOSITOR_BLUR_ACTIVE.load(Ordering::SeqCst)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
#[cfg(target_os = "linux")]
fn linux_x11_compositor_blur_xid() -> u32 {
    LINUX_X11_COMPOSITOR_BLUR_XID.load(Ordering::SeqCst)
}
#[cfg(not(target_os = "linux"))]
fn linux_x11_compositor_blur_xid() -> u32 {
    0
}
#[cfg(target_os = "linux")]
fn linux_x11_compositor_blur_property_present_cached() -> bool {
    LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.load(Ordering::SeqCst)
}
#[cfg(not(target_os = "linux"))]
fn linux_x11_compositor_blur_property_present_cached() -> bool {
    false
}
fn material_blur_filter(material_blur_px: f32, saturate: f32) -> String {
    format!(
        "blur({:.1}px) saturate({:.0}%)",
        material_blur_px.clamp(10.0, 42.0),
        saturate.clamp(1.0, 2.0) * 100.0
    )
}
fn shell_backdrop_style(
    maximized: bool,
    transparent_window: bool,
    material_blur_px: f32,
) -> String {
    if maximized
        || !transparent_window
        || !shell_live_blur_supported()
        || !shell_full_window_css_blur_enabled()
    {
        "none".to_string()
    } else {
        material_blur_filter(material_blur_px, 1.50)
    }
}
fn overlay_backdrop_style_for_platform(
    default_style: &'static str,
    css_backdrop_enabled: bool,
    native_compositor_blur_active: bool,
) -> &'static str {
    if css_backdrop_enabled && !native_compositor_blur_active {
        default_style
    } else {
        "none"
    }
}
fn overlay_backdrop_style(default_style: &'static str) -> &'static str {
    overlay_backdrop_style_for_platform(
        default_style,
        shell_css_backdrop_filter_enabled(),
        linux_compositor_blur_active(),
    )
}
fn shell_effective_radius_for_platform(
    radius: u8,
    maximized: bool,
    transparent_window: bool,
    native_shape_supported: bool,
) -> u8 {
    if maximized {
        return 0;
    }
    if cfg!(target_os = "linux") && !transparent_window && !native_shape_supported {
        return 0;
    }
    radius
}
fn shell_effective_radius(radius: u8, maximized: bool, transparent_window: bool) -> u8 {
    shell_effective_radius_for_platform(
        radius,
        maximized,
        transparent_window,
        linux_native_window_shape_supported(),
    )
}
#[cfg(target_os = "linux")]
fn linux_startup_reveal_should_wait_for_shape_for_platform(
    transparent_window: bool,
    radius: u8,
    maximized: bool,
    native_decorations_forced: bool,
    native_shape_supported: bool,
) -> bool {
    transparent_window
        && radius > 0
        && !maximized
        && !native_decorations_forced
        && native_shape_supported
}
#[cfg(target_os = "linux")]
fn linux_startup_reveal_should_wait_for_shape(
    transparent_window: bool,
    radius: u8,
    maximized: bool,
) -> bool {
    linux_startup_reveal_should_wait_for_shape_for_platform(
        transparent_window,
        radius,
        maximized,
        linux_force_native_decorations(),
        linux_native_window_shape_supported(),
    )
}
fn shell_root_background(palette: Palette, transparent_window: bool) -> &'static str {
    if transparent_window {
        "transparent"
    } else if shell_uses_opaque_linux_paint(transparent_window) {
        shell_opaque_fill(palette)
    } else {
        palette.shell
    }
}
fn linux_window_chrome_apply_needed(
    last: Option<LinuxWindowChromeApplySignature>,
    next: LinuxWindowChromeApplySignature,
) -> bool {
    last != Some(next)
}
fn shell_uses_opaque_linux_paint(transparent_window: bool) -> bool {
    cfg!(target_os = "linux") && !transparent_window
}
fn shell_opaque_fill(palette: Palette) -> &'static str {
    if palette_is_dark(palette) {
        "#272e34"
    } else {
        "#f3f7fa"
    }
}
fn shell_transparent_material_fill(
    shell_tint: &str,
    chrome_material_tint: &str,
    maximized: bool,
    transparent_window: bool,
    live_blur_supported: bool,
) -> String {
    if transparent_window && !maximized && live_blur_supported {
        shell_tint.to_string()
    } else if transparent_window && !maximized {
        let _ = chrome_material_tint;
        "var(--yggterm-opaque-shell-fill, #f3f7fa)".to_string()
    } else {
        String::new()
    }
}
/// The app-background paint (fill + gradient) — computed ONCE, painted twice:
/// by `shell_style` on the frame (the everyday pixels) and by
/// `shell_background_layer_style` on the dedicated `data-yggterm-app-bg`
/// layer that takes the under-glass clip-path hole treatment. Sharing the
/// computation is what keeps the two paints from diverging.
fn shell_background_paint(
    palette: Palette,
    shell_tint: &str,
    chrome_material_tint: &str,
    shell_gradient: &str,
    maximized: bool,
    transparent_window: bool,
) -> (String, String) {
    let opaque_linux_paint = shell_uses_opaque_linux_paint(transparent_window);
    let transparent_material_fill = shell_transparent_material_fill(
        shell_tint,
        chrome_material_tint,
        maximized,
        transparent_window,
        shell_live_blur_supported(),
    );
    let fill = if opaque_linux_paint {
        shell_opaque_fill(palette).to_string()
    } else if !transparent_material_fill.is_empty() {
        transparent_material_fill
    } else {
        palette.shell.to_string()
    };
    let gradient = if opaque_linux_paint {
        "none".to_string()
    } else {
        shell_gradient.to_string()
    };
    (fill, gradient)
}
/// The dedicated app-background layer (`data-yggterm-app-bg`): z-index:-1
/// first child of the shell frame, replicating the frame's background paint
/// exactly. Invisible in legacy stacking (it sits beneath the frame's own
/// identical paint); under glass the frame/root paints clear (CSS) and this
/// layer — the only element with no children — safely takes the evenodd
/// `clip-path` page holes via `--yggterm-under-glass-holes`. Hole coords are
/// window-logical px; on Linux the frame is flush (inset 0) so the layer box
/// IS the window box — the one platform under-glass runs on today.
fn shell_background_layer_style(
    palette: Palette,
    radius: u8,
    shell_tint: &str,
    chrome_material_tint: &str,
    shell_gradient: &str,
    shell_gradient_background_size: &str,
    shell_gradient_background_repeat: &str,
    maximized: bool,
    transparent_window: bool,
) -> String {
    let (fill, gradient) = shell_background_paint(
        palette,
        shell_tint,
        chrome_material_tint,
        shell_gradient,
        maximized,
        transparent_window,
    );
    let effective_radius = if maximized { 0 } else { radius };
    format!(
        "position:absolute; inset:0; z-index:-1; pointer-events:none; border-radius:{}px; \
         background-color:{}; background-image:{}; background-size:{}; background-repeat:{}; background-clip:padding-box;",
        effective_radius,
        fill,
        gradient,
        shell_gradient_background_size,
        shell_gradient_background_repeat,
    )
}
fn shell_style(
    palette: Palette,
    radius: u8,
    shell_tint: &str,
    chrome_material_tint: &str,
    shell_gradient: &str,
    shell_gradient_background_size: &str,
    shell_gradient_background_repeat: &str,
    shell_material_blur_px: f32,
    maximized: bool,
    transparent_window: bool,
) -> String {
    let backdrop = shell_backdrop_style(maximized, transparent_window, shell_material_blur_px);
    let (effective_shell_fill, effective_shell_gradient) = shell_background_paint(
        palette,
        shell_tint,
        chrome_material_tint,
        shell_gradient,
        maximized,
        transparent_window,
    );
    let exported_shell_gradient = shell_gradient;
    let stable_transparent_fill = shell_opaque_fill(palette).to_string();
    let chrome_tint = if transparent_window && !maximized && !shell_live_blur_supported() {
        stable_transparent_fill.clone()
    } else if transparent_window && !maximized {
        chrome_material_tint.to_string()
    } else if palette_is_dark(palette) {
        "#272e34".to_string()
    } else {
        "#f3f7fa".to_string()
    };
    let native_window_flush_shell = cfg!(any(target_os = "linux", target_os = "macos"));
    let frame_inset = if maximized || native_window_flush_shell {
        0.0
    } else {
        SHELL_FRAME_INSET_PX
    };
    let effective_radius = if maximized { 0 } else { radius };
    let suppress_outer_shadow = cfg!(any(target_os = "linux", target_os = "macos"));
    let frame_outline = if maximized || suppress_outer_shadow {
        "none".to_string()
    } else if palette_is_dark(palette) {
        "0 0 0 1px rgba(86,108,129,0.32)".to_string()
    } else {
        "0 0 0 1px rgba(206,218,229,0.58)".to_string()
    };
    let box_shadow = if suppress_outer_shadow {
        "none".to_string()
    } else if frame_outline == "none" {
        palette.shadow.to_string()
    } else {
        format!("{}, {}", frame_outline, palette.shadow)
    };
    let frame_clip = if effective_radius > 0 {
        format!("inset(0 round {}px)", effective_radius)
    } else {
        "none".to_string()
    };
    format!(
        "position:absolute; inset:{}px; display:flex; flex-direction:column; overflow:hidden; \
         border-radius:{}px; background-color:{}; background-image:{}; background-size:{}; background-repeat:{}; box-shadow:{}; background-clip:padding-box; backdrop-filter:{}; \
         -webkit-backdrop-filter:{}; clip-path:{}; -webkit-clip-path:{}; font-family:{}; \
         --yggterm-panel-color:{}; --yggterm-border-color:{}; --yggterm-shell-fill:{}; --yggterm-opaque-shell-fill:{}; --yggterm-shell-tint:{}; --yggterm-shell-gradient:{}; --yggterm-shell-background-size:{}; --yggterm-shell-background-repeat:{}; --yggterm-chrome-tint:{};",
        frame_inset,
        effective_radius,
        effective_shell_fill,
        effective_shell_gradient,
        shell_gradient_background_size,
        shell_gradient_background_repeat,
        box_shadow,
        backdrop,
        backdrop,
        frame_clip,
        frame_clip,
        interface_font_family(),
        palette.panel,
        palette.border,
        effective_shell_fill,
        stable_transparent_fill,
        shell_tint,
        exported_shell_gradient,
        shell_gradient_background_size,
        shell_gradient_background_repeat,
        chrome_tint
    )
}
#[cfg(target_os = "linux")]
fn rounded_window_row_inset(radius: i32, y_from_edge: i32) -> i32 {
    if radius <= 0 {
        return 0;
    }
    let dy = radius as f64 - (y_from_edge as f64 + 0.5);
    let chord = ((radius * radius) as f64 - dy * dy).max(0.0).sqrt();
    (radius as f64 - chord).ceil() as i32
}
#[cfg(target_os = "linux")]
fn rounded_window_region(width: i32, height: i32, radius: i32) -> cairo::Region {
    let region = cairo::Region::create();
    let radius = radius.max(0).min(width / 2).min(height / 2);
    for y in 0..height {
        let inset = if y < radius {
            rounded_window_row_inset(radius, y)
        } else if y >= height - radius {
            rounded_window_row_inset(radius, height - 1 - y)
        } else {
            0
        };
        let row_width = (width - inset * 2).max(0);
        if row_width > 0 {
            let _ = region.union_rectangle(&cairo::RectangleInt::new(inset, y, row_width, 1));
        }
    }
    region
}
#[cfg(target_os = "linux")]
fn apply_linux_transparent_window_surface_style(
    desktop: &dioxus::desktop::DesktopContext,
    transparent_window: bool,
    radius: u8,
) {
    use gtk::prelude::*;

    if !transparent_window {
        return;
    }

    let radius_px = u32::from(radius);
    let provider = gtk::CssProvider::new();
    let css = format!(
        "
        window.yggterm-transparent-window,
        window.yggterm-transparent-window decoration,
        window.yggterm-transparent-window > box,
        window.yggterm-transparent-window box {{
            background-color: transparent;
            background-image: none;
            box-shadow: none;
            border-radius: {radius_px}px;
            background-clip: padding-box;
        }}
    "
    );
    let _ = provider.load_from_data(css.as_bytes());

    let gtk_window = desktop.gtk_window();
    if let Some(screen) = gtk::prelude::GtkWindowExt::screen(gtk_window)
        && let Some(visual) = screen.rgba_visual()
    {
        gtk_window.set_visual(Some(&visual));
    }
    gtk_window
        .style_context()
        .add_class("yggterm-transparent-window");
    gtk_window
        .style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    gtk_window.set_app_paintable(true);

    if let Some(vbox) = desktop.default_vbox() {
        vbox.style_context().add_class("yggterm-transparent-window");
        vbox.style_context()
            .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        vbox.set_app_paintable(true);
    }
}
#[cfg(not(target_os = "linux"))]
fn apply_linux_transparent_window_surface_style(
    _desktop: &dioxus::desktop::DesktopContext,
    _transparent_window: bool,
    _radius: u8,
) {
}
#[cfg(target_os = "linux")]
fn prepare_linux_window_reveal_after_corner_shape(desktop: &dioxus::desktop::DesktopContext) {
    use gtk::prelude::*;

    desktop.gtk_window().set_opacity(0.0);
}
#[cfg(target_os = "linux")]
const LINUX_TRANSPARENT_WINDOW_PRE_REVEAL_RECONFIGURE_MS: u64 = 190;
#[cfg(target_os = "linux")]
fn schedule_linux_transparent_window_pre_reveal_reconfigure(
    desktop: &dioxus::desktop::DesktopContext,
    radius: u8,
    maximized: bool,
    trace_home: PathBuf,
) -> u64 {
    let desktop = desktop.clone();
    gtk::glib::timeout_add_local_once(Duration::from_millis(16), move || {
        let size = desktop.inner_size();
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "transparent_window_pre_reveal_reconfigure_begin",
            json!({
                "pid": std::process::id(),
                "width": size.width,
                "height": size.height,
                "radius": radius,
            }),
        );
        if size.width <= 2 || size.height <= 2 {
            append_trace_event(
                &trace_home,
                "ui",
                "startup",
                "transparent_window_pre_reveal_reconfigure_skipped",
                json!({
                    "pid": std::process::id(),
                    "width": size.width,
                    "height": size.height,
                }),
            );
            return;
        }
        apply_linux_transparent_window_surface_style(&desktop, true, radius);
        apply_linux_window_corner_shape(&desktop, radius, maximized);
        desktop.request_redraw();
        desktop.set_inner_size(LogicalSize::new(
            f64::from(size.width + 1),
            f64::from(size.height),
        ));
        let restore_desktop = desktop.clone();
        let restore_trace_home = trace_home.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(72), move || {
            restore_desktop.set_inner_size(LogicalSize::new(
                f64::from(size.width),
                f64::from(size.height),
            ));
            apply_linux_transparent_window_surface_style(&restore_desktop, true, radius);
            apply_linux_window_corner_shape(&restore_desktop, radius, maximized);
            restore_desktop.request_redraw();
            append_trace_event(
                &restore_trace_home,
                "ui",
                "startup",
                "transparent_window_pre_reveal_reconfigure_end",
                json!({
                    "pid": std::process::id(),
                    "width": size.width,
                    "height": size.height,
                    "radius": radius,
                }),
            );
        });
    });
    LINUX_TRANSPARENT_WINDOW_PRE_REVEAL_RECONFIGURE_MS
}
#[cfg(target_os = "linux")]
fn reveal_linux_window_after_corner_shape(
    desktop: &dioxus::desktop::DesktopContext,
    radius: u8,
    maximized: bool,
    trace_home: PathBuf,
    initial_delay_ms: u64,
) {
    use gtk::prelude::*;

    let revealed = std::rc::Rc::new(std::cell::Cell::new(false));
    for delay_ms in [0_u64, 8, 16, 32, 64, 120, 240, 480] {
        let effective_delay_ms = delay_ms.saturating_add(initial_delay_ms);
        let desktop = desktop.clone();
        let gtk_window = desktop.gtk_window().clone();
        let revealed = revealed.clone();
        let trace_home = trace_home.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(effective_delay_ms), move || {
            if revealed.get() {
                return;
            }
            let allocation = gtk_window.allocation();
            let has_window = gtk_window.window().is_some();
            let can_shape = allocation.width() > 0 && allocation.height() > 0 && has_window;
            if can_shape {
                apply_linux_window_corner_shape(&desktop, radius, maximized);
                gtk_window.queue_draw();
                gtk_window.set_opacity(1.0);
                revealed.set(true);
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "linux_window_revealed_after_corner_shape",
                    json!({
                        "pid": std::process::id(),
                        "delay_ms": effective_delay_ms,
                        "initial_delay_ms": initial_delay_ms,
                        "width": allocation.width(),
                        "height": allocation.height(),
                        "radius": radius,
                    }),
                );
                return;
            }
            if delay_ms >= 480 {
                gtk_window.set_opacity(1.0);
                revealed.set(true);
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "linux_window_revealed_after_corner_shape_fallback",
                    json!({
                        "pid": std::process::id(),
                        "delay_ms": effective_delay_ms,
                        "initial_delay_ms": initial_delay_ms,
                        "width": allocation.width(),
                        "height": allocation.height(),
                        "has_window": has_window,
                        "radius": radius,
                    }),
                );
            }
        });
    }
}
#[cfg(target_os = "linux")]
thread_local! {
    static LINUX_WAYLAND_BLUR_STATE: RefCell<Option<LinuxWaylandBlurState>> = const { RefCell::new(None) };
}
#[cfg(target_os = "linux")]
struct LinuxWaylandBlurAppState {
    capabilities: Arc<AtomicU32>,
}
#[cfg(target_os = "linux")]
struct LinuxWaylandBlurState {
    conn: wayland_client::Connection,
    event_queue: wayland_client::EventQueue<LinuxWaylandBlurAppState>,
    app_state: LinuxWaylandBlurAppState,
    _globals: wayland_client::globals::GlobalList,
    _compositor: wayland_client::protocol::wl_compositor::WlCompositor,
    backend: LinuxWaylandBlurBackend,
    last_width: i32,
    last_height: i32,
    apply_count: u64,
}
#[cfg(target_os = "linux")]
enum LinuxWaylandBlurBackend {
    ExtBackgroundEffect {
        _manager: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        effect: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
    },
    KdeKwinBlur {
        _manager: wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
        blur: wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur,
    },
}
#[cfg(target_os = "linux")]
impl LinuxWaylandBlurBackend {
    fn protocol_name(&self) -> &'static str {
        match self {
            Self::ExtBackgroundEffect { .. } => "ext-background-effect-v1",
            Self::KdeKwinBlur { .. } => "org_kde_kwin_blur_manager",
        }
    }
}
#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &wayland_client::globals::GlobalListContents,
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl wayland_client::Dispatch<wayland_client::protocol::wl_compositor::WlCompositor, ()>
    for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_compositor::WlCompositor,
        _event: wayland_client::protocol::wl_compositor::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl wayland_client::Dispatch<wayland_client::protocol::wl_region::WlRegion, ()>
    for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_region::WlRegion,
        _event: wayland_client::protocol::wl_region::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        Arc<AtomicU32>,
    > for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        event: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::Event,
        data: &Arc<AtomicU32>,
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        use wayland_client::WEnum;
        use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
            Capability, Event,
        };

        let Event::Capabilities { flags } = event else {
            return;
        };
        let bits = match flags {
            WEnum::Value(flags) => flags.bits(),
            WEnum::Unknown(bits) => bits,
        };
        if bits & Capability::Blur.bits() != 0 {
            data.store(bits, Ordering::SeqCst);
        } else {
            data.store(0, Ordering::SeqCst);
        }
    }
}
#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
        (),
    > for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
        _event: wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
        (),
    > for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
        _event: wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur,
        (),
    > for LinuxWaylandBlurAppState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur,
        _event: wayland_protocols_plasma::blur::client::org_kde_kwin_blur::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
#[cfg(target_os = "linux")]
impl LinuxWaylandBlurState {
    fn create(
        desktop: &dioxus::desktop::DesktopContext,
        width: i32,
        height: i32,
    ) -> Result<Self, String> {
        use gtk::glib::object::ObjectType;
        use gtk::glib::translate::FromGlib;
        use gtk::prelude::*;
        use wayland_client::Proxy;

        let gtk_window = desktop.gtk_window();
        let Some(gdk_window) = gtk_window.window() else {
            return Err("gtk window has no GDK window yet".to_string());
        };
        let display = gdk_window.display();
        let display_ptr = display.as_ptr() as *mut gdk_wayland_sys::GdkWaylandDisplay;
        let gdk_window_ptr = gdk_window.as_ptr() as *mut gdk_wayland_sys::GdkWaylandWindow;
        let display_type = display.type_();
        let wayland_display_type =
            unsafe { gtk::glib::Type::from_glib(gdk_wayland_sys::gdk_wayland_display_get_type()) };
        if !display_type.is_a(wayland_display_type) {
            return Err(format!(
                "GDK display is not Wayland: {}",
                display_type.name()
            ));
        }
        let window_type = gdk_window.type_();
        let wayland_window_type =
            unsafe { gtk::glib::Type::from_glib(gdk_wayland_sys::gdk_wayland_window_get_type()) };
        if !window_type.is_a(wayland_window_type) {
            return Err(format!("GDK window is not Wayland: {}", window_type.name()));
        }
        let registry_advertises = |name: &str| -> Result<bool, String> {
            let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
            Ok(unsafe {
                gdk_wayland_sys::gdk_wayland_display_query_registry(display_ptr, name.as_ptr()) != 0
            })
        };
        let ext_background_supported = registry_advertises("ext_background_effect_manager_v1")?;
        let kde_blur_supported = registry_advertises("org_kde_kwin_blur_manager")?;
        if !ext_background_supported && !kde_blur_supported {
            return Err(
                "Wayland compositor advertises neither ext-background-effect-v1 nor org_kde_kwin_blur_manager"
                    .to_string(),
            );
        }
        let wl_display =
            unsafe { gdk_wayland_sys::gdk_wayland_display_get_wl_display(display_ptr) };
        let wl_surface =
            unsafe { gdk_wayland_sys::gdk_wayland_window_get_wl_surface(gdk_window_ptr) };
        if wl_display.is_null() {
            return Err("GDK returned a null wl_display".to_string());
        }
        if wl_surface.is_null() {
            return Err("GDK returned a null wl_surface".to_string());
        }

        let backend =
            unsafe { wayland_client::backend::Backend::from_foreign_display(wl_display.cast()) };
        let conn = wayland_client::Connection::from_backend(backend);
        let surface_id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wayland_client::protocol::wl_surface::WlSurface::interface(),
                wl_surface.cast(),
            )
            .map_err(|error| format!("wl_surface proxy import failed: {error}"))?
        };
        let surface = wayland_client::protocol::wl_surface::WlSurface::from_id(&conn, surface_id)
            .map_err(|error| format!("wl_surface proxy creation failed: {error}"))?;
        let (globals, mut event_queue) =
            wayland_client::globals::registry_queue_init::<LinuxWaylandBlurAppState>(&conn)
                .map_err(|error| format!("Wayland registry init failed: {error}"))?;
        let qh = event_queue.handle();
        let compositor: wayland_client::protocol::wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .map_err(|error| format!("wl_compositor bind failed: {error}"))?;
        let mut app_state = LinuxWaylandBlurAppState {
            capabilities: Arc::new(AtomicU32::new(0)),
        };
        let backend = if ext_background_supported {
            let capabilities = Arc::new(AtomicU32::new(0));
            match globals
                .bind::<wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1, _, _>(
                    &qh,
                    1..=1,
                    capabilities.clone(),
                )
            {
                Ok(manager) => {
                    app_state.capabilities = capabilities;
                    event_queue.roundtrip(&mut app_state).map_err(|error| {
                        format!("ext-background-effect capability roundtrip failed: {error}")
                    })?;
                    let bits = app_state.capabilities.load(Ordering::SeqCst);
                    if bits
                        & wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::Capability::Blur
                            .bits()
                        != 0
                    {
                        LinuxWaylandBlurBackend::ExtBackgroundEffect {
                            effect: manager.get_background_effect(&surface, &qh, ()),
                            _manager: manager,
                        }
                    } else {
                        let ext_error = format!(
                            "ext-background-effect manager is present but blur capability is absent: {bits}"
                        );
                        Self::create_kde_blur_backend(
                            kde_blur_supported,
                            &globals,
                            &qh,
                            &surface,
                            Some(&ext_error),
                        )?
                    }
                }
                Err(error) => {
                    let ext_error = format!("ext-background-effect manager bind failed: {error}");
                    Self::create_kde_blur_backend(
                        kde_blur_supported,
                        &globals,
                        &qh,
                        &surface,
                        Some(&ext_error),
                    )?
                }
            }
        } else {
            Self::create_kde_blur_backend(kde_blur_supported, &globals, &qh, &surface, None)?
        };
        let mut state = Self {
            conn,
            event_queue,
            app_state,
            _globals: globals,
            _compositor: compositor,
            backend,
            last_width: 0,
            last_height: 0,
            apply_count: 0,
        };
        state.update_region(width, height)?;
        Ok(state)
    }

    fn create_kde_blur_backend(
        kde_blur_supported: bool,
        globals: &wayland_client::globals::GlobalList,
        qh: &wayland_client::QueueHandle<LinuxWaylandBlurAppState>,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        ext_error: Option<&str>,
    ) -> Result<LinuxWaylandBlurBackend, String> {
        if !kde_blur_supported {
            let prefix = ext_error
                .map(|error| format!("{error}; "))
                .unwrap_or_default();
            return Err(format!(
                "{prefix}Wayland compositor does not advertise org_kde_kwin_blur_manager"
            ));
        }
        let manager: wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager =
            globals
                .bind(qh, 1..=1, ())
                .map_err(|error| format!("org_kde_kwin_blur_manager bind failed: {error}"))?;
        let blur = manager.create(surface, qh, ());
        Ok(LinuxWaylandBlurBackend::KdeKwinBlur {
            _manager: manager,
            blur,
        })
    }

    fn update_region(&mut self, width: i32, height: i32) -> Result<bool, String> {
        if width <= 0 || height <= 0 {
            return Ok(false);
        }
        let _ = self.event_queue.dispatch_pending(&mut self.app_state);
        if matches!(
            self.backend,
            LinuxWaylandBlurBackend::ExtBackgroundEffect { .. }
        ) && self.app_state.capabilities.load(Ordering::SeqCst)
                & wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::Capability::Blur
                    .bits()
                == 0
        {
            return Err("ext-background-effect blur capability is no longer advertised".to_string());
        }
        if self.last_width == width && self.last_height == height && self.apply_count > 0 {
            return Ok(false);
        }
        let qh = self.event_queue.handle();
        let region = self._compositor.create_region(&qh, ());
        region.add(0, 0, width, height);
        match &self.backend {
            LinuxWaylandBlurBackend::ExtBackgroundEffect { effect, .. } => {
                effect.set_blur_region(Some(&region));
            }
            LinuxWaylandBlurBackend::KdeKwinBlur { blur, .. } => {
                blur.set_region(Some(&region));
                blur.commit();
            }
        }
        region.destroy();
        self.conn
            .flush()
            .map_err(|error| format!("Wayland blur-region flush failed: {error}"))?;
        self.last_width = width;
        self.last_height = height;
        self.apply_count = self.apply_count.saturating_add(1);
        Ok(true)
    }
}
#[cfg(target_os = "linux")]
fn linux_gtk_backend_is_x11_for_platform(
    gdk_backend: Option<&str>,
    wayland_display_present: bool,
    display_present: bool,
) -> bool {
    let backend_names = gdk_backend
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    backend_names.iter().any(|backend| *backend == "x11")
        || (!wayland_display_present && display_present)
}

#[cfg(target_os = "linux")]
fn linux_gtk_backend_is_x11() -> bool {
    let gdk_backend = std::env::var("GDK_BACKEND").ok();
    linux_gtk_backend_is_x11_for_platform(
        gdk_backend.as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct X11CompositorBlurApplyReport {
    xid: u32,
    width: i32,
    height: i32,
    property_present: bool,
    property_value_len: u32,
}

#[cfg(target_os = "linux")]
fn query_kde_x11_blur_region_property(xid: u32) -> std::result::Result<(bool, u32), String> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoConnectionExt};

    if xid == 0 {
        return Ok((false, 0));
    }
    let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
    let atom = connection
        .intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION")
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .atom;
    let reply = connection
        .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 16)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?;
    Ok((reply.format == 32 && reply.value_len >= 4, reply.value_len))
}

#[cfg(target_os = "linux")]
fn set_kde_x11_blur_region(
    xid: u32,
    width: i32,
    height: i32,
) -> std::result::Result<X11CompositorBlurApplyReport, String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as X11rbConnectionExt;

    if xid == 0 || width <= 0 || height <= 0 {
        return Err(format!(
            "invalid X11 blur window geometry: xid={xid} width={width} height={height}"
        ));
    }
    let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
    let atom = connection
        .intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION")
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .atom;
    let region = [0_u32, 0_u32, width as u32, height as u32];
    connection
        .change_property32(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, &region)
        .map_err(|error| error.to_string())?;
    connection.flush().map_err(|error| error.to_string())?;
    let reply = connection
        .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 16)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?;
    let property_present = reply.format == 32 && reply.value_len >= 4;
    Ok(X11CompositorBlurApplyReport {
        xid,
        width,
        height,
        property_present,
        property_value_len: reply.value_len,
    })
}

#[cfg(target_os = "linux")]
fn store_linux_x11_compositor_blur_report(report: &X11CompositorBlurApplyReport) {
    LINUX_X11_COMPOSITOR_BLUR_XID.store(report.xid, Ordering::SeqCst);
    LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(report.property_present, Ordering::SeqCst);
    LINUX_COMPOSITOR_BLUR_ACTIVE.store(report.property_present, Ordering::SeqCst);
}

#[cfg(target_os = "linux")]
fn clear_linux_x11_compositor_blur_state() {
    LINUX_X11_COMPOSITOR_BLUR_XID.store(0, Ordering::SeqCst);
    LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(false, Ordering::SeqCst);
    LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
    LINUX_X11_COMPOSITOR_BLUR_REVERIFY_GENERATION.fetch_add(1, Ordering::SeqCst);
}

#[cfg(target_os = "linux")]
fn refresh_linux_x11_compositor_blur_property_state() {
    if !linux_gtk_backend_is_x11() {
        return;
    }
    let xid = LINUX_X11_COMPOSITOR_BLUR_XID.load(Ordering::SeqCst);
    if xid == 0 {
        LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(false, Ordering::SeqCst);
        LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }
    match query_kde_x11_blur_region_property(xid) {
        Ok((present, _value_len)) => {
            LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(present, Ordering::SeqCst);
            LINUX_COMPOSITOR_BLUR_ACTIVE.store(present, Ordering::SeqCst);
        }
        Err(_error) => {
            LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(false, Ordering::SeqCst);
            LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(target_os = "linux")]
fn schedule_linux_x11_compositor_blur_reverify(
    xid: u32,
    width: i32,
    height: i32,
    trace_home: PathBuf,
    generation: u64,
) {
    let _ = thread::Builder::new()
        .name("yggterm-x11-blur-reverify".to_string())
        .spawn(move || {
            for delay in [Duration::from_millis(350), Duration::from_millis(1_200)] {
                thread::sleep(delay);
                if LINUX_X11_COMPOSITOR_BLUR_REVERIFY_GENERATION.load(Ordering::SeqCst)
                    != generation
                {
                    return;
                }
                match set_kde_x11_blur_region(xid, width, height) {
                    Ok(report) => {
                        store_linux_x11_compositor_blur_report(&report);
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "window",
                            "x11_compositor_blur_region_reverified",
                            json!({
                                "pid": std::process::id(),
                                "xid": report.xid,
                                "width": report.width,
                                "height": report.height,
                                "property_present": report.property_present,
                                "property_value_len": report.property_value_len,
                            }),
                        );
                    }
                    Err(error) => {
                        LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(false, Ordering::SeqCst);
                        LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "window",
                            "x11_compositor_blur_region_reverify_failed",
                            json!({
                                "pid": std::process::id(),
                                "xid": xid,
                                "width": width,
                                "height": height,
                                "error": error,
                            }),
                        );
                    }
                }
            }
        });
}

#[cfg(target_os = "linux")]
fn apply_linux_x11_compositor_blur(
    desktop: &dioxus::desktop::DesktopContext,
    transparent_window: bool,
    trace_home: &Path,
) {
    use gtk::prelude::*;

    if !shell_live_blur_supported()
        || !transparent_window
        || env_flag_truthy("YGGTERM_DISABLE_LIVE_BLUR")
        || env_flag_truthy("YGGTERM_DISABLE_COMPOSITOR_BLUR")
        || !linux_gtk_backend_is_x11()
    {
        clear_linux_x11_compositor_blur_state();
        return;
    }
    let gtk_window = desktop.gtk_window();
    let Some(gdk_window) = gtk_window.window() else {
        return;
    };
    let allocation = gtk_window.allocation();
    let width = allocation.width();
    let height = allocation.height();
    let xid = unsafe { gdk_x11_sys::gdk_x11_window_get_xid(gdk_window.as_ptr() as *mut _) as u32 };
    let generation =
        LINUX_X11_COMPOSITOR_BLUR_REVERIFY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    match set_kde_x11_blur_region(xid, width, height) {
        Ok(report) => {
            store_linux_x11_compositor_blur_report(&report);
            schedule_linux_x11_compositor_blur_reverify(
                report.xid,
                report.width,
                report.height,
                trace_home.to_path_buf(),
                generation,
            );
            append_trace_event(
                trace_home,
                "ui",
                "window",
                "x11_compositor_blur_region_applied",
                json!({
                    "pid": std::process::id(),
                    "xid": report.xid,
                    "width": report.width,
                    "height": report.height,
                    "property_present": report.property_present,
                    "property_value_len": report.property_value_len,
                    "atom": "_KDE_NET_WM_BLUR_BEHIND_REGION",
                }),
            );
            desktop.gtk_window().queue_draw();
            desktop.request_redraw();
        }
        Err(error) => {
            LINUX_X11_COMPOSITOR_BLUR_XID.store(xid, Ordering::SeqCst);
            LINUX_X11_COMPOSITOR_BLUR_PROPERTY_PRESENT.store(false, Ordering::SeqCst);
            LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
            append_trace_event(
                trace_home,
                "ui",
                "window",
                "x11_compositor_blur_region_failed",
                json!({
                    "pid": std::process::id(),
                    "xid": xid,
                    "width": width,
                    "height": height,
                    "error": error,
                }),
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn apply_linux_compositor_blur(
    desktop: &dioxus::desktop::DesktopContext,
    transparent_window: bool,
    trace_home: &Path,
) {
    if linux_gtk_backend_is_x11() {
        apply_linux_x11_compositor_blur(desktop, transparent_window, trace_home);
    } else {
        apply_linux_wayland_compositor_blur(desktop, transparent_window, trace_home);
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_compositor_blur(
    _desktop: &dioxus::desktop::DesktopContext,
    _transparent_window: bool,
    _trace_home: &Path,
) {
}

#[cfg(target_os = "linux")]
fn apply_linux_wayland_compositor_blur(
    desktop: &dioxus::desktop::DesktopContext,
    transparent_window: bool,
    trace_home: &Path,
) {
    use gtk::prelude::*;

    if !shell_live_blur_supported()
        || !transparent_window
        || env_flag_truthy("YGGTERM_DISABLE_LIVE_BLUR")
        || env_flag_truthy("YGGTERM_DISABLE_COMPOSITOR_BLUR")
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
        return;
    }
    let allocation = desktop.gtk_window().allocation();
    let width = allocation.width();
    let height = allocation.height();
    if width <= 0 || height <= 0 {
        return;
    }
    LINUX_WAYLAND_BLUR_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let mut created = false;
        if state.is_none() {
            match LinuxWaylandBlurState::create(desktop, width, height) {
                Ok(next_state) => {
                    let protocol = next_state.backend.protocol_name();
                    *state = Some(next_state);
                    created = true;
                    LINUX_COMPOSITOR_BLUR_ACTIVE.store(true, Ordering::SeqCst);
                    append_trace_event(
                        trace_home,
                        "ui",
                        "window",
                        "wayland_compositor_blur_enabled",
                        json!({
                            "pid": std::process::id(),
                            "width": width,
                            "height": height,
                            "protocol": protocol,
                        }),
                    );
                }
                Err(error) => {
                    LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
                    append_trace_event(
                        trace_home,
                        "ui",
                        "window",
                        "wayland_compositor_blur_unavailable",
                        json!({
                            "pid": std::process::id(),
                            "width": width,
                            "height": height,
                            "error": error,
                        }),
                    );
                    return;
                }
            }
        }
        let mut redraw_needed = false;
        if let Some(blur_state) = state.as_mut() {
            match blur_state.update_region(width, height) {
                Ok(region_applied) => {
                    LINUX_COMPOSITOR_BLUR_ACTIVE.store(true, Ordering::SeqCst);
                    redraw_needed = created || region_applied;
                    if created || blur_state.apply_count <= 2 {
                        append_trace_event(
                            trace_home,
                            "ui",
                            "window",
                            "wayland_compositor_blur_region_applied",
                            json!({
                                "pid": std::process::id(),
                                "width": width,
                                "height": height,
                                "apply_count": blur_state.apply_count,
                            }),
                        );
                    }
                }
                Err(error) => {
                    LINUX_COMPOSITOR_BLUR_ACTIVE.store(false, Ordering::SeqCst);
                    append_trace_event(
                        trace_home,
                        "ui",
                        "window",
                        "wayland_compositor_blur_region_failed",
                        json!({
                            "pid": std::process::id(),
                            "width": width,
                            "height": height,
                            "error": error,
                        }),
                    );
                    *state = None;
                }
            }
        }
        if redraw_needed {
            desktop.gtk_window().queue_draw();
            desktop.request_redraw();
        }
    });
}
#[cfg(not(target_os = "linux"))]
fn apply_linux_wayland_compositor_blur(
    _desktop: &dioxus::desktop::DesktopContext,
    _transparent_window: bool,
    _trace_home: &Path,
) {
}
#[cfg(target_os = "linux")]
fn env_flag_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
#[cfg(target_os = "linux")]
fn linux_session_looks_like_kde_plasma() -> bool {
    [
        std::env::var("XDG_CURRENT_DESKTOP").ok(),
        std::env::var("XDG_SESSION_DESKTOP").ok(),
        std::env::var("DESKTOP_SESSION").ok(),
        std::env::var("KDE_FULL_SESSION").ok(),
    ]
    .into_iter()
    .flatten()
    .any(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        normalized.contains("kde") || normalized.contains("plasma")
    })
}
#[cfg(target_os = "linux")]
fn linux_force_native_decorations() -> bool {
    env_flag_truthy("YGGTERM_FORCE_NATIVE_DECORATIONS")
}
#[cfg(not(target_os = "linux"))]
fn linux_force_native_decorations() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn linux_desktop_app_id() -> String {
    let explicit_suffix = std::env::var("YGGTERM_DESKTOP_APP_ID_SUFFIX").ok();
    let home_seed = std::env::var("YGGTERM_HOME").ok().or_else(|| {
        resolve_yggterm_home()
            .ok()
            .map(|path| path.to_string_lossy().to_string())
    });
    linux_desktop_app_id_for_context(
        explicit_suffix.as_deref(),
        env_flag_truthy("YGGTERM_REMOTE_SMOKE_TAG"),
        false,
        home_seed.as_deref(),
    )
}
#[cfg(target_os = "linux")]
fn linux_desktop_app_id_for_context(
    explicit_suffix: Option<&str>,
    isolated_multi_window: bool,
    _stale_client_isolation: bool,
    home_seed: Option<&str>,
) -> String {
    if let Some(suffix) = explicit_suffix
        .map(linux_app_id_component)
        .filter(|value| !value.is_empty())
    {
        return format!("{YGGTERM_DESKTOP_APP_ID}.{suffix}");
    }
    if isolated_multi_window {
        let seed = home_seed
            .map(str::to_string)
            .unwrap_or_else(|| format!("pid-{}", std::process::id()));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut hasher);
        return format!(
            "{YGGTERM_DESKTOP_APP_ID}.{}",
            linux_app_id_component(&format!("multi{:x}", hasher.finish()))
        );
    }
    YGGTERM_DESKTOP_APP_ID.to_string()
}
#[cfg(target_os = "linux")]
fn linux_desktop_identity_snapshot(gdk_program_class: Option<&str>) -> Value {
    json!({
        "glib_prgname": gtk::glib::prgname().map(|value| value.to_string()),
        "glib_application_name": gtk::glib::application_name().map(|value| value.to_string()),
        "gdk_program_class": gdk_program_class,
    })
}
#[cfg(target_os = "linux")]
fn set_linux_gdk_program_class_raw(app_id: &str) {
    use gtk::glib::translate::ToGlibPtr as _;

    let app_id = app_id.to_glib_none();
    unsafe {
        gdk::ffi::gdk_set_program_class(app_id.0);
    }
}
#[cfg(target_os = "linux")]
fn apply_linux_desktop_identity(app_id: &str, trace_home: &Path) {
    let before = linux_desktop_identity_snapshot(None);
    gtk::glib::set_prgname(Some(app_id));
    gtk::glib::set_application_name("Yggterm");
    set_linux_gdk_program_class_raw(app_id);
    let after = linux_desktop_identity_snapshot(Some(app_id));
    append_trace_event(
        trace_home,
        "ui",
        "startup",
        "linux_desktop_identity_applied",
        json!({
            "pid": std::process::id(),
            "app_id": app_id,
            "before": before,
            "after": after,
        }),
    );
}
fn linux_client_record_requires_app_id_isolation(
    record_pid: u32,
    record_executable_path: Option<&str>,
    current_pid: u32,
    current_executable_path: Option<&str>,
) -> bool {
    if record_pid == current_pid {
        return false;
    }
    let Some(record_executable_path) = record_executable_path.and_then(non_empty_trimmed_string)
    else {
        return false;
    };
    let Some(current_executable_path) = current_executable_path.and_then(non_empty_trimmed_string)
    else {
        return false;
    };
    record_executable_path != current_executable_path
}
fn non_empty_trimmed_string(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() { None } else { Some(value) }
}
#[cfg(target_os = "linux")]
fn linux_app_id_component(value: &str) -> String {
    let mut output = String::from("i");
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            output.push(ch.to_ascii_lowercase());
        }
    }
    output.truncate(63);
    output
}
#[cfg(target_os = "linux")]
fn linux_close_requires_terminal_detach() -> bool {
    linux_session_looks_like_kde_plasma()
}
#[cfg(not(target_os = "linux"))]
fn linux_close_requires_terminal_detach() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn linux_limit_live_terminal_retention() -> bool {
    limit_live_terminal_retention_for_platform(
        linux_session_looks_like_kde_plasma(),
        std::env::var_os("DISPLAY").is_some(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        env_flag_truthy("YGGTERM_LIMIT_LIVE_TERMINAL_RETENTION"),
    )
}
#[cfg(not(target_os = "linux"))]
fn linux_limit_live_terminal_retention() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn linux_native_window_shape_supported() -> bool {
    let gdk_backend = std::env::var("GDK_BACKEND").ok();
    linux_native_window_shape_supported_for_platform(
        env_flag_truthy("YGGTERM_DISABLE_NATIVE_WINDOW_SHAPE"),
        linux_force_native_decorations(),
        gdk_backend.as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}
#[cfg(not(target_os = "linux"))]
fn linux_native_window_shape_supported() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn linux_native_window_shape_supported_for_platform(
    disabled_by_env: bool,
    native_decorations_forced: bool,
    gdk_backend: Option<&str>,
    wayland_display_present: bool,
) -> bool {
    if disabled_by_env || native_decorations_forced {
        return false;
    }
    match gdk_backend {
        Some("x11") => true,
        Some("wayland") => false,
        _ => !wayland_display_present,
    }
}
#[cfg(target_os = "linux")]
fn apply_linux_widget_corner_region(
    desktop: &dioxus::desktop::DesktopContext,
    region: Option<&cairo::Region>,
) {
    use gtk::prelude::*;

    let gtk_window = desktop.gtk_window();
    if gtk_window.has_window() {
        gtk_window.shape_combine_region(region);
        gtk_window.input_shape_combine_region(region);
    }
    if let Some(vbox) = desktop.default_vbox() {
        if vbox.has_window() {
            vbox.shape_combine_region(region);
            vbox.input_shape_combine_region(region);
        }
    }
}
#[cfg(target_os = "linux")]
fn apply_linux_window_corner_shape(
    desktop: &dioxus::desktop::DesktopContext,
    radius: u8,
    maximized: bool,
) {
    use gtk::prelude::*;

    let gtk_window = desktop.gtk_window();
    let allocation = gtk_window.allocation();
    let width = allocation.width();
    let height = allocation.height();
    if width <= 0 || height <= 0 {
        return;
    }
    let Some(gdk_window) = gtk_window.window() else {
        return;
    };
    if maximized || radius == 0 || !linux_native_window_shape_supported() {
        let full_region =
            cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, width, height));
        apply_linux_widget_corner_region(desktop, None);
        gdk_window.shape_combine_region(Option::<&cairo::Region>::None, 0, 0);
        gdk_window.input_shape_combine_region(&full_region, 0, 0);
        return;
    }
    let region = rounded_window_region(width, height, i32::from(radius));
    apply_linux_widget_corner_region(desktop, Some(&region));
    gdk_window.shape_combine_region(Some(&region), 0, 0);
    gdk_window.input_shape_combine_region(&region, 0, 0);
}
#[cfg(not(target_os = "linux"))]
fn apply_linux_window_corner_shape(
    _desktop: &dioxus::desktop::DesktopContext,
    _radius: u8,
    _maximized: bool,
) {
}
#[cfg(target_os = "linux")]
fn apply_linux_window_shape_reapply_sequence(
    desktop: &dioxus::desktop::DesktopContext,
    radius: u8,
    maximized: bool,
) {
    use gtk::prelude::*;

    let gtk_window = desktop.gtk_window();
    for delay_ms in [
        0_u64, 1, 8, 16, 24, 72, 180, 400, 900, 1_600, 2_800, 4_500, 7_000, 10_000, 14_000,
    ] {
        let gtk_window = gtk_window.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
            let allocation = gtk_window.allocation();
            let width = allocation.width();
            let height = allocation.height();
            if width <= 0 || height <= 0 {
                return;
            }
            let Some(gdk_window) = gtk_window.window() else {
                return;
            };
            if maximized || radius == 0 || !linux_native_window_shape_supported() {
                let full_region =
                    cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, width, height));
                if gtk_window.has_window() {
                    gtk_window.shape_combine_region(Option::<&cairo::Region>::None);
                    gtk_window.input_shape_combine_region(Option::<&cairo::Region>::None);
                }
                gdk_window.shape_combine_region(Option::<&cairo::Region>::None, 0, 0);
                gdk_window.input_shape_combine_region(&full_region, 0, 0);
            } else {
                let region = rounded_window_region(width, height, i32::from(radius));
                if gtk_window.has_window() {
                    gtk_window.shape_combine_region(Some(&region));
                    gtk_window.input_shape_combine_region(Some(&region));
                }
                gdk_window.shape_combine_region(Some(&region), 0, 0);
                gdk_window.input_shape_combine_region(&region, 0, 0);
            }
            gtk_window.queue_draw();
        });
    }
}
#[cfg(not(target_os = "linux"))]
fn apply_linux_window_shape_reapply_sequence(
    _desktop: &dioxus::desktop::DesktopContext,
    _radius: u8,
    _maximized: bool,
) {
}
#[cfg(target_os = "linux")]
fn apply_linux_always_on_top_state(desktop: &dioxus::desktop::DesktopContext, always_on_top: bool) {
    use gtk::prelude::*;

    desktop.set_always_on_bottom(false);
    let gtk_window = desktop.gtk_window();
    gtk_window.set_keep_below(false);
    gtk_window.set_keep_above(always_on_top);
    if let Some(gdk_window) = gtk_window.window() {
        gdk_window.set_keep_below(false);
        gdk_window.set_keep_above(always_on_top);
    }
}
#[cfg(not(target_os = "linux"))]
fn apply_linux_always_on_top_state(
    _desktop: &dioxus::desktop::DesktopContext,
    _always_on_top: bool,
) {
}
fn sync_window_frame_state(shell: &mut ShellState) {
    if shell.closing_app {
        return;
    }
    shell.remember_window_maximized(window().is_maximized());
}
fn icon_button_style(palette: Palette) -> String {
    format!(
        "width:26px; height:26px; border:none; border-radius:8px; background:{}; color:{}; font-size:13px; \
         box-shadow: inset 0 0 0 1px {}; user-select:none; -webkit-user-select:none; pointer-events:auto; transition:{};",
        chrome_chip_fill(palette, false),
        chrome_chip_text_color(palette, false, false),
        chrome_chip_border(palette),
        standard_transition(&["background-color", "color", "box-shadow"])
    )
}
fn utility_icon_style(palette: Palette, selected: bool) -> String {
    utility_icon_style_sized(palette, selected, 13)
}
fn utility_icon_style_sized(palette: Palette, selected: bool, font_size_px: u8) -> String {
    format!(
        "width:26px; height:26px; border:none; border-radius:9px; background:{}; color:{}; font-size:{}px; font-weight:{}; \
         box-shadow: inset 0 0 0 1px {}; user-select:none; -webkit-user-select:none; pointer-events:auto; transition:{};",
        chrome_chip_fill(palette, selected),
        if selected {
            palette.accent
        } else {
            chrome_chip_text_color(palette, selected, false)
        },
        font_size_px,
        if selected { 800 } else { 700 },
        chrome_chip_border(palette),
        standard_transition(&["background-color", "color", "box-shadow"])
    )
}
fn connect_button_style(palette: Palette, selected: bool) -> String {
    let fill = if selected {
        palette.accent_soft.to_string()
    } else if palette_is_dark(palette) {
        "rgba(46, 128, 224, 0.22)".to_string()
    } else {
        "rgba(9, 105, 218, 0.12)".to_string()
    };
    let text = if selected {
        palette.accent.to_string()
    } else if palette_is_dark(palette) {
        "#7fc0ff".to_string()
    } else {
        "#0969da".to_string()
    };
    let border = if selected {
        palette.accent.to_string()
    } else if palette_is_dark(palette) {
        "rgba(95, 168, 255, 0.40)".to_string()
    } else {
        "rgba(9, 105, 218, 0.32)".to_string()
    };
    format!(
        "height:26px; padding:0 11px; border:none; border-radius:10px; background:{}; color:{}; \
         font-size:11px; font-weight:700; white-space:nowrap; user-select:none; -webkit-user-select:none; pointer-events:auto; \
         box-shadow: inset 0 0 0 1px {}; transition:{};",
        fill,
        text,
        border,
        standard_transition(&["background-color", "color", "box-shadow"])
    )
}
fn chip_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:24px; padding:0 10px; border-radius:999px; border:1px solid {}; background:{}; \
         color:{}; font-size:11px; font-weight:600; transition:{};",
        if selected {
            palette.accent
        } else if palette_is_dark(palette) {
            "rgba(138,170,197,0.18)"
        } else {
            "rgba(255,255,255,0.10)"
        },
        if selected {
            palette.accent_soft
        } else if palette_is_dark(palette) {
            "rgba(12,18,24,0.84)"
        } else {
            "rgba(255,255,255,0.28)"
        },
        if selected {
            palette.text
        } else {
            palette.muted
        },
        standard_transition(&["border-color", "background-color", "color"])
    )
}
fn titlebar_new_action_style(palette: Palette) -> String {
    shared_menu_item_style(palette, MenuItemTone::Standard, 29, 11.0, "0 10px", 0, true)
}
fn titlebar_modal_action_style(_palette: Palette) -> String {
    format!(
        "display:inline-flex; align-items:center; justify-content:center; flex:0 1 134px; min-width:134px; max-width:100%; height:31px; padding:0 13px; border:none; border-radius:10px; \
         background:{}; color:{}; font-size:11.5px; font-weight:700; text-align:center; cursor:pointer; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; box-shadow:{}; box-sizing:border-box;",
        "rgb(255,255,255)",
        "#18222d",
        "inset 0 0 0 1px rgba(214,223,232,0.92), 0 1px 2px rgba(15,23,42,0.08)",
    )
}
fn rename_ai_action_button_style(palette: Palette) -> String {
    format!(
        "display:inline-flex; align-items:center; justify-content:center; width:26px; height:26px; \
         border:none; border-radius:9px; background:rgba(255,255,255,0.96); color:{}; \
         box-shadow:inset 0 0 0 1px rgba(204,214,224,0.94), 0 1px 2px rgba(15,23,42,0.08); \
         padding:0; flex:0 0 auto; cursor:pointer;",
        palette.accent
    )
}
fn titlebar_modal_icon_button_style(palette: Palette) -> String {
    format!(
        "display:inline-flex; align-items:center; justify-content:center; width:24px; height:24px; \
         border:none; border-radius:9px; background:rgba(255,255,255,0.96); color:{}; \
         box-shadow:inset 0 0 0 1px rgba(214,223,232,0.92), 0 1px 2px rgba(15,23,42,0.08); \
         padding:0; flex:0 0 auto; cursor:pointer;",
        palette.accent
    )
}
fn theme_editor_action_button_style(
    palette: Palette,
    accent: &str,
    enabled: bool,
    primary: bool,
) -> String {
    format!(
        "display:flex; align-items:center; gap:7px; height:30px; padding:0 11px; border:none; border-radius:11px; \
         background:{}; color:{}; font-size:11px; font-weight:700; box-shadow:{}; opacity:{};",
        if primary {
            if palette_is_dark(palette) {
                "rgba(24,31,38,0.96)"
            } else {
                "rgba(255,255,255,0.98)"
            }
        } else if enabled {
            if palette_is_dark(palette) {
                "rgba(21,28,35,0.92)"
            } else {
                "rgba(235,242,248,0.96)"
            }
        } else {
            if palette_is_dark(palette) {
                "rgba(21,28,35,0.72)"
            } else {
                "rgba(244,247,250,0.84)"
            }
        },
        if primary {
            accent
        } else if enabled {
            palette.text
        } else {
            palette.muted
        },
        if primary {
            if palette_is_dark(palette) {
                "inset 0 0 0 1px rgba(110,138,162,0.38), 0 10px 24px rgba(0,0,0,0.22)".to_string()
            } else {
                "inset 0 0 0 1px rgba(214,223,232,0.92), 0 8px 18px rgba(81,113,138,0.08)"
                    .to_string()
            }
        } else if enabled {
            if palette_is_dark(palette) {
                "inset 0 0 0 1px rgba(93,116,134,0.56)".to_string()
            } else {
                "inset 0 0 0 1px rgba(214,223,232,0.92)".to_string()
            }
        } else {
            if palette_is_dark(palette) {
                "inset 0 0 0 1px rgba(74,91,106,0.42)".to_string()
            } else {
                "inset 0 0 0 1px rgba(224,231,238,0.9)".to_string()
            }
        },
        if enabled || primary { "1" } else { "0.72" },
    )
}
fn primary_action_style(palette: Palette) -> String {
    format!(
        "width:100%; border:none; border-radius:12px; background:{}; color:white; padding:10px 12px; text-align:left; \
         box-shadow: 0 12px 28px rgba(47,124,246,0.24), inset 0 0 0 1px rgba(255,255,255,0.18); \
         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
        palette.accent
    )
}
/// Standard segmented control (pill toggle) — the "snug" look where the track is
/// only a hair larger than the active segment. The titlebar Web View/Terminal
/// toggle is the reference. Used by every multi-segment MODE switch: titlebar
/// view mode, the agent-mode selector, Settings Light/Dark, and Notifications
/// App/Both/System. NOT for binary on/off switches — those use the
/// `inline_toggle_*` track+thumb (e.g. Auto-hide Titlebar, Sound).
/// See DESIGN.md "Segmented controls".
/// A row of titlebar controls that REFLECTS with the chrome mirror.
///
/// The rule ([[DESIGN.md]] §Mirrored chrome): a mirror reflects the
/// ARRANGEMENT of controls, not the inside of a control. So the cluster rows
/// either side of the search box reverse — the `☰` that opens the tree stays
/// against the tree's own edge, and the button nearest the search box stays
/// nearest the search box — while a single control's own parts (the
/// segmented Web View/Terminal toggle, the window-button strip) keep their
/// order, because they are one thing rather than an arrangement.
///
/// `flex-direction` is emitted in BOTH branches: Dioxus applies `style`
/// property-by-property and never clears a dropped key, so a one-sided
/// `row-reverse` would survive un-mirroring forever.
fn titlebar_cluster_row_style(orientation: ChromeOrientation, gap_px: u32, extra: &str) -> String {
    format!(
        "display:flex; flex-direction:{}; align-items:center; gap:{}px; min-width:0; max-width:100%;{}",
        if orientation.is_mirrored() {
            "row-reverse"
        } else {
            "row"
        },
        gap_px,
        extra,
    )
}
/// Where a titlebar popover hangs from its trigger.
///
/// A menu grows AWAY from the window edge its cluster is anchored to, so it
/// stays on screen in both orientations: the `+` menu drops down-and-right from
/// a left-hand cluster and down-and-LEFT from a mirrored one. `offset_px` is the
/// distance from that edge (0 for a menu pinned to its trigger, a fixed inset
/// for the overflow menu, which hangs from the titlebar rather than a box).
///
/// Both `left` and `right` are emitted every time — the style-key trap again: a
/// popover that anchored by omission would keep the previous orientation's
/// anchor and be pinned to both edges at once.
fn titlebar_menu_anchor_style(edge: SidebarEdge, offset_px: f64) -> String {
    format!(
        "{}:{}px; {}:auto;",
        edge.css_near(),
        offset_px,
        edge.css_far()
    )
}
/// The corner radius of a panel ATTACHED under a tab at one end of its top
/// edge: square where the tab meets it, rounded everywhere else. The square
/// corner follows the cluster's edge, so the seam stays under the tab after a
/// mirror instead of appearing on the far side of the menu.
fn titlebar_attached_menu_radius(edge: SidebarEdge) -> &'static str {
    match edge {
        SidebarEdge::Left => "0 16px 16px 16px",
        SidebarEdge::Right => "16px 0 16px 16px",
    }
}
fn segmented_control_track_style(palette: Palette) -> String {
    format!(
        "display:flex; align-items:center; gap:4px; padding:3px; border:none; border-radius:999px; background:{}; box-shadow: inset 0 0 0 1px {}; user-select:none; -webkit-user-select:none; transition:{};",
        if palette_is_dark(palette) {
            "rgba(10,14,18,0.74)"
        } else {
            "rgba(255,255,255,0.46)"
        },
        if palette_is_dark(palette) {
            "rgba(187,204,219,0.16)"
        } else {
            "rgba(201,214,226,0.42)"
        },
        standard_transition(&["background-color", "box-shadow"])
    )
}
/// One segment of a [`segmented_control_track_style`] control. `grow` makes the
/// segment flex to fill the track evenly (settings panels); `false` keeps a fixed
/// min-width (compact chrome toggles). `on_chrome` picks luminance-aware text for
/// the variable titlebar chrome; `false` uses plain palette text on a card. The
/// snug look comes from a near-edge-to-edge active fill with NO drop shadow — the
/// track's 3px padding is the only gap (the old settings pill added a
/// `0 3px 10px` lift that read as a much larger bg pill).
fn segmented_control_segment_style(
    palette: Palette,
    selected: bool,
    grow: bool,
    on_chrome: bool,
) -> String {
    let sizing = if grow { "flex:1; min-width:0;" } else { "min-width:82px;" };
    let background = if selected {
        if palette_is_dark(palette) {
            "rgba(255,255,255,0.10)"
        } else {
            "rgba(255,255,255,0.92)"
        }
    } else if palette_is_dark(palette) {
        "rgba(255,255,255,0.03)"
    } else {
        "transparent"
    };
    let color: String = if on_chrome {
        chrome_chip_text_color(palette, selected, selected).to_string()
    } else if selected {
        palette.text.to_string()
    } else {
        palette.muted.to_string()
    };
    let font_weight = if on_chrome || selected { 700 } else { 600 };
    format!(
        "{sizing} height:26px; padding:0 12px; border:none; border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:{}; \
         cursor:pointer; user-select:none; -webkit-user-select:none; transition:{};",
        background,
        color,
        font_weight,
        standard_transition(&["background-color", "color"])
    )
}
fn chrome_chip_fill(palette: Palette, selected: bool) -> &'static str {
    if selected {
        palette.accent_soft
    } else if palette_is_dark(palette) {
        "rgba(8,12,16,0.82)"
    } else {
        "rgba(255,255,255,0.72)"
    }
}
fn chrome_chip_border(palette: Palette) -> &'static str {
    if palette_is_dark(palette) {
        "rgba(214,229,242,0.24)"
    } else {
        "rgba(201,214,226,0.56)"
    }
}
fn chrome_chip_text_color(palette: Palette, selected: bool, emphasized: bool) -> &'static str {
    let fill = chrome_chip_fill(palette, selected);
    let base = if palette.titlebar == "transparent" {
        palette.shell
    } else {
        palette.titlebar
    };
    match chrome_blended_luminance(fill, base) {
        Some(luminance) if luminance < 0.46 => {
            if emphasized {
                "#f6fbff"
            } else {
                "#e6f0f9"
            }
        }
        _ => {
            if emphasized {
                "#18222d"
            } else {
                "#33414d"
            }
        }
    }
}
fn settings_section_card_style(palette: Palette) -> String {
    format!(
        "display:flex; flex-direction:column; gap:8px; padding:9px; border-radius:14px; \
         background:{}; box-shadow: inset 0 0 0 1px {}; transition:{};",
        if palette_is_dark(palette) {
            "rgba(255,255,255,0.04)"
        } else {
            "rgba(255,255,255,0.22)"
        },
        if palette_is_dark(palette) {
            "rgba(141,160,178,0.16)"
        } else {
            "rgba(198,212,224,0.32)"
        },
        standard_transition(&["background-color", "box-shadow"])
    )
}
/// A Settings box.
///
/// ⛔ NO `background`, NO `box-shadow`: the SKIN is `text_field_css`'s (it wears
/// `data-yggui-field="true"`), because hover and focus are CSS states an inline
/// style cannot express. This function is the BOX — height, padding, radius,
/// type — and nothing else.
/// A settings / vault field.
///
/// ⚠ It sets NO fill, not even `transparent`. The element wears
/// `data-yggui-field`, so the resting fill, hairline, hover and focus ring all
/// come from the one field stylesheet the search box uses — and ANY inline
/// background, transparent included, out-specifies that stylesheet and kills
/// hover and focus outright. `every_text_field_wears_one_skin_…` locks it.
fn settings_input_style(palette: Palette) -> String {
    format!(
        "height:32px; padding:0 11px; border:none; border-radius:9px; \
         color:{}; outline:none; font-size:11.5px;",
        palette.text,
    )
}
fn zoom_button_style(palette: Palette) -> String {
    format!(
        "width:22px; height:22px; border:none; border-radius:8px; background:{}; \
         color:{}; font-size:13px; font-weight:700; display:inline-flex; align-items:center; justify-content:center; transition:{};",
        if palette_is_dark(palette) {
            "rgba(21,28,35,0.92)"
        } else {
            "rgba(255,255,255,0.36)"
        },
        palette.text,
        standard_transition(&["background-color", "color"])
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItemTone {
    Standard,
    Emphasized,
    Destructive,
    /// Drawn but inert: dimmed, and no hover tint — hover feedback on something
    /// that cannot be clicked is a lie about the affordance.
    Inert,
}

/// THE product's one destructive red — a menu's "Delete", a pane footer's trash.
///
/// Named because it now has a second consumer: an app-pane button that declares
/// `danger`. Two surfaces spelling the same intent with two hex values is how a
/// product ends up with a delete that is a different red in every menu.
const DESTRUCTIVE_RED: &str = "#c23f4d";
fn shared_menu_item_style(
    palette: Palette,
    tone: MenuItemTone,
    min_height_px: u32,
    font_size_px: f64,
    padding: &str,
    margin_bottom_px: u32,
    bold_standard: bool,
) -> String {
    let dark = palette_is_dark(palette);
    let hover_background = match tone {
        MenuItemTone::Destructive => "rgba(194,63,77,0.12)",
        MenuItemTone::Emphasized => {
            if dark {
                "rgba(124,200,255,0.20)"
            } else {
                "rgba(36,117,191,0.12)"
            }
        }
        MenuItemTone::Inert => "transparent",
        MenuItemTone::Standard => {
            if dark {
                "rgba(124,200,255,0.14)"
            } else {
                "rgba(36,117,191,0.10)"
            }
        }
    };
    let base_color = match tone {
        MenuItemTone::Destructive => DESTRUCTIVE_RED,
        MenuItemTone::Emphasized => palette.accent,
        MenuItemTone::Inert => {
            if dark {
                "rgba(244,249,255,0.42)"
            } else {
                "rgba(24,34,45,0.42)"
            }
        }
        MenuItemTone::Standard => {
            if dark {
                "rgba(244,249,255,0.96)"
            } else {
                "#18222d"
            }
        }
    };
    let font_weight = match tone {
        MenuItemTone::Standard if !bold_standard => 600,
        MenuItemTone::Inert => 600,
        _ => 700,
    };
    format!(
        "--yggterm-menu-item-background:transparent; --yggterm-menu-item-hover-background:{}; \
         --yggterm-menu-item-color:{}; --yggterm-menu-item-hover-color:{}; \
         --yggterm-menu-item-focus-ring:{}; --yggterm-menu-item-font-weight:{}; \
         --yggterm-menu-item-min-height:{}px; --yggterm-menu-item-font-size:{:.1}px; \
         --yggterm-menu-item-padding:{}; --yggterm-menu-item-margin-bottom:{}px;",
        hover_background,
        base_color,
        base_color,
        if dark {
            "rgba(124,200,255,0.34)"
        } else {
            "rgba(36,117,191,0.22)"
        },
        font_weight,
        min_height_px,
        font_size_px,
        padding,
        margin_bottom_px,
    )
}
fn context_menu_action_style(palette: Palette, emphasized: bool) -> String {
    shared_menu_item_style(
        palette,
        if emphasized {
            MenuItemTone::Emphasized
        } else {
            MenuItemTone::Standard
        },
        32,
        12.0,
        "0 12px",
        4,
        false,
    )
}
fn context_menu_action_style_destructive(palette: Palette) -> String {
    shared_menu_item_style(
        palette,
        MenuItemTone::Destructive,
        32,
        12.0,
        "0 12px",
        4,
        false,
    )
}
/// The drawn style for ONE [`ContextMenuOverlay`] entry, dimming included.
///
/// THE one style owner for menu entries: every branch routes through the
/// shared style engine below, so all branches emit the IDENTICAL key set
/// (Dioxus applies `style` PROPERTY BY PROPERTY and never clears a key a
/// later render drops — the sidebar-overlay trap). A named function rather
/// than an inline `format!` because "shown and dimmed" is an assertable fact.
fn context_menu_item_style(palette: Palette, item: &RowMenuItem) -> String {
    if item.disabled {
        context_menu_action_style_disabled(palette)
    } else if item.destructive {
        context_menu_action_style_destructive(palette)
    } else {
        context_menu_action_style(palette, item.emphasized)
    }
}
/// Whether a click on a drawn menu entry may reach the overlay's `on_action`.
///
/// A thin adapter over [`context_menu_click_action`] — THE dispatch owner —
/// kept so "may this item dispatch?" stays askable as a bool. Separators are
/// part of the same sentence: a divider is never dispatched.
fn context_menu_item_dispatches(item: &RowMenuItem) -> bool {
    context_menu_click_action(item).is_some()
}
/// A menu item that is drawn but inert. Same style ENGINE and therefore exactly
/// the same custom-property keys as the other two — only the values differ (no
/// hover tint, dimmed text). Emitting a different key set here would leave the
/// dropped keys painted from the previous render forever.
fn context_menu_action_style_disabled(palette: Palette) -> String {
    shared_menu_item_style(palette, MenuItemTone::Inert, 32, 12.0, "0 12px", 4, false)
}
fn cancel_confirm_button_style(_palette: Palette) -> String {
    "height:34px; padding:0 16px; border:none; border-radius:12px; background:#5fa8ff; color:#ffffff; \
     font-size:13px; font-weight:500;"
        .to_string()
}
fn delete_confirm_button_style(_palette: Palette, hard_delete: bool) -> String {
    format!(
        "height:34px; padding:0 16px; border:none; border-radius:12px; background:{}; color:#ffffff; \
         font-size:13px; font-weight:500;",
        if hard_delete { "#b3263f" } else { "#c23f4d" }
    )
}
fn join_debug_paths(mut paths: Vec<String>) -> String {
    if paths.is_empty() {
        return "none".to_string();
    }
    paths.sort();
    if paths.len() > 4 {
        let extra = paths.len() - 4;
        paths.truncate(4);
        format!("{}, +{} more", paths.join(", "), extra)
    } else {
        paths.join(", ")
    }
}
fn inline_toggle_row_button_style(palette: Palette, enabled: bool) -> String {
    format!(
        "display:flex; align-items:center; justify-content:space-between; gap:12px; width:100%; min-height:44px; padding:10px 12px; \
         border:none; border-radius:12px; background:{}; color:{}; text-align:left; opacity:{}; transition:{};",
        if palette_is_dark(palette) {
            "rgba(255,255,255,0.02)"
        } else {
            "rgba(255,255,255,0.18)"
        },
        palette.text,
        if enabled { "1" } else { "0.94" },
        standard_transition(&["background-color", "opacity"])
    )
}
/// The row an app-contributed `toggle` widget draws in. Narrower than
/// `inline_toggle_row_button_style` (a 300px rail, no description line) but the
/// same switch, so a contributed pane and yggterm's own settings speak one
/// visual language.
fn app_pane_toggle_row_style(palette: Palette, enabled: bool) -> String {
    format!(
        "display:flex; align-items:center; justify-content:space-between; gap:10px; width:100%; \
         padding:8px 10px; border:none; border-radius:10px; background:{}; color:{}; \
         font-size:11px; font-weight:600; text-align:left; cursor:pointer; opacity:{}; \
         transition:{};",
        if palette_is_dark(palette) {
            "rgba(255,255,255,0.03)"
        } else {
            "rgba(255,255,255,0.22)"
        },
        palette.text,
        if enabled { "1" } else { "0.94" },
        standard_transition(&["background-color", "opacity"])
    )
}
fn inline_toggle_affordance_style(enabled: bool) -> String {
    format!(
        "display:flex; align-items:center; gap:10px; justify-content:flex-end; flex:0 0 auto; pointer-events:none; opacity:{}; transition:{};",
        if enabled { "1" } else { "0.92" },
        standard_transition(&["opacity"])
    )
}
fn inline_toggle_track_style(palette: Palette, enabled: bool) -> String {
    let transition = if enabled {
        emphasized_enter_transition(&["background-color", "box-shadow"])
    } else {
        emphasized_exit_transition(&["background-color", "box-shadow"])
    };
    // flex:0 0 auto — the track sits in a flex row beside the label, and a
    // long label (SponsorBlock, the cookie extension) otherwise SHRINKS it:
    // a 36px track squeezed to ~20px with the thumb translated 16px paints
    // the knob hanging outside the pill (the "UI corruption" report).
    format!(
        "position:relative; flex:0 0 auto; width:36px; height:20px; border-radius:999px; background:{}; box-shadow:inset 0 0 0 1px {}; transition:{};",
        if enabled {
            palette.accent
        } else if palette_is_dark(palette) {
            "rgba(103,121,137,0.68)"
        } else {
            "rgba(189,201,212,0.92)"
        },
        if enabled {
            "rgba(255,255,255,0.16)"
        } else if palette_is_dark(palette) {
            "rgba(214,229,242,0.12)"
        } else {
            "rgba(161,176,190,0.12)"
        },
        transition
    )
}
fn inline_toggle_thumb_style(enabled: bool) -> String {
    let transition = if enabled {
        emphasized_enter_transition(&["transform", "box-shadow"])
    } else {
        emphasized_exit_transition(&["transform", "box-shadow"])
    };
    format!(
        "position:absolute; top:3px; left:3px; width:14px; height:14px; border-radius:999px; background:white; \
         box-shadow:{}; transform:translateX({}px); transition:{};",
        if enabled {
            "0 6px 14px rgba(15,23,42,0.18)"
        } else {
            "0 2px 8px rgba(36,48,58,0.18)"
        },
        if enabled { 16 } else { 0 },
        transition
    )
}
fn sanitize_zoom_percent_text(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}
fn normalize_zoom_percent_text(value: &str, fallback: i32) -> i32 {
    sanitize_zoom_percent_text(value)
        .parse::<i32>()
        .unwrap_or(fallback)
        .clamp(50, 250)
}
fn clamp_zoom_value_for_base(value: f32, base: f32) -> f32 {
    value.clamp(base * 0.5, base * 2.5)
}
fn clamp_zoom_value(value: f32) -> f32 {
    clamp_zoom_value_for_base(value, 14.0)
}
fn clamp_zoom_value_main(value: f32, target: MainZoomTarget) -> f32 {
    clamp_zoom_value_for_base(value, main_zoom_base(target))
}
fn zoom_percent(value: f32, base: f32) -> i32 {
    ((value / base) * 100.0).round() as i32
}
fn zoom_percent_f32(value: f32, base: f32) -> f32 {
    (value / base) * 100.0
}
fn metadata_value(session: &ManagedSessionView, label: &str) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}
#[cfg(target_os = "linux")]
fn interface_font_family() -> &'static str {
    "\"Inter Variable\", \"Inter\", system-ui, sans-serif"
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn interface_font_family() -> &'static str {
    "system-ui, sans-serif"
}
