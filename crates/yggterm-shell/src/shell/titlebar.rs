#[component]
fn Titlebar(
    snapshot: SharedSnapshot,
    hovered: Signal<Option<HoveredControl>>,
    on_toggle_sidebar: EventHandler<()>,
    on_search: EventHandler<String>,
    on_clear_search: EventHandler<()>,
    on_execute_search_command: EventHandler<String>,
    on_set_search_focus: EventHandler<bool>,
    on_prev_search_content: EventHandler<()>,
    on_next_search_content: EventHandler<()>,
    on_hover_control: EventHandler<Option<HoveredControl>>,
    on_set_view_mode: EventHandler<WorkspaceViewMode>,
    /// Show (`true`) or hide (`false`) a session's document surface. The other
    /// half of the surface-switch slot — see [`TitlebarSurfaceSwitch`]. The
    /// SESSION rides along rather than being re-derived at the far end: the
    /// slot was drawn for one particular path, and a second derivation is a
    /// second chance to pick a different one.
    on_set_document_surface_visible: EventHandler<(String, bool)>,
    on_document_action: EventHandler<(String, String, String, Option<String>)>, 
    on_toggle_session_menu: EventHandler<()>,
    on_toggle_new_menu: EventHandler<()>,
    on_toggle_overflow_menu: EventHandler<()>,
    on_close_overflow_menu: EventHandler<()>,
    on_start_session: EventHandler<()>,
    on_start_claude_code: EventHandler<()>,
    on_start_terminal: EventHandler<()>,
    /// Launch a verb an installed libyggterm app contributed. yggterm knows
    /// nothing about the app beyond its manifest.
    on_launch_app_verb: EventHandler<(AppManifest, AppVerb)>,
    on_refresh_summary: EventHandler<()>,
    on_begin_active_rename: EventHandler<()>,
    on_edit_active_summary: EventHandler<()>,
    on_toggle_meta: EventHandler<()>,
    on_toggle_settings: EventHandler<()>,
    on_toggle_connect: EventHandler<()>,
    on_toggle_notifications: EventHandler<()>,
    /// Opens (or closes) a pane the active app contributed, by its pane id.
    on_toggle_app_pane: EventHandler<String>,
    /// Open/close the tab tree rail — which IS vertical-tabs mode.
    on_toggle_web_tabs: EventHandler<()>,
    on_restart_update: EventHandler<()>,
    on_request_window_drag: EventHandler<()>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_fullscreen: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
    on_close_app: EventHandler<()>,
    maximized: bool,
    fullscreen: bool,
) -> Element {
    let command_mode_active = snapshot.command_mode_active;
    let search_query = snapshot.search_query.clone();
    let native_macos_titlebar = cfg!(target_os = "macos");
    let titlebar_leading_inset = if native_macos_titlebar { 88 } else { 0 };
    let active_title = snapshot.active_title.clone().or_else(|| {
        snapshot
            .active_session
            .as_ref()
            .map(|session| session.title.clone())
    });
    let active_summary = snapshot.active_summary.clone().or_else(|| {
        snapshot
            .active_session
            .as_ref()
            .and_then(|session| titlebar_summary_text(&snapshot, session))
    });
    let titlebar_loading_label = if snapshot.active_view_mode == WorkspaceViewMode::Rendered {
        snapshot
            .preview_loading
            .then_some("Refreshing Web View…".to_string())
    } else {
        None
    };
    let active_session_path_value = snapshot
        .active_session_path
        .clone()
        .unwrap_or_else(|| "__none__".to_string());
    let titlebar_session_key = format!(
        "titlebar-session:{}:{}:{}",
        active_session_path_value,
        active_title.clone().unwrap_or_default(),
        active_summary.clone().unwrap_or_default()
    );
    let titlebar_session_menu_open = snapshot.titlebar_session_menu_open;
    let search_dropdown_open = titlebar_search_dropdown_open(&snapshot);
    let search_placeholder = if snapshot.command_mode_active {
        "/ Run command"
    } else {
        "Search or type /"
    };
    let search_content_controls_enabled = !snapshot.search_content_hits.is_empty();
    let search_counter_label = if search_content_controls_enabled {
        if let Some(ix) = snapshot.search_content_hit_index {
            format!("{}/{}", ix + 1, snapshot.search_content_hits.len())
        } else {
            format!("0/{}", snapshot.search_content_hits.len())
        }
    } else if !snapshot.search_sidebar_matches.is_empty() {
        format!("{} rows", snapshot.search_sidebar_matches.len())
    } else {
        "0 hits".to_string()
    };
    let search_dropdown_hint = if snapshot.search_active {
        if snapshot.search_sidebar_matches.is_empty() {
            "No matching rows yet. Keep typing or prefix '/' for a command."
        } else {
            "Jump between matching rows and live terminal content from here."
        }
    } else {
        "Search live sessions, folders, and hot terminals. Prefix '/' to run a yggterm command."
    };
    let floating_panel_background = if palette_is_dark(snapshot.palette) {
        "rgb(15,20,25)"
    } else {
        "rgb(242,246,250)"
    };
    let floating_panel_shadow = if palette_is_dark(snapshot.palette) {
        "0 20px 48px rgba(0,0,0,0.30)"
    } else {
        "0 20px 48px rgba(74,93,122,0.19)"
    };
    let floating_panel_border = chrome_chip_border(snapshot.palette);
    let floating_panel_box_shadow = format!(
        "{}, 0 0 0 1px {}",
        floating_panel_shadow, floating_panel_border
    );
    let floating_panel_tab_box_shadow = format!(
        "0 -1px 0 {}, 1px 0 0 {}, -1px 0 0 {}, 0 1px 0 {}",
        floating_panel_border,
        floating_panel_border,
        floating_panel_border,
        floating_panel_background
    );
    let floating_panel_attached_box_shadow = format!(
        "{}, 1px 0 0 {}, -1px 0 0 {}, 0 1px 0 {}",
        floating_panel_shadow, floating_panel_border, floating_panel_border, floating_panel_border
    );
    let search_panel_background = floating_panel_background;
    let search_panel_shadow = floating_panel_box_shadow.as_str();
    let titlebar_modal_tab_height = 28;
    let titlebar_modal_width = "min(500px, 76vw)";
    let search_divider_color = if palette_is_dark(snapshot.palette) {
        "rgba(187,204,219,0.14)"
    } else {
        "rgba(201,214,226,0.9)"
    };
    let search_center_style =
        "position:relative; display:flex; align-items:center; justify-content:center; width:100%; height:100%; min-width:0; overflow:visible;"
            .to_string();
    let titlebar_search_field_shell_style =
        search_field_shell_style(palette_is_dark(snapshot.palette));
    let search_shell_style = if search_dropdown_open {
        format!(
            "position:fixed; left:50vw; top:2px; transform:translateX(-50%); z-index:221; display:flex; flex-direction:column; gap:8px; \
             width:min(520px, calc(100vw - 24px)); min-width:0; height:auto; min-height:0; max-height:min(360px, calc(100vh - 12px)); \
             padding:10px 10px 8px 10px; border-radius:14px; box-sizing:border-box; \
             background:{}; box-shadow:{}; overflow:hidden; pointer-events:auto; transition:background 120ms ease, box-shadow 120ms ease;",
            search_panel_background, search_panel_shadow
        )
    } else {
        "position:fixed; left:50vw; right:auto; top:3px; transform:translateX(-50%); z-index:221; display:flex; flex-direction:row; align-items:center; gap:0; \
         width:min(520px, 100%); min-width:0; height:26px; padding:0; border-radius:0; box-sizing:border-box; background:transparent; box-shadow:none; pointer-events:auto; \
         overflow:visible; transition:none;"
            .to_string()
    };
    let search_modal_style = if search_dropdown_open {
        "display:flex; flex-direction:column; gap:10px; width:100%; min-width:0; padding:0; box-sizing:border-box; overflow:hidden;"
            .to_string()
    } else {
        "display:flex; align-items:center; width:100%; min-width:0; height:100%; padding:0; gap:0; box-sizing:border-box; overflow:visible;"
            .to_string()
    };
    let search_field_shell_style = if search_dropdown_open {
        format!("{} z-index:1;", titlebar_search_field_shell_style).to_string()
    } else {
        titlebar_search_field_shell_style
    };
    let search_dropdown_panel_style = if search_dropdown_open {
        format!(
            "position:static; display:flex; flex:0 0 auto; flex-direction:column; gap:6px; min-width:0; padding:10px 0 0 0; \
             border-top:1px solid {}; background:transparent; box-shadow:none; overflow:hidden;",
            search_divider_color
        )
    } else {
        format!(
            "position:absolute; left:0; right:0; top:calc(100% + 4px); z-index:220; display:flex; flex-direction:column; gap:5px; \
             padding:9px 8px 6px 8px; border-radius:0 0 16px 16px; background:{}; box-shadow:{}; border-top:1px solid {};",
            search_panel_background, search_panel_shadow, search_divider_color
        )
    };
    let search_field_style = if search_dropdown_open {
        format!(
            "{}",
            search_input_style(snapshot.palette.text, palette_is_dark(snapshot.palette)),
        )
    } else {
        search_input_style(snapshot.palette.text, palette_is_dark(snapshot.palette))
    };
    let titlebar_session_panel_background = floating_panel_background;
    // The mirror's ONE question, asked once per titlebar render. Everything the
    // titlebar flips — which cluster is on which edge, which way each cluster
    // reads, which side its menus hang from — comes from this answer.
    let orientation = snapshot.settings.chrome_orientation;
    let tree_edge = orientation.edge(ChromeSlot::Tree);
    let rail_edge = orientation.edge(ChromeSlot::Rail);
    // The ONE surface switch. Asked once, here, so the slot and the probe stamp
    // cannot disagree about what it is showing.
    let surface_switch = titlebar_surface_switch(&snapshot);
    let surface_switch_kind = match surface_switch {
        TitlebarSurfaceSwitch::None => "none",
        TitlebarSurfaceSwitch::Rendered => "rendered",
        TitlebarSurfaceSwitch::Document { .. } => "document",
    };
    rsx! {
        TitlebarChrome {
            background: snapshot.palette.titlebar.to_string(),
            zoom_percent: zoom_percent_f32(snapshot.settings.ui_font_size, 14.0),
            leading_edge: tree_edge,
            leading_inset_px: titlebar_leading_inset,
            on_request_window_drag: on_request_window_drag,
            on_toggle_maximized: on_toggle_maximized,
            leading: rsx! {
                div {
                    // ⚠ LEGACY NAME. This is the TREE cluster, which the chrome
                    // mirror can put on the right; the honest, side-free stamp is
                    // `data-yggui-titlebar-cluster="leading"` on the box around
                    // it. Kept because the responsive CSS and the app-control
                    // probes select on it — those selectors mean "the tree's
                    // half", not "the left half".
                    "data-yggterm-titlebar-left": "1",
                    style: titlebar_cluster_row_style(orientation, 12, ""),
                    button {
                        "data-titlebar-sidebar-button": "1",
                        style: icon_button_style(snapshot.palette),
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        ondoubleclick: |evt| evt.stop_propagation(),
                        onclick: move |_| on_toggle_sidebar.call(()),
                        span {
                            "data-keytip-node": keytip_node_id("sidebar.toggle"),
                            "data-keytip-tip": keytip_tip_attr(&snapshot, "sidebar.toggle"),
                            style: "display:none;",
                        }
                        "☰"
                    }
                    div {
                        class: "yggterm-titlebar-view-toggle",
                        // THE surface switch — one slot, for every kind of
                        // second surface a session can have. Its footprint is
                        // kept even when nothing applies so the rest of the
                        // titlebar holds position: hidden + inert, not removed.
                        //
                        // ⚠ Both arms of the visibility branch emit BOTH keys.
                        // Dioxus applies a style string property-by-property and
                        // never clears a key a later render stops emitting, so
                        // an empty arm would leave `visibility:hidden` latched
                        // the first time a session stopped offering a switch.
                        "data-titlebar-surface-switch": surface_switch_kind,
                        style: format!(
                            "{}{}",
                            segmented_control_track_style(snapshot.palette),
                            if surface_switch == TitlebarSurfaceSwitch::None {
                                " visibility:hidden; pointer-events:none;"
                            } else {
                                " visibility:visible; pointer-events:auto;"
                            },
                        ),
                        onmousedown: |evt| evt.stop_propagation(),
                        if let (TitlebarSurfaceSwitch::Document { document_visible, custom }, Some(switch_path)) =
                            (surface_switch.clone(), snapshot.active_session_path.clone())
                        {
                            if let Some(app_switch) = custom {
                                for segment in app_switch.segments.into_iter() {
                                    {
                                        let is_active = segment.id == app_switch.active;
                                        let action = app_switch.action.clone();
                                        let seg_id = segment.id.clone();
                                        let label = segment.label.clone();
                                        let title = if !segment.title.is_empty() {
                                            segment.title.clone()
                                        } else {
                                            format!("Switch to {label}")
                                        };
                                        rsx! {
                                            button {
                                                key: "{seg_id}",
                                                class: "yggterm-titlebar-view-toggle-segment",
                                                "data-titlebar-surface-switch-segment": "{seg_id}",
                                                style: segmented_control_segment_style(
                                                    snapshot.palette,
                                                    is_active,
                                                    false,
                                                    true,
                                                ),
                                                title: "{title}",
                                                ondoubleclick: |evt| evt.stop_propagation(),
                                                onclick: {
                                                    let switch_path = switch_path.clone();
                                                    let seg_id = seg_id.clone();
                                                    let action = action.clone();
                                                    let on_document_action = on_document_action.clone();
                                                    let on_set_document_surface_visible = on_set_document_surface_visible.clone();
                                                    move |_| {
                                                        on_set_document_surface_visible
                                                            .call((switch_path.clone(), true));
                                                        on_document_action.call((
                                                            switch_path.clone(),
                                                            "topo".to_string(),
                                                            action.clone(),
                                                            Some(seg_id.clone()),
                                                        ));
                                                    }
                                                },
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                            } else {
                                button {
                                    class: "yggterm-titlebar-view-toggle-segment",
                                    "data-titlebar-surface-switch-segment": "document",
                                    style: segmented_control_segment_style(
                                        snapshot.palette,
                                        document_visible,
                                        false,
                                        true,
                                    ),
                                    title: if document_visible {
                                        "The app's document view (you are here)"
                                    } else {
                                        "Show the app's document view"
                                    },
                                    ondoubleclick: |evt| evt.stop_propagation(),
                                    onclick: {
                                        let switch_path = switch_path.clone();
                                        move |_| {
                                            on_set_document_surface_visible
                                                .call((switch_path.clone(), true))
                                        }
                                    },
                                    "📄\u{fe0e} Document"
                                }
                                button {
                                    class: "yggterm-titlebar-view-toggle-segment",
                                    "data-titlebar-surface-switch-segment": "terminal",
                                    style: segmented_control_segment_style(
                                        snapshot.palette,
                                        !document_visible,
                                        false,
                                        true,
                                    ),
                                    title: if document_visible {
                                        "Show the terminal (the app keeps running)"
                                    } else {
                                        "Showing the terminal (the app keeps running)"
                                    },
                                    ondoubleclick: |evt| evt.stop_propagation(),
                                    onclick: {
                                        let switch_path = switch_path.clone();
                                        move |_| {
                                            on_set_document_surface_visible
                                                .call((switch_path.clone(), false))
                                        }
                                    },
                                    "⌨\u{fe0e} Terminal"
                                }
                            }
                        } else {
                            // An agent CLI's rendered transcript ↔ its PTY.
                            button {
                                class: "yggterm-titlebar-view-toggle-segment",
                                "data-titlebar-surface-switch-segment": "rendered",
                                style: segmented_control_segment_style(
                                    snapshot.palette,
                                    snapshot.active_view_mode == WorkspaceViewMode::Rendered,
                                    false,
                                    true,
                                ),
                                ondoubleclick: |evt| evt.stop_propagation(),
                                onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Rendered),
                                span {
                                    "data-keytip-node": keytip_node_id("view.web"),
                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "view.web"),
                                    style: "display:none;",
                                }
                                "Web View"
                            }
                            button {
                                class: "yggterm-titlebar-view-toggle-segment",
                                "data-titlebar-surface-switch-segment": "terminal",
                                style: segmented_control_segment_style(
                                    snapshot.palette,
                                    snapshot.active_view_mode == WorkspaceViewMode::Terminal,
                                    false,
                                    true,
                                ),
                                ondoubleclick: |evt| evt.stop_propagation(),
                                onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Terminal),
                                span {
                                    "data-keytip-node": keytip_node_id("view.terminal"),
                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "view.terminal"),
                                    style: "display:none;",
                                }
                                "Terminal"
                            }
                        }
                    }
                    div {
                        style: titlebar_cluster_row_style(orientation, 6, " flex:1 1 auto;"),
                        div {
                            class: "yggterm-titlebar-new-shell",
                            style: "position:relative; display:flex; align-items:flex-start; height:100%; overflow:visible;",
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: |evt| evt.stop_propagation(),
                            ondoubleclick: |evt| evt.stop_propagation(),
                            if !snapshot.titlebar_new_menu_open {
                                button {
                                        "data-titlebar-new-button": "1",
                                        style: format!(
                                        "display:inline-flex; align-items:center; gap:6px; height:{}px; padding:0 11px; border:none; border-radius:11px; \
                                         background:{}; color:{}; font-size:11px; font-weight:800; cursor:pointer; \
                                         box-shadow: inset 0 0 0 1px {}; white-space:nowrap;",
                                        titlebar_modal_tab_height,
                                        chrome_chip_fill(snapshot.palette, false),
                                        chrome_chip_text_color(snapshot.palette, false, true),
                                        chrome_chip_border(snapshot.palette)
                                        ),
                                    onmousedown: |evt| evt.stop_propagation(),
                                    onclick: move |_| on_toggle_new_menu.call(()),
                                    span {
                                        "data-keytip-node": keytip_node_id("insert.menu"),
                                        "data-keytip-tip": keytip_tip_attr(&snapshot, "insert.menu"),
                                        style: "display:none;",
                                    }
                                    "+"
                                    span {
                                        style: format!("font-size:10px; color:{};", snapshot.palette.muted),
                                        "▾"
                                    }
                                }
                            } else {
                                div {
                                    style: format!(
                                        "display:inline-flex; align-items:center; gap:6px; height:{}px; padding:0 12px 1px 12px; visibility:hidden; pointer-events:none; white-space:nowrap;",
                                        titlebar_modal_tab_height
                                    ),
                                    "+"
                                    span { "▾" }
                                }
                            }
                            if snapshot.titlebar_new_menu_open {
                                div {
                                    "data-titlebar-new-menu": "1",
                                    style: format!(
                                        "position:absolute; {} top:1px; z-index:210; display:flex; flex-direction:column; align-items:{}; \
                                         min-width:184px; background:transparent; overflow:visible; pointer-events:none;",
                                        titlebar_menu_anchor_style(tree_edge, 0.0),
                                        tree_edge.css_justify(),
                                    ),
                                    div {
                                        "data-titlebar-new-button": "1",
                                        style: format!(
                                        "display:inline-flex; align-items:center; gap:6px; height:{}px; padding:0 12px 1px 12px; border:none; border-radius:11px 11px 0 0; \
                                             background:{}; color:{}; font-size:11px; font-weight:800; cursor:pointer; box-shadow:{}; white-space:nowrap; pointer-events:auto;",
                                            titlebar_modal_tab_height,
                                            floating_panel_background,
                                            snapshot.palette.accent,
                                            floating_panel_tab_box_shadow,
                                        ),
                                        onmousedown: |evt| evt.stop_propagation(),
                                        onclick: move |_| on_toggle_new_menu.call(()),
                                        ondoubleclick: |evt| evt.stop_propagation(),
                                        "+"
                                        span {
                                            style: format!("font-size:10px; color:{};", snapshot.palette.accent),
                                            "▾"
                                        }
                                    }
                                    div {
                                        "data-titlebar-new-menu-shell": "1",
                                        "data-yggterm-menu-surface": "1",
                                        style: format!(
                                            "display:flex; flex-direction:column; gap:8px; width:min(292px, 72vw); margin-top:-1px; padding:12px; border-radius:{}; \
                                             background:{}; box-shadow:{}; pointer-events:auto; box-sizing:border-box; overflow:hidden;",
                                            titlebar_attached_menu_radius(tree_edge),
                                            floating_panel_background,
                                            floating_panel_attached_box_shadow,
                                        ),
                                        onclick: |evt| evt.stop_propagation(),
                                        div {
                                            style: "display:flex; flex-direction:column; gap:6px;",
                                            onmousedown: |evt| evt.stop_propagation(),
                                            button {
                                                "data-titlebar-new-menu-action": "1",
                                                class: "yggterm-menu-item",
                                                style: titlebar_new_action_style(snapshot.palette),
                                                onclick: move |_| on_start_session.call(()),
                                                span {
                                                    "data-keytip-node": keytip_node_id("insert.session"),
                                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "insert.session"),
                                                    style: "display:none;",
                                                }
                                                "New Session"
                                            }
                                            button {
                                                "data-titlebar-new-menu-action": "1",
                                                class: "yggterm-menu-item",
                                                style: titlebar_new_action_style(snapshot.palette),
                                                onclick: move |_| on_start_claude_code.call(()),
                                                span {
                                                    "data-keytip-node": keytip_node_id("insert.claude"),
                                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "insert.claude"),
                                                    style: "display:none;",
                                                }
                                                "New Claude Code"
                                            }
                                            button {
                                                "data-titlebar-new-menu-action": "1",
                                                class: "yggterm-menu-item",
                                                style: titlebar_new_action_style(snapshot.palette),
                                                onclick: move |_| on_start_terminal.call(()),
                                                span {
                                                    "data-keytip-node": keytip_node_id("insert.terminal"),
                                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "insert.terminal"),
                                                    style: "display:none;",
                                                }
                                                "New Terminal"
                                            }
                                            // Entries CONTRIBUTED by the libyggterm apps
                                            // installed on this host (~/.yggterm/apps/*.json).
                                            // No app is hardcoded here, and an app whose
                                            // binary was purged is pruned by the daemon
                                            // before it ever reaches this list.
                                            for (app, verb) in app_launcher_entries(&snapshot.apps) {
                                                button {
                                                    key: "app-verb-{app.name}-{verb.id}",
                                                    "data-titlebar-new-menu-action": "1",
                                                    "data-app-verb": "{app.name}:{verb.id}",
                                                    class: "yggterm-menu-item",
                                                    style: titlebar_new_action_style(snapshot.palette),
                                                    onclick: {
                                                        let on_launch_app_verb = on_launch_app_verb.clone();
                                                        let entry = (app.clone(), verb.clone());
                                                        move |_| on_launch_app_verb.call(entry.clone())
                                                    },
                                                    span {
                                                        "data-keytip-node": keytip_node_id(&app_verb_node_key(&app.name, &verb.id)),
                                                        "data-keytip-tip": keytip_tip_attr(&snapshot, &app_verb_node_key(&app.name, &verb.id)),
                                                        style: "display:none;",
                                                    }
                                                    if !app.icon.is_empty() {
                                                        span {
                                                            style: "margin-right:8px;",
                                                            "{app.icon}"
                                                        }
                                                    }
                                                    "{verb.label}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(active_title) = active_title.clone() {
                            div {
                            key: "{titlebar_session_key}",
                            "data-titlebar-session-path": "{active_session_path_value}",
                            class: "yggterm-titlebar-session-shell",
                            style: format!(
                                "position:relative; display:flex; align-items:flex-start; align-self:center; flex:1 1 220px; min-width:0; max-width:360px; height:{}px; overflow:visible; margin-left:0;",
                                titlebar_modal_tab_height
                            ),
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: |evt| {
                                evt.stop_propagation();
                            },
                            ondoubleclick: |evt| evt.stop_propagation(),
                            // A KeyTip belongs ON the thing it acts on. With the
                            // sidebar OPEN, `ALT,E`'s badge is painted on the "here"
                            // row itself and `ALT,J`'s on the Live Sessions row (see
                            // `SidebarRow`). With the sidebar CLOSED there is no row
                            // on screen, and this chip IS the active session — so it
                            // hosts E, and only then. The marker is a hidden span (no
                            // layout, not an interactable); the chip's own click still
                            // opens the session details, which is why it stays exempt.
                            if !snapshot.sidebar_open {
                                span {
                                    "data-keytip-node": keytip_node_id("session.menu"),
                                    "data-keytip-tip": keytip_tip_attr(&snapshot, "session.menu"),
                                    style: "display:none;",
                                }
                            }
                            button {
                                "data-titlebar-session-button": "1",
                                // The old `active-session-menu` exemption is
                                // dissolved (§12.2): ALT,D reaches the details
                                // too, but a visible interactable gets its own
                                // derived letter rather than an alias-excuse.
                                title: if titlebar_session_menu_open { "Close session details" } else { "Session details" },
                                style: format!(
                                    "display:flex; align-items:center; gap:8px; width:100%; height:{}px; padding:0 12px; border:none; \
                                     border-radius:{}; background:{}; color:{}; box-shadow:{}; clip-path:{}; \
                                     font-size:12px; font-weight:700; cursor:pointer; min-width:0; box-sizing:border-box; position:relative; z-index:{};",
                                    titlebar_modal_tab_height,
                                    if titlebar_session_menu_open {
                                        "11px 11px 0 0"
                                    } else {
                                        "11px"
                                    },
                                    if snapshot.titlebar_session_menu_open {
                                        titlebar_session_panel_background
                                    } else {
                                        chrome_chip_fill(snapshot.palette, false)
                                    },
                                    chrome_chip_text_color(snapshot.palette, snapshot.titlebar_session_menu_open, true),
                                    if snapshot.titlebar_session_menu_open {
                                        "none".to_string()
                                    } else {
                                        format!("inset 0 0 0 1px {}", chrome_chip_border(snapshot.palette))
                                    },
                                    "none",
                                    if snapshot.titlebar_session_menu_open { 241 } else { 211 }
                                ),
                                onmousedown: |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                },
                                onclick: move |evt| {
                                    evt.stop_propagation();
                                    on_toggle_session_menu.call(());
                                },
                                ondoubleclick: move |evt| {
                                    evt.stop_propagation();
                                    on_begin_active_rename.call(());
                                },
                                span {
                                    "data-titlebar-title": "1",
                                    title: "Rename session",
                                    style: "min-width:0; flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; text-align:left; cursor:text;",
                                    onmousedown: |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                    },
                                    onclick: move |evt| {
                                        evt.stop_propagation();
                                        on_begin_active_rename.call(());
                                    },
                                    "{active_title}"
                                }
                                if let Some(loading_label) = titlebar_loading_label.clone() {
                                    span {
                                        style: format!(
                                            "display:inline-flex; align-items:center; gap:5px; flex:0 1 auto; min-width:0; \
                                             font-size:10px; font-weight:700; color:{};",
                                            snapshot.palette.muted
                                        ),
                                        span {
                                            class: "yggterm-loading-dot",
                                            style: format!(
                                                "width:6px; height:6px; border-radius:999px; background:{}; animation:yggterm-tree-loading-dot 1.05s ease-in-out infinite;",
                                                snapshot.palette.accent
                                            )
                                        }
                                        span {
                                            style: "min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                                            "{loading_label}"
                                        }
                                    }
                                }
                                span {
                                    style: format!(
                                        "font-size:10px; color:{}; flex:0 0 auto;",
                                        if snapshot.titlebar_session_menu_open {
                                            snapshot.palette.accent
                                        } else {
                                            snapshot.palette.muted
                                        }
                                    ),
                                    "▾"
                                }
                            }
                            if snapshot.titlebar_session_menu_open {
                                div {
                                    "data-titlebar-summary-shell": "1",
                                    style: format!(
                                        "position:absolute; {} top:{}px; z-index:240; width:{}; min-width:max(100%, 352px); height:1px; overflow:visible; pointer-events:none;",
                                        titlebar_menu_anchor_style(tree_edge, 0.0),
                                        titlebar_modal_tab_height - 1,
                                        titlebar_modal_width,
                                    ),
                                    div {
                                        "data-titlebar-summary-menu": "1",
                                        style: format!(
                                            "display:flex; flex-direction:column; gap:12px; width:{}; min-width:max(100%, 352px); margin-top:-1px; \
                                             padding:14px; border:none; border-radius:{}; background:{}; \
                                             box-shadow:{}; pointer-events:auto; box-sizing:border-box; overflow:hidden;",
                                            titlebar_modal_width,
                                            titlebar_attached_menu_radius(tree_edge),
                                            titlebar_session_panel_background,
                                            floating_panel_attached_box_shadow,
                                        ),
                                        onmousedown: |evt| evt.stop_propagation(),
                                        onclick: |evt| evt.stop_propagation(),
                                        div {
                                            style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                                            div {
                                                "data-titlebar-summary-title": "1",
                                                title: "Rename session",
                                                style: format!("font-size:13px; font-weight:800; color:{}; min-width:0; flex:1; cursor:text;", snapshot.palette.text),
                                                onmousedown: |evt| {
                                                    evt.prevent_default();
                                                    evt.stop_propagation();
                                                },
                                                onclick: move |evt| {
                                                    evt.stop_propagation();
                                                    on_begin_active_rename.call(());
                                                },
                                                "{active_title}"
                                            }
                                            button {
                                                "data-titlebar-summary-action": "copy",
                                                title: "Regenerate title and summary",
                                                style: titlebar_modal_icon_button_style(snapshot.palette),
                                                onmousedown: |evt| {
                                                    evt.prevent_default();
                                                    evt.stop_propagation();
                                                },
                                                onclick: move |_| on_refresh_summary.call(()),
                                                AiSparkleIcon { size: 13 }
                                            }
                                            button {
                                                "data-titlebar-summary-action": "edit-summary",
                                                title: "Edit summary",
                                                style: titlebar_modal_icon_button_style(snapshot.palette),
                                                onmousedown: |evt| {
                                                    evt.prevent_default();
                                                    evt.stop_propagation();
                                                },
                                                onclick: move |_| on_edit_active_summary.call(()),
                                                PencilIcon { size: 13 }
                                            }
                                        }
                                        div {
                                            "data-titlebar-summary-body": "1",
                                            SummaryTimeline {
                                                entries: snapshot.active_summary_timeline.clone(),
                                                fallback: active_summary.clone().unwrap_or_else(|| "Summary not generated yet.".to_string()),
                                                palette: snapshot.palette,
                                                on_edit: on_edit_active_summary,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    }
                }
            },
            center: rsx! {
                div {
                    style: "{search_center_style}",
                    div {
                        "data-yggterm-titlebar-search": "1",
                        "data-titlebar-search-focused": if snapshot.search_focused { "1" } else { "0" },
                        "data-titlebar-search-dropdown-open": if search_dropdown_open { "1" } else { "0" },
                        style: "{search_shell_style}",
                        div {
                            "data-titlebar-search-modal": "1",
                            style: "{search_modal_style}",
                            div {
                                "data-titlebar-search-field-shell": "1",
                                // The shell is a CONTAINER (the input sits inside it), so it
                                // wears the shared field skin and the stylesheet's
                                // :focus-within arm rings it when the inner input focuses.
                                // libyggterm's style is box-only as of v0.3.1.
                                "data-yggui-field": "true",
                                style: "{search_field_shell_style}",
                                onmousedown: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    on_set_search_focus.call(true);
                                    focus_search_input(false);
                                },
                                onclick: move |evt| {
                                    evt.stop_propagation();
                                    on_set_search_focus.call(true);
                                    focus_search_input(false);
                                },
                                ondoubleclick: |evt| evt.stop_propagation(),
                                button {
                                    key: "titlebar-search-activator",
                                    "data-titlebar-search-activator": "1",
                                    // The invisible click target that focuses the
                                    // search field; also anchors the ALT,S badge
                                    // (`search.focus` replaced the bare "/").
                                    // DECLARED — so it needs no exemption; the old
                                    // redundant "search" stamp is gone (§12.1).
                                    "data-keytip-node": keytip_node_id("search.focus"),
                                    style: format!(
                                        "position:absolute; inset:0; z-index:2; border:none; background:transparent; cursor:text; padding:0; margin:0; \
                                         opacity:{}; pointer-events:{};",
                                        if search_dropdown_open { "0" } else { "1" },
                                        if search_dropdown_open { "none" } else { "auto" },
                                    ),
                                    onmousedown: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        on_set_search_focus.call(true);
                                        focus_search_input(false);
                                    },
                                    onclick: move |evt| {
                                        evt.stop_propagation();
                                        on_set_search_focus.call(true);
                                        focus_search_input(false);
                                    },
                                    ondoubleclick: |evt| evt.stop_propagation(),
                                }
                                div {
                                    key: "titlebar-search-field-row",
                                    style: "display:flex; align-items:center; gap:8px; width:100%; min-width:0;",
                                    input {
                                        // UNCONTROLLED on purpose: a controlled
                                        // `value:` rewrite raced the per-keystroke
                                        // tree rebuild and ate/reordered characters
                                        // mid-composition. External sets (Escape,
                                        // clear, app-control) bump the epoch, which
                                        // rebuilds this node with the new
                                        // initial_value — the app-pane widget
                                        // pattern.
                                        key: "titlebar-search-input-{snapshot.search_value_epoch}",
                                        id: SEARCH_INPUT_ID,
                                        // Per-ELEMENT exemption with its reason
                                        // (§12.1): the declared `search.focus`
                                        // activator overlays this exact box and
                                        // already focuses it, so a second derived
                                        // letter here would badge one affordance
                                        // twice.
                                        "data-keytip-exempt": "aliased-by-search.focus",
                                        r#type: "text",
                                        initial_value: "{snapshot.search_query}",
                                        placeholder: "{search_placeholder}",
                                        style: "{search_field_style}",
                                        onmousedown: move |evt| {
                                            evt.stop_propagation();
                                            on_set_search_focus.call(true);
                                        },
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            on_set_search_focus.call(true);
                                            focus_search_input(false);
                                        },
                                        ondoubleclick: |evt| evt.stop_propagation(),
                                        onfocus: move |_| on_set_search_focus.call(true),
                                        onblur: move |_| on_set_search_focus.call(false),
                                        oninput: move |evt| on_search.call(evt.value()),
                                        onkeydown: move |evt| {
                                            if evt.key() == Key::Enter && command_mode_active {
                                                evt.prevent_default();
                                                on_execute_search_command.call(search_query.clone());
                                            }
                                        },
                                    }
                                    // The exit affordance the search never had
                                    // (user call 2026-07-23): clears the query
                                    // AND releases focus — same action as
                                    // Escape, reachable by mouse.
                                    if !snapshot.search_query.is_empty() {
                                        button {
                                            "data-titlebar-search-clear": "1",
                                            // A distinct affordance (clear ≠ focus):
                                            // derived by the overlay-open walk, the
                                            // old blanket "search" stamp is gone.
                                            title: "Clear search (Esc)",
                                            style: format!(
                                                "z-index:3; border:none; background:transparent; cursor:pointer; \
                                                 padding:0 6px; font-size:13px; line-height:1; color:{};",
                                                snapshot.palette.muted
                                            ),
                                            onmousedown: |evt| {
                                                evt.prevent_default();
                                                evt.stop_propagation();
                                            },
                                            onclick: move |evt| {
                                                evt.stop_propagation();
                                                on_clear_search.call(());
                                            },
                                            "✕"
                                        }
                                    }
                                    if snapshot.search_active
                                        && matches!(
                                            snapshot.active_view_mode,
                                            WorkspaceViewMode::Rendered | WorkspaceViewMode::Terminal
                                        )
                                    {
                                        if search_content_controls_enabled {
                                            button {
                                                title: "Previous search hit",
                                                style: chip_style(snapshot.palette, false),
                                                onclick: move |_| on_prev_search_content.call(()),
                                                "↑"
                                            }
                                            button {
                                                title: "Next search hit",
                                                style: chip_style(snapshot.palette, false),
                                                onclick: move |_| on_next_search_content.call(()),
                                                "↓"
                                            }
                                        }
                                        div {
                                            "data-titlebar-search-counter": "1",
                                            style: format!("font-size:11px; color:{}; min-width:62px; text-align:right;", snapshot.palette.muted),
                                            "{search_counter_label}"
                                        }
                                    }
                                }
                            }
                            div {
                                key: "titlebar-search-dropdown",
                                "data-titlebar-search-dropdown": "1",
                                style: format!(
                                    "{} display:{}; pointer-events:{};",
                                    search_dropdown_panel_style,
                                    if search_dropdown_open { "flex" } else { "none" },
                                    if search_dropdown_open { "auto" } else { "none" },
                                ),
                                onmousedown: |evt| evt.stop_propagation(),
                                onclick: |evt| evt.stop_propagation(),
                                div {
                                    "data-titlebar-search-dropdown-header": "1",
                                    style: format!(
                                        "display:flex; align-items:center; justify-content:space-between; gap:10px; padding:0 4px 2px 4px; \
                                         font-size:11.5px; font-weight:600; letter-spacing:-0.012em; color:{}; \
                                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
                                        snapshot.palette.muted
                                    ),
                                    span { "{search_dropdown_hint}" }
                                    span { "Ctrl+Shift+P" }
                                }
                                if snapshot.command_mode_active {
                                    for suggestion in snapshot.search_command_suggestions.iter().take(5) {
                                        button {
                                            "data-titlebar-search-dropdown-entry": "1",
                                            style: format!(
                                                "display:flex; align-items:center; justify-content:space-between; gap:12px; \
                                                 height:31px; border:none; border-radius:10px; padding:0 10px; cursor:pointer; \
                                                 background:{}; color:{}; box-shadow: inset 0 0 0 1px {};",
                                                chrome_chip_fill(snapshot.palette, false),
                                                chrome_chip_text_color(snapshot.palette, false, true),
                                                chrome_chip_border(snapshot.palette)
                                            ),
                                            onclick: {
                                                let command = suggestion.command.clone();
                                                move |_| on_execute_search_command.call(command.clone())
                                            },
                                            span {
                                                style: "font-size:12.5px; font-weight:750; letter-spacing:-0.012em; text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
                                                "{suggestion.command}"
                                            }
                                            span {
                                                style: format!("font-size:11.5px; color:{}; text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;", snapshot.palette.muted),
                                                "{suggestion.description}"
                                            }
                                        }
                                    }
                                } else {
                                    div {
                                        "data-titlebar-search-dropdown-entry": "1",
                                        style: format!(
                                            "display:flex; align-items:center; justify-content:space-between; gap:12px; height:31px; \
                                             padding:0 10px; border-radius:10px; background:{}; color:{}; \
                                             box-shadow: inset 0 0 0 1px {};",
                                            chrome_chip_fill(snapshot.palette, false),
                                            chrome_chip_text_color(snapshot.palette, false, true),
                                            chrome_chip_border(snapshot.palette)
                                        ),
                                        span {
                                            style: "font-size:12.5px; font-weight:750; letter-spacing:-0.012em; text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
                                            if snapshot.search_active {
                                                {format!("{} matching rows", snapshot.search_sidebar_matches.len())}
                                            } else {
                                                "Search sessions and live terminals"
                                            }
                                        }
                                        span {
                                            style: format!("font-size:11.5px; color:{}; text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;", snapshot.palette.muted),
                                            if snapshot.search_active {
                                                if let Some(ix) = snapshot.search_content_hit_index {
                                                    {format!("content {}/{}", ix + 1, snapshot.search_content_hits.len())}
                                                } else {
                                                    {format!("content 0/{}", snapshot.search_content_hits.len())}
                                                }
                                            } else {
                                                "Prefix '/' for commands"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            trailing: rsx! {
                div {
                    // ⚠ LEGACY NAME, as above: this is the RAIL cluster, and the
                    // mirror can put it on the left.
                    "data-yggterm-titlebar-right": "1",
                    style: format!(
                        "position:relative; justify-content:flex-end; {}",
                        titlebar_cluster_row_style(orientation, 8, "")
                    ),
                    onclick: |evt| evt.stop_propagation(),
                    style { "{TITLEBAR_RESPONSIVE_CSS}" }
                    if let Some(update) = snapshot.pending_update_restart.clone() {
                        button {
                            class: "yggterm-titlebar-inline-update",
                            style: format!(
                                "display:inline-flex; align-items:center; gap:7px; height:26px; padding:0 11px; border:none; border-radius:10px; \
                                 background:{}; color:{}; font-size:11px; font-weight:700; cursor:pointer; \
                                 box-shadow: inset 0 0 0 1px {}; white-space:nowrap;",
                                chrome_chip_fill(snapshot.palette, true),
                                snapshot.palette.accent
                                ,
                                chrome_chip_border(snapshot.palette)
                            ),
                            onmousedown: |evt| evt.stop_propagation(),
                            ondoubleclick: |evt| evt.stop_propagation(),
                            onclick: move |_| on_restart_update.call(()),
                            "Restart to Use {update.version}"
                        }
                    }
                    button {
                        "data-titlebar-connect-button": "1",
                        class: "yggterm-titlebar-connect-primary",
                        style: connect_button_style(
                            snapshot.palette,
                            snapshot.right_panel_mode == RightPanelMode::Connect
                        ),
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        ondoubleclick: |evt| evt.stop_propagation(),
                        onclick: {
                            let on_toggle_connect = on_toggle_connect.clone();
                            move |evt| {
                                evt.stop_propagation();
                                on_toggle_connect.call(());
                            }
                        },
                        span {
                            "data-keytip-node": keytip_node_id("connect.toggle"),
                            "data-keytip-tip": keytip_tip_attr(&snapshot, "connect.toggle"),
                            style: "display:none;",
                        }
                        "Connect SSH"
                    }
                    div {
                        class: "yggterm-titlebar-inline-secondary",
                        style: "display:flex; align-items:center; gap:8px;",
                        // The TAB TREE of the active web surface. yggterm's own
                        // button (it owns the tabs), so unlike the contributed
                        // buttons below it is not declared by anyone — it simply
                        // appears when the session HAS a surface, and vanishes
                        // with it. It is the same fact as vertical-tabs mode.
                        if snapshot.active_web_surface_overlay.is_some() {
                            button {
                                "data-titlebar-web-tabs-button": "1",
                                title: "Tabs (vertical tab tree)",
                                style: utility_icon_style(
                                    snapshot.palette,
                                    snapshot.right_panel_mode == RightPanelMode::WebTabs,
                                ),
                                onmousedown: |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                },
                                onclick: {
                                    let on_toggle_web_tabs = on_toggle_web_tabs.clone();
                                    move |evt: MouseEvent| {
                                        evt.stop_propagation();
                                        on_toggle_web_tabs.call(());
                                    }
                                },
                                ondoubleclick: |evt| evt.stop_propagation(),
                                "⊟"
                            }
                        }
                        // Buttons CONTRIBUTED by the active libyggterm app. The
                        // rail draws exactly what the app declared over OSC 7717
                        // and nothing else — no app icon is hardcoded here.
                        for pane in snapshot.sidebar_panes.iter().cloned() {
                            button {
                                key: "app-pane-{pane.id}",
                                "data-titlebar-app-pane-button": "{pane.id}",
                                title: "{pane.title}",
                                style: utility_icon_style(
                                    snapshot.palette,
                                    matches!(
                                        &snapshot.right_panel_mode,
                                        RightPanelMode::AppPane(open_pane)
                                            if open_pane.pane == pane.id
                                    )
                                ),
                                onmousedown: |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                },
                                onclick: {
                                    let on_toggle_app_pane = on_toggle_app_pane.clone();
                                    let pane_id = pane.id.clone();
                                    move |evt: MouseEvent| {
                                        evt.stop_propagation();
                                        on_toggle_app_pane.call(pane_id.clone());
                                    }
                                },
                                ondoubleclick: |evt| evt.stop_propagation(),
                                "{pane.icon}"
                            }
                        }
                        button {
                            key: "titlebar-notifications-button",
                            "data-titlebar-notifications-button": "1",
                            style: utility_icon_style(
                                snapshot.palette,
                                snapshot.right_panel_mode == RightPanelMode::Notifications
                            ),
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: {
                                let on_toggle_notifications = on_toggle_notifications.clone();
                                move |evt| {
                                    evt.stop_propagation();
                                    on_toggle_notifications.call(());
                                }
                            },
                            ondoubleclick: |evt| evt.stop_propagation(),
                            span {
                                "data-keytip-node": keytip_node_id("notifications.toggle"),
                                "data-keytip-tip": keytip_tip_attr(&snapshot, "notifications.toggle"),
                                style: "display:none;",
                            }
                            BellIcon {}
                        }
                        button {
                            key: "titlebar-settings-button",
                            "data-titlebar-settings-button": "1",
                            style: utility_icon_style_sized(
                                snapshot.palette,
                                snapshot.right_panel_mode == RightPanelMode::Settings,
                                17
                            ),
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: {
                                let on_toggle_settings = on_toggle_settings.clone();
                                move |evt| {
                                    evt.stop_propagation();
                                    on_toggle_settings.call(());
                                }
                            },
                            ondoubleclick: |evt| evt.stop_propagation(),
                            span {
                                "data-keytip-node": keytip_node_id("settings.toggle"),
                                "data-keytip-tip": keytip_tip_attr(&snapshot, "settings.toggle"),
                                style: "display:none;",
                            }
                            "⚙"
                        }
                        button {
                            key: "titlebar-metadata-button",
                            "data-titlebar-metadata-button": "1",
                            style: utility_icon_style(
                                snapshot.palette,
                                snapshot.right_panel_mode == RightPanelMode::Metadata
                            ),
                            onmousedown: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                            onclick: {
                                let on_toggle_meta = on_toggle_meta.clone();
                                move |evt| {
                                    evt.stop_propagation();
                                    on_toggle_meta.call(());
                                }
                            },
                            ondoubleclick: |evt| evt.stop_propagation(),
                            span {
                                "data-keytip-node": keytip_node_id("metadata.toggle"),
                                "data-keytip-tip": keytip_tip_attr(&snapshot, "metadata.toggle"),
                                style: "display:none;",
                            }
                            "ⓘ"
                        }
                    }
                    button {
                        "data-titlebar-overflow-button": "1",
                        class: "yggterm-titlebar-overflow-trigger",
                        title: "More actions",
                        style: utility_icon_style(
                            snapshot.palette,
                            snapshot.titlebar_overflow_menu_open
                        ),
                        onmousedown: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        onclick: {
                            let on_toggle_overflow_menu = on_toggle_overflow_menu.clone();
                            move |evt| {
                                evt.stop_propagation();
                                on_toggle_overflow_menu.call(());
                            }
                        },
                        ondoubleclick: |evt| evt.stop_propagation(),
                        "⋮"
                    }
                    if snapshot.titlebar_overflow_menu_open {
                        div {
                            "data-titlebar-overflow-menu": "1",
                            style: format!(
                                "position:absolute; {} top:34px; z-index:220; min-width:196px; padding:8px; border-radius:14px; \
                                 background:{}; box-shadow:{}; display:flex; flex-direction:column; gap:6px;",
                                // The overflow menu hangs off the RAIL cluster's
                                // edge, not off its own trigger box, so its inset
                                // follows the rail.
                                titlebar_menu_anchor_style(rail_edge, 110.0),
                                if palette_is_dark(snapshot.palette) {
                                    "rgba(14,19,24,0.98)"
                                } else {
                                    "rgba(255,255,255,0.98)"
                                },
                                if palette_is_dark(snapshot.palette) {
                                    "0 16px 34px rgba(0,0,0,0.22), inset 0 0 0 1px rgba(187,204,219,0.14)"
                                } else {
                                    "0 16px 34px rgba(74,93,122,0.12), inset 0 0 0 1px rgba(198,212,226,0.84)"
                                },
                            ),
                            onmousedown: |evt| evt.stop_propagation(),
                            if let Some(update) = snapshot.pending_update_restart.clone() {
                                button {
                                    style: titlebar_new_action_style(snapshot.palette),
                                    onclick: move |_| {
                                        on_close_overflow_menu.call(());
                                        on_restart_update.call(());
                                    },
                                    "Restart to Use {update.version}"
                                }
                            }
                            button {
                                style: titlebar_new_action_style(snapshot.palette),
                                onclick: move |_| {
                                    on_close_overflow_menu.call(());
                                    on_toggle_connect.call(());
                                },
                                "Connect SSH"
                            }
                            button {
                                style: titlebar_new_action_style(snapshot.palette),
                                onclick: move |_| {
                                    on_close_overflow_menu.call(());
                                    on_toggle_notifications.call(());
                                },
                                "Notifications"
                            }
                            button {
                                style: titlebar_new_action_style(snapshot.palette),
                                onclick: move |_| {
                                    on_close_overflow_menu.call(());
                                    on_toggle_settings.call(());
                                },
                                "Settings"
                            }
                            button {
                                style: titlebar_new_action_style(snapshot.palette),
                                onclick: move |_| {
                                    on_close_overflow_menu.call(());
                                    on_toggle_meta.call(());
                                },
                                "Session Metadata"
                            }
                        }
                    }
                }
            },
            // NOT mirrored: the window buttons belong to the platform. The
            // mirror is an app-chrome preference; where the compositor expects
            // minimise/maximise/close is not ours to flip, so this slot rides
            // whichever cluster shares the physical right edge and stays
            // outermost on it.
            window_controls: rsx! {
                div {
                    "data-yggterm-titlebar-window-controls": "1",
                    style: "display:flex; align-items:center; height:100%;",
                    div { style: "flex:1; min-width:24px; max-width:40px; height:100%;" }
                    if native_macos_titlebar {
                        WindowControlsStrip {
                            palette: ChromePalette {
                                titlebar: snapshot.palette.titlebar,
                                text: snapshot.palette.text,
                                muted: snapshot.palette.muted,
                                accent: snapshot.palette.accent,
                                close_hover: snapshot.palette.close_hover,
                                control_hover: snapshot.palette.control_hover,
                                is_dark: palette_is_dark(snapshot.palette),
                            },
                            hovered: hovered(),
                            on_hover_control: on_hover_control,
                            on_toggle_maximized: on_toggle_maximized,
                            on_toggle_fullscreen: on_toggle_fullscreen,
                            on_toggle_always_on_top: on_toggle_always_on_top,
                            on_close_app: on_close_app,
                            maximized: maximized,
                            fullscreen: fullscreen,
                            always_on_top: snapshot.always_on_top,
                            show_always_on_top_button: true,
                            show_fullscreen_button: true,
                            show_window_buttons: false,
                            overlay: false,
                        }
                    } else {
                        WindowControlsStrip {
                            palette: ChromePalette {
                                titlebar: snapshot.palette.titlebar,
                                text: snapshot.palette.text,
                                muted: snapshot.palette.muted,
                                accent: snapshot.palette.accent,
                                close_hover: snapshot.palette.close_hover,
                                control_hover: snapshot.palette.control_hover,
                                is_dark: palette_is_dark(snapshot.palette),
                            },
                            hovered: hovered(),
                            on_hover_control: on_hover_control,
                            on_toggle_maximized: on_toggle_maximized,
                            on_toggle_fullscreen: on_toggle_fullscreen,
                            on_toggle_always_on_top: on_toggle_always_on_top,
                            on_close_app: on_close_app,
                            maximized: maximized,
                            fullscreen: fullscreen,
                            always_on_top: snapshot.always_on_top,
                            show_always_on_top_button: true,
                            show_fullscreen_button: true,
                            show_window_buttons: true,
                            overlay: false,
                        }
                    }
                }
            },
        }
    }
}
#[component]
fn WindowResizeHandles() -> Element {
    rsx! {
        ResizeHandle {
            style: format!("position:absolute; top:0; left:0; width:{}px; height:{}px; z-index:120; cursor:nwse-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::NorthWest,
        }
        ResizeHandle {
            style: format!("position:absolute; top:0; right:0; width:{}px; height:{}px; z-index:120; cursor:nesw-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::NorthEast,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; left:0; width:{}px; height:{}px; z-index:120; cursor:nesw-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::SouthWest,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; right:0; width:{}px; height:{}px; z-index:120; cursor:nwse-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::SouthEast,
        }
        ResizeHandle {
            style: format!("position:absolute; top:0; left:{}px; right:{}px; height:{}px; z-index:119; cursor:ns-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::North,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; left:{}px; right:{}px; height:{}px; z-index:119; cursor:ns-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::South,
        }
        ResizeHandle {
            style: format!("position:absolute; top:{}px; bottom:{}px; left:0; width:{}px; z-index:119; cursor:ew-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::West,
        }
        ResizeHandle {
            style: format!("position:absolute; top:{}px; bottom:{}px; right:0; width:{}px; z-index:119; cursor:ew-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::East,
        }
    }
}
#[component]
fn ResizeHandle(style: String, direction: ResizeDirection) -> Element {
    rsx! {
        div {
            style: "{style}",
            onmousedown: move |evt| {
                evt.stop_propagation();
                let _ = window().drag_resize_window(direction);
            },
            ondoubleclick: |evt| evt.stop_propagation(),
        }
    }
}
