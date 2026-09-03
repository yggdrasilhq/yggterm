#[component]
fn RightRail(
    snapshot: SharedSnapshot,
    /// The right edge's reveal state machine. Live only while the rail is
    /// hidden — a hidden rail IS an auto-hide rail.
    autohide: AutoHideSignals,
    /// Is the hidden rail currently revealed as an overlay?
    autohide_revealed: bool,
    /// Is that reveal PINNED by something standing (a modal the rail launched,
    /// KeyTips, keyboard focus in it) rather than by hover? Stamped into the
    /// DOM so the page-placement rule can tell a standing claim from a
    /// transient one.
    autohide_pinned: bool,
    /// Begin a rail-width drag (client x at grab). Mirrors the tree's resize.
    on_start_rail_resize: EventHandler<f64>,
    on_endpoint_change: EventHandler<String>,
    on_api_key_change: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_open_launch_flags: EventHandler<MouseEvent>,
    on_open_cli_install: EventHandler<MouseEvent>,
    on_focus_input: EventHandler<String>,
    on_blur_input: EventHandler<()>,
    on_set_ui_theme: EventHandler<UiTheme>,
    on_open_theme_editor: EventHandler<MouseEvent>,
    on_open_keymap_editor: EventHandler<MouseEvent>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_set_terminal_telemetry: EventHandler<bool>,
    on_set_perf_profiling: EventHandler<bool>,
    on_set_titlebar_auto_hide: EventHandler<bool>,
    /// Flip the whole app chrome about the window's vertical centre line.
    on_set_chrome_mirrored: EventHandler<bool>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_set_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
    on_set_main_zoom: EventHandler<i32>,
    on_set_terminal_theme_name: EventHandler<(UiTheme, String)>,
    on_trigger_update: EventHandler<MouseEvent>,
    on_connect_ssh_custom: EventHandler<MouseEvent>,
    on_ssh_target_change: EventHandler<String>,
    on_ssh_prefix_change: EventHandler<String>,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
    /// User asked for a daemon hot-restart by hand, from the metadata rail.
    on_daemon_hot_restart: EventHandler<MouseEvent>,
    /// (pane_id, action, value) — fired by any widget in a contributed pane.
    on_app_pane_action: EventHandler<(String, String, Option<String>)>,
    /// `(pane id, action, moved row id, the pane's new row order)` — fired
    /// when a reorderable rail row is dropped somewhere that changes the order.
    on_app_pane_reorder: EventHandler<(String, String, String, Option<String>, Vec<String>)>,
    /// (widget_id, value) — a draft input changed; stays in the GUI until an
    /// action carries it to the app.
    on_app_pane_value: EventHandler<(String, String)>,
    /// The tab rail edits live tab state directly (select, close, file, rename a
    /// folder) — a dozen event handlers threaded as props would be noise, and
    /// these are yggterm's OWN tabs, not an app's contributed schema.
    state: Signal<ShellState>,
) -> Element {
    // ⛔ ONE owner for "what is the rail drawing" — [`rail_render_view`], which
    // the agent probe reads too. A private copy here is how the pixels and
    // `server app state` came to disagree.
    let RailRenderView {
        requested_mode: _,
        rendered_mode,
        docked: visible,
    } = rail_render_view(&snapshot);
    // Extracted before the rsx! chain: an `if let` arm inside it defeats the
    // macro's branch-type inference.
    // Only the pane ID reaches the body: `rendered_mode` can be `AppPane` ONLY
    // when `app_pane_available` said the owner is the active session, so every
    // active-session lookup below it resolves to that same owner by
    // construction. That invariant is what keeps the body from needing the
    // session too — break it and this must carry the whole ref.
    let rendered_app_pane_id = match &rendered_mode {
        RightPanelMode::AppPane(open_pane) => Some(open_pane.pane.clone()),
        _ => None,
    };
    // A hidden rail hover-reveals from the right edge as a floating card, exactly
    // as a hidden session tree does from the left — SAME geometry helper, mirrored
    // edge, same fixed-key styles. Docked, it is the classic in-flow rail. Out of
    // flow when auto-hidden, so a hover never resizes the viewport.
    let mode = if visible {
        SidebarPanelMode::InFlow
    } else if autohide_revealed {
        SidebarPanelMode::Revealed
    } else {
        SidebarPanelMode::Collapsed
    };
    let rail_width = snapshot.rail_width;
    // Same rule as the tree: ASK which edge this panel is on. Mirrored, the rail
    // is the LEFT panel and everything about it — anchor, shadow, collapse
    // slide, grip side, drag sign — follows from this one answer.
    let rail_edge = chrome_slot_edge(&snapshot, ChromeSlot::Rail);
    let outer_style = sidebar_panel_outer_style(
        rail_edge,
        mode,
        rail_width,
        zoom_percent_f32(snapshot.settings.ui_font_size, 14.0),
    );
    let content_style = sidebar_panel_card_style(rail_edge, mode, rail_width, snapshot.palette);
    let rail_reveal = (!visible).then(|| SideRailReveal {
        on_reveal: EventHandler::new(move |_| autohide.reveal()),
        on_reveal_if_idle: EventHandler::new(move |_| autohide.reveal_if_idle()),
        on_mouse_leave: EventHandler::new(move |_| autohide.handle_mouse_leave()),
        on_focus_within: EventHandler::new(move |focused: bool| autohide.set_focus_within(focused)),
    });
    // A grip on the rail's INNER edge, mirroring the tree's grip on ITS inner
    // edge — same helper, so the mirror moves both without either being told
    // about the other. Rendered in EVERY mode so a hidden rail is draggable too
    // (the grip lives inside the card; a fully-collapsed card is un-clickable,
    // so you hover-reveal first, then drag, and the drag docks + resizes).
    // `start_rail_resize` un-hides to the last shown mode.
    let rail_resize_handle = Some(rsx! {
        div {
            "data-rail-resize-handle": "1",
            style: sidebar_resize_handle_style(rail_edge),
            onmousedown: move |evt| {
                evt.stop_propagation();
                on_start_rail_resize.call(evt.client_coordinates().x);
            },
            ondoubleclick: |evt| evt.stop_propagation(),
        }
    });
    rsx! {
        SideRailShell {
            visible: visible,
            auto_hide: !visible,
            revealed: autohide_revealed && !visible,
            pinned: autohide_pinned && !visible,
            outer_style: outer_style,
            content_style: content_style,
            reveal: rail_reveal,
            resize_handle: rail_resize_handle,
            cover_label: "sidebar-rail",
            body: rsx!{
            if rendered_mode == RightPanelMode::Metadata {
                MetadataRailBody { snapshot: snapshot.clone(), on_daemon_hot_restart }
            } else if rendered_mode == RightPanelMode::Settings {
                SettingsRailBody {
                    snapshot: snapshot.clone(),
                    on_endpoint_change,
                    on_api_key_change,
                    on_model_change,
                    on_open_launch_flags,
                    on_open_cli_install,
                    on_focus_input,
                    on_blur_input,
                    on_set_ui_theme,
                    on_open_theme_editor,
                    on_open_keymap_editor,
                    on_set_notification_delivery,
                    on_set_notification_sound,
                    on_set_terminal_telemetry,
                    on_set_perf_profiling,
                    on_set_titlebar_auto_hide,
                    on_set_chrome_mirrored,
                    on_adjust_ui_zoom,
                    on_set_ui_zoom,
                    on_adjust_main_zoom,
                    on_set_main_zoom,
                    on_set_terminal_theme_name,
                    on_trigger_update,
                }
            } else if rendered_mode == RightPanelMode::Connect {
                ConnectRailBody {
                    snapshot: snapshot.clone(),
                    on_connect_ssh_custom,
                    on_ssh_target_change,
                    on_ssh_prefix_change,
                }
            } else if rendered_mode == RightPanelMode::Notifications {
                NotificationsRailBody {
                    snapshot: snapshot.clone(),
                    on_clear_notification,
                    on_clear_notifications,
                    on_activate_notification: move |session_path: String| {
                        spawn_open_session_from_notification(state, session_path)
                    },
                }
            } else if rendered_mode == RightPanelMode::WebTabs {
                WebTabsRailBody { snapshot: snapshot.clone(), state }
            } else if rendered_app_pane_id.is_some() {
                AppPaneRailBody {
                    snapshot: snapshot.clone(),
                    pane_id: rendered_app_pane_id.clone().unwrap_or_default(),
                    on_app_pane_action,
                    on_app_pane_reorder,
                    on_app_pane_value,
                    state,
                }
            }
            }
        }
    }
}
/// The web chrome's input-pill border. Named because the find bar swaps it for
/// a danger tint on a no-match query and the two must otherwise be the SAME
/// pill — a find field that is a different shape from the address field above it
/// reads as two products stapled together.
const WEB_CHROME_INPUT_BORDER: &str = "rgba(127,127,127,0.35)";
/// The danger tint (`#ef4444`, the shell's one red) at pill-border strength.
const WEB_CHROME_INPUT_BORDER_NO_MATCH: &str = "rgba(239,68,68,0.55)";
/// The web chrome's small glyph button: transparent, theme-foreground, dimmed
/// when inert. ONE definition, worn by the omnibox's back/forward/reload/history
/// cluster and by the find bar's prev/next/close — see the reuse doctrine.
fn web_chrome_icon_button_style(foreground: &str, enabled: bool) -> String {
    format!(
        "border:none; background:transparent; color:{}; font-size:14px; line-height:1; padding:4px 7px; border-radius:6px; cursor:{}; opacity:{};",
        foreground,
        if enabled { "pointer" } else { "default" },
        if enabled { "0.85" } else { "0.3" },
    )
}
/// The pill's box sizing, as its callers need it: the omnibox fills its bar (or
/// its own line in the ~300px rail), the find pill is capped so the `3/17` and
/// the buttons keep the right edge they are anchored to. A PARAMETER rather
/// than something a caller appends, because appending `flex:` to the returned
/// string emits the same property twice in one style attribute.
const WEB_CHROME_INPUT_FLEX_FILL: &str = "1 1 auto";
const WEB_CHROME_INPUT_FLEX_FILL_COMPACT: &str = "1 1 100%";
const WEB_CHROME_INPUT_FLEX_FIND: &str = "0 1 240px";
/// The web chrome's input pill. `compact` is the ~300px rail variant (the
/// omnibox's Zen home, where the field drops onto its own line); `border` and
/// `flex` are the only things a caller may vary.
///
/// ONE format string, no branch: the key set is therefore identical by
/// construction, not by a promise a later edit can break — the Dioxus trap
/// where a dropped key never clears (a compact-only `order` / `margin-top`
/// would stay applied after a re-render that no longer emits it).
fn web_chrome_input_style(foreground: &str, compact: bool, border: &str, flex: &str) -> String {
    let (order, margin_top, padding, radius, font_size) = if compact {
        ("9", "4px", "6px 12px", "12px", "12px")
    } else {
        ("0", "0", "5px 14px", "14px", "12.5px")
    };
    // ⛔ NO `background`: the pill's fill, hover and focus ring belong to
    // `text_field_css` (it wears `data-yggui-field="pill"`), and an inline
    // background out-specifies a stylesheet — which would leave the omnibox the
    // one flat, inert field in the window. The BORDER stays inline because here
    // it carries state: red when a find has no matches.
    format!(
        "flex:{flex}; order:{order}; min-width:0; margin-top:{margin_top}; padding:{padding}; \
         border-radius:{radius}; border:1px solid {border}; \
         color:{foreground}; font-size:{font_size}; outline:none;",
    )
}
// ===== MIDDLE-CLICK ON A NAV CONTROL: THE SAME ACTION, IN A NEW TAB =========
//
// The grammar every browser already taught the user: a middle-click does what a
// left-click does, but somewhere ELSE. Back, forward, reload and history each
// had exactly one behaviour — in place — so the only way to see the previous
// page without losing this one was to duplicate the tab first and step the copy.
//
// "Somewhere else" is NOT a new destination. `WebTabOrigin::Opener` already
// means "above this tab, before the children it already has" — it is what the
// row menu's "New tab above this one" uses and what a middle-clicked LINK uses —
// so these route through that same owner. A second middle-click therefore
// cascades above the first instead of shoving in between them.

/// Is this the middle button?
///
/// One spelling, so the four controls cannot disagree about what a middle-click
/// is — and so the `onclick` refusal below is testably the same question the
/// `onmouseup` action asks.
fn web_nav_middle_click(evt: &MouseEvent) -> bool {
    evt.trigger_button() == Some(MouseButton::Auxiliary)
}

/// WHERE a middle-clicked Back or Forward would land: that entry's URL, from the
/// ENGINE's own history list.
///
/// ⚠ **Not `WebSurfaceOverlayView::back_target`'s URL.** That pair carries the
/// ACTIVE tab's own address — it is the button's ENABLEMENT, which is all that
/// stepping in place ever needed, because `go_back` needs no URL to act on.
/// Opening it would hand the user a second copy of the page they are already on.
/// The engine is the only thing that knows the previous address on a site
/// browsed by clicking links, which is every site.
fn web_surface_nav_target_url(session_path: &str, tab_id: u64, forward: bool) -> Option<String> {
    // The ONE owner of (session, tab) -> native id, the same one the stepper
    // resolves through.
    let native_id = web_surface_native_id_for(session_path, tab_id)?;
    dioxus_desktop::window().web_surface_nav_target_url(native_id, forward)
}

/// Do a nav control's action in a NEW TAB below the active one.
///
/// BACKGROUND, deliberately: Chrome's grammar and this codebase's own (see
/// [`WebTabOpenRequest::opened_by`]) is that a middle-click opens something
/// without going there, so the tab the user is reading keeps the front — and
/// the tab it minted carries a URL, so it never steals the keyboard either.
fn open_web_nav_target_in_new_tab(
    state: Signal<ShellState>,
    session_path: &str,
    opener_tab_id: u64,
    url: String,
    ssh_target: Option<String>,
) {
    if url.is_empty() {
        return;
    }
    // THE placement owner — `Opener` is "below this tab, after its existing
    // children". Nothing here decides placement for itself.
    let Some(tab_id) = open_web_surface_tab(
        state,
        session_path,
        WebTabOpenRequest::opened_by(opener_tab_id, true),
    ) else {
        return;
    };
    navigate_web_surface_tab(
        state,
        session_path.to_string(),
        tab_id,
        url,
        ssh_target,
        None,
    );
}

/// The browser omnibox with its navigation controls: back / forward / reload,
/// the address input (Chrome-style inline history completion + a keyboard-driven
/// suggestion dropdown), and the history-viewer button. ONE implementation for
/// two homes — the viewport nav bar in classic (horizontal-tabs) mode, and the
/// top of the tab-tree rail in vertical-tabs mode (the Zen-style omnibox). The
/// address bar never lives in both at once: the viewport bar draws only when
/// vertical tabs are OFF, and the rail exists only when they are ON.
///
/// `compact` selects the rail styling — the buttons and input wrap so the input
/// drops to its own line under the nav cluster in a ~300px rail. `input_id` must
/// be a stable, unique DOM id so the focus/selection scripts address the right
/// field.
#[component]
fn WebOmniboxBar(
    state: Signal<ShellState>,
    session_path: String,
    input_id: String,
    ssh_target: Option<String>,
    overlay: WebSurfaceOverlayView,
    foreground: String,
    background: String,
    compact: bool,
) -> Element {
    let nav_path = session_path.clone();
    let nav_ssh = ssh_target.clone();
    let active_tab_id = overlay.active_tab_id;
    let nav_profile = overlay.profile.clone();
    let back_target = overlay.back_target.clone();
    let forward_target = overlay.forward_target.clone();
    let address_text = overlay.address_text.clone();
    let address_editing = overlay.address_editing;
    let suggestions = overlay.address_suggestions.clone();
    // How many rows the PALETTE is offering, which is what the inline field's
    // arrow keys have to step through — the palette and this field are two views
    // of one selection, so the count must be the palette's, not a second tally.
    let dropdown_rows = if address_editing {
        web_omnibox_palette_items(&address_text, &suggestions).len()
    } else {
        0
    };
    let nav_button_style = |enabled: bool| web_chrome_icon_button_style(&foreground, enabled);
    let back_style = nav_button_style(back_target.is_some());
    let forward_style = nav_button_style(forward_target.is_some());
    let reload_style = nav_button_style(true);
    // In the rail (compact) the cluster wraps and the input drops onto its own
    // line below the buttons (a 300px rail is too narrow for one row); over the
    // page it is the classic single-row nav bar.
    let bar_style = if compact {
        "display:flex; flex-wrap:wrap; align-items:center; gap:3px; padding:0 0 2px; user-select:none;".to_string()
    } else {
        format!(
            "display:flex; align-items:center; gap:4px; padding:6px 10px; background:{background}; user-select:none; \
             overflow:hidden; max-height:60px;",
        )
    };
    let input_style = web_chrome_input_style(
        &foreground,
        compact,
        WEB_CHROME_INPUT_BORDER,
        if compact {
            WEB_CHROME_INPUT_FLEX_FILL_COMPACT
        } else {
            WEB_CHROME_INPUT_FLEX_FILL
        },
    );
    rsx! {
        div {
            style: "{bar_style}",
            button {
                style: "{back_style}",
                title: "Back",
                disabled: back_target.is_none(),
                onclick: {
                    let nav_path = nav_path.clone();
                    let back_target = back_target.clone();
                    move |evt: MouseEvent| {
                        // The middle button is `onmouseup`'s, and it opens a new
                        // tab. If the engine ever routes one here as well, it
                        // must not ALSO step this tab — the user would get both.
                        if web_nav_middle_click(&evt) {
                            return;
                        }
                        // Step the ENGINE's history, not a URL the shell
                        // remembered: a re-navigation to the previous address is
                        // not "back" (it loses the page's scroll and form state,
                        // and on a site the user browsed by link clicks the
                        // shell has no previous address at all).
                        if back_target.is_some() {
                            web_surface_step_history(state, &nav_path, active_tab_id, false);
                        }
                    }
                },
                onmouseup: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    move |evt: MouseEvent| {
                        if !web_nav_middle_click(&evt) {
                            return;
                        }
                        evt.prevent_default();
                        evt.stop_propagation();
                        let Some(url) =
                            web_surface_nav_target_url(&nav_path, active_tab_id, false)
                        else {
                            return;
                        };
                        open_web_nav_target_in_new_tab(
                            state,
                            &nav_path,
                            active_tab_id,
                            url,
                            nav_ssh.clone(),
                        );
                    }
                },
                "←"
            }
            button {
                style: "{forward_style}",
                title: "Forward",
                disabled: forward_target.is_none(),
                onclick: {
                    let nav_path = nav_path.clone();
                    let forward_target = forward_target.clone();
                    move |evt: MouseEvent| {
                        if web_nav_middle_click(&evt) {
                            return;
                        }
                        if forward_target.is_some() {
                            web_surface_step_history(state, &nav_path, active_tab_id, true);
                        }
                    }
                },
                onmouseup: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    move |evt: MouseEvent| {
                        if !web_nav_middle_click(&evt) {
                            return;
                        }
                        evt.prevent_default();
                        evt.stop_propagation();
                        let Some(url) = web_surface_nav_target_url(&nav_path, active_tab_id, true)
                        else {
                            return;
                        };
                        open_web_nav_target_in_new_tab(
                            state,
                            &nav_path,
                            active_tab_id,
                            url,
                            nav_ssh.clone(),
                        );
                    }
                },
                "→"
            }
            button {
                style: "{reload_style}",
                title: "Reload",
                onclick: {
                    let nav_path = nav_path.clone();
                    move |evt: MouseEvent| {
                        if web_nav_middle_click(&evt) {
                            return;
                        }
                        state.with_mut_counted(|shell| shell.web_surface_reload_active_tab(&nav_path));
                    }
                },
                onmouseup: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    move |evt: MouseEvent| {
                        if !web_nav_middle_click(&evt) {
                            return;
                        }
                        evt.prevent_default();
                        evt.stop_propagation();
                        // "Reload, elsewhere" is this page in a new tab. The
                        // tab's own URL is the honest answer: the reconciler
                        // follows the ENGINE onto it, so it is the address the
                        // user is looking at, not the one they last typed.
                        let Some(url) = state.with(|shell| {
                            shell.web_surfaces.get(nav_path.as_str()).and_then(|surface| {
                                surface
                                    .tabs
                                    .iter()
                                    .find(|tab| tab.id == active_tab_id)
                                    .map(|tab| tab.url.clone())
                            })
                        }) else {
                            return;
                        };
                        open_web_nav_target_in_new_tab(
                            state,
                            &nav_path,
                            active_tab_id,
                            url,
                            nav_ssh.clone(),
                        );
                    }
                },
                "⟳"
            }
            input {
                id: "{input_id}",
                "data-yggui-field": "pill",
                style: "{input_style}",
                value: "{address_text}",
                spellcheck: "false",
                autocomplete: "off",
                placeholder: "Search or enter address",
                onfocus: {
                    let address_input_id = input_id.clone();
                    let nav_path = nav_path.clone();
                    move |_| {
                        // ⛔⛔ FOCUS IS THE TRIGGER, WHOEVER CAUSED IT. Owner-
                        // reported: *"Not all omnibox highlight in vertical tab
                        // mode spawn the command palette."*
                        //
                        // The palette is DERIVED — it is open exactly when the
                        // active surface holds an `address_draft`
                        // (`web_command_palette_open`). Until now the only
                        // non-test caller of `web_surface_begin_address_edit`
                        // was `focus_web_omnibox`, the path Ctrl+L and a new
                        // typing-ready tab take. **Clicking the field took a
                        // different road**: the DOM focused, the text below
                        // selected — the "highlight" the report names — and no
                        // draft was ever set, so the palette did not open.
                        //
                        // One gesture, two outcomes, decided by which code path
                        // happened to focus the input. Beginning the edit here
                        // makes the DOM's own focus event the single trigger, so
                        // every way in agrees. It is idempotent: the edit begins
                        // from the current URL, and a focus that arrives via
                        // `focus_web_omnibox` has already set the same draft.
                        // ⚠ ONLY when there is no draft yet. Beginning the edit
                        // RESETS the draft to the tab's URL — right for Ctrl+L,
                        // which is a deliberate "address me", and wrong here: a
                        // focus event can arrive while the user is mid-word (a
                        // re-render that returns focus, a click back into a
                        // field they had already typed into), and resetting
                        // then would eat what they had typed. That is the same
                        // complaint as the completion race one handler below,
                        // and it must not be reintroduced by the fix for the
                        // palette.
                        let has_draft = state.with(|shell| {
                            shell
                                .web_surfaces
                                .get(&nav_path)
                                .is_some_and(|surface| surface.address_draft.is_some())
                        });
                        if !has_draft {
                            state.with_mut_counted(|shell| {
                                shell.web_surface_begin_address_edit(&nav_path)
                            });
                        }
                        let _ = document::eval(&format!(
                            r#"(function(){{
                                var el = document.getElementById('{id}');
                                if (!el || !el.select) return;
                                el.select();
                                var guard = function(e){{
                                    e.preventDefault();
                                    el.removeEventListener('mouseup', guard);
                                }};
                                el.addEventListener('mouseup', guard);
                            }})();"#,
                            id = address_input_id,
                        ));
                    }
                },
                oninput: {
                    let nav_path = nav_path.clone();
                    let address_input_id = input_id.clone();
                    move |evt: FormEvent| {
                        let completion = state.with_mut_counted(|shell| {
                            shell.web_surface_type_address(&nav_path, evt.value())
                        });
                        if let Some((completed, typed_len, completed_len)) = completion {
                            let completed_js = serde_json::to_string(&completed)
                                .unwrap_or_else(|_| "\"\"".to_string());
                            // ⛔⛔ THIS WRITE-BACK IS ASYNCHRONOUS AND THE USER IS
                            // STILL TYPING INTO THE FIELD IT WRITES TO. Owner-
                            // reported: *"Typing in the command palette is a
                            // nightmare. It just does not let me type."*
                            //
                            // `oninput` fires, this schedules a frame, and the
                            // frame lands ~16 ms later. A fast typist's next
                            // keystroke arrives FIRST — and then the frame
                            // overwrites the field with the completion computed
                            // for the PREVIOUS keystroke, discarding the
                            // character that was just typed. It reads exactly
                            // like the field fighting you, and it gets worse the
                            // faster you type, which is why it feels like the
                            // omnibox refusing input rather than a race.
                            //
                            // ⇒ The completion is only valid for the text it was
                            // computed FROM. Check that the field still holds
                            // that prefix at the moment the frame runs; if the
                            // user has moved on, drop this completion silently
                            // — a later `oninput` has already produced the right
                            // one. Never write a value derived from a keystroke
                            // the field has already left behind.
                            // ⚠ `typed_len` is a BYTE offset (it comes from
                            // `value.len()`), so slice by bytes — and guard the
                            // boundary rather than risk a panic on a non-ASCII
                            // address the user pasted a prefix of.
                            let typed_prefix = completed
                                .is_char_boundary(typed_len)
                                .then(|| completed[..typed_len].to_string())
                                .unwrap_or_default();
                            let typed_prefix_js = serde_json::to_string(&typed_prefix)
                                .unwrap_or_else(|_| "\"\"".to_string());
                            let _ = document::eval(&format!(
                                r#"requestAnimationFrame(function(){{
                                    var el = document.getElementById('{id}');
                                    if (!el) return;
                                    // Still what we completed from? A completion
                                    // for text the user has left behind must not
                                    // land on top of what they typed since.
                                    if (el.value !== {typed_prefix} && el.value !== {completed}) return;
                                    if (el.value !== {completed}) el.value = {completed};
                                    if (el.setSelectionRange) el.setSelectionRange({start}, {end});
                                }});"#,
                                id = address_input_id,
                                completed = completed_js,
                                typed_prefix = typed_prefix_js,
                                start = typed_len,
                                end = completed_len,
                            ));
                        }
                    }
                },
                onkeydown: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    let suggestions = suggestions.clone();
                    move |evt: KeyboardEvent| {
                        if evt.key() == Key::ArrowDown || evt.key() == Key::ArrowUp {
                            if dropdown_rows > 0 {
                                evt.prevent_default();
                                let delta = if evt.key() == Key::ArrowDown { 1 } else { -1 };
                                state.with_mut_counted(|shell| {
                                    shell.web_surface_move_address_suggestion(&nav_path, delta, dropdown_rows);
                                });
                            }
                        } else if evt.key() == Key::Enter {
                            evt.prevent_default();
                            // A selected history row navigates to that URL directly;
                            // row 0 (or no selection) commits the draft.
                            let selected = state.with(|shell| {
                                shell.web_surfaces.get(&nav_path).and_then(|surface| surface.address_suggestion_index)
                            });
                            if let Some(index) = selected
                                && index >= 1
                                && let Some((url, _)) = suggestions.get(index - 1)
                            {
                                let tab_id = state.with(|shell| {
                                    shell.web_surfaces.get(&nav_path).map(|surface| surface.active_tab)
                                });
                                if let Some(tab_id) = tab_id {
                                    navigate_web_surface_tab(state, nav_path.clone(), tab_id, url.clone(), nav_ssh.clone(), None);
                                }
                                return;
                            }
                            let target = state.with(|shell| {
                                let surface = shell.web_surfaces.get(&nav_path)?;
                                let text = surface.address_draft.clone().or_else(|| {
                                    surface.tabs.iter().find(|tab| tab.id == surface.active_tab).map(|tab| tab.url.clone())
                                })?;
                                Some((surface.active_tab, text))
                            });
                            if let Some((tab_id, text)) = target
                                && let Some(url) = web_surface_address_to_url(&text)
                            {
                                navigate_web_surface_tab(state, nav_path.clone(), tab_id, url, nav_ssh.clone(), None);
                            }
                        } else if evt.key() == Key::Escape {
                            state.with_mut_counted(|shell| {
                                shell.web_surface_set_address_draft(&nav_path, None);
                            });
                        }
                    }
                },
            }
            button {
                style: "{reload_style}",
                title: "History",
                onclick: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    let nav_profile = nav_profile.clone();
                    move |evt: MouseEvent| {
                        if web_nav_middle_click(&evt) {
                            return;
                        }
                        navigate_web_surface_tab(state, nav_path.clone(), active_tab_id, web_history_data_url(&nav_profile), nav_ssh.clone(), None);
                    }
                },
                onmouseup: {
                    let nav_path = nav_path.clone();
                    let nav_ssh = nav_ssh.clone();
                    let nav_profile = nav_profile.clone();
                    move |evt: MouseEvent| {
                        if !web_nav_middle_click(&evt) {
                            return;
                        }
                        evt.prevent_default();
                        evt.stop_propagation();
                        open_web_nav_target_in_new_tab(
                            state,
                            &nav_path,
                            active_tab_id,
                            web_history_data_url(&nav_profile),
                            nav_ssh.clone(),
                        );
                    }
                },
                "🕘"
            }
        }
        // ⛔ NO INLINE DROPDOWN. The results are the CENTRED PALETTE's
        // (`TopModal::CommandPalette`, mounted with the other over-viewport
        // modals), which is the owner requirement and DESIGN.md ▸ Search in
        // chrome both: the result surface wraps the field itself into one
        // continuous shell rather than hanging a popover under a small input.
        // A second list drawn here would be that popover, back beside the thing
        // that replaced it.
    }
}
/// The find bar's input. One bar at a time — only the ACTIVE session's surface
/// draws chrome — so a stable constant id is the whole addressing scheme, the
/// same shape `SEARCH_INPUT_ID` uses for the titlebar search.
const WEB_FIND_INPUT_ID: &str = "yggterm-web-find-input";
/// Put the keyboard in the find field, and claim the UI focus window so the
/// terminal's own reclaim loops do not yank it straight back out.
///
/// This BORROWS: `close_web_find` hands the keyboard back to the recorded
/// origin, which is what keeps Ctrl+F out of the five focus-theft classes this
/// product has already paid for.
///
/// The borrow is ORDERED (and thereby recorded) by
/// `web_find::borrow_focus_for_bar`. That call is what puts a `FocusMoved` in
/// the ledger the `web find` verb publishes — so the ledger describes moves
/// this function actually makes, and a path that opens a bar without moving the
/// keyboard cannot publish one.
fn focus_web_find_input() {
    let target = web_find::borrow_focus_for_bar();
    debug_assert_eq!(target, web_find::FindFocusTarget::FindInput);
    clear_sidebar_keyboard_owner();
    let _ = document::eval(&format!(
        r#"
        (() => {{
          const claim = () => {{
            try {{
              window.__yggtermUiFocusClaimUntilMs = Math.max(
                Number(window.__yggtermUiFocusClaimUntilMs || 0),
                Date.now() + 2200
              );
            }} catch (_error) {{}}
          }};
          const run = () => {{
            const input = document.getElementById({WEB_FIND_INPUT_ID:?});
            if (!input || typeof input.focus !== 'function') {{ return false; }}
            claim();
            input.focus({{ preventScroll: true }});
            if (typeof input.select === 'function') {{ input.select(); }}
            return document.activeElement === input;
          }};
          claim();
          if (run()) {{ return; }}
          window.requestAnimationFrame(run);
          window.setTimeout(run, 0);
          window.setTimeout(run, 32);
          window.setTimeout(run, 96);
        }})();
        "#
    ));
}
/// Give the keyboard back to whoever lent it when the bar closes.
///
/// The bar never DECIDES where focus belongs — `origin` was recorded at open
/// time and this only replays it. A terminal lender gets its input re-enabled
/// and refocused; a PAGE lender gets the toplevel's keyboard back through the
/// host focus verb; a chrome lender (the origin an agent-opened bar records)
/// gets the field blurred and nothing else, because nobody in particular lent
/// anything.
///
/// ⚠ **The page arm is not decoration.** Ctrl+F pressed on a focused page is
/// claimed at the GTK level and the relay grabs the keyboard for the shell so
/// the field — which lives in the shell's DOM — can take a keystroke at all.
/// Without the give-back, Escape would leave the user's page with no keyboard
/// until they clicked it: no PageUp, no PageDown, no typing into the form they
/// were filling. `web_find::give_back_moves_the_keyboard` is where that is
/// stated; this is where it is obeyed. The host verb refuses a surface that is
/// not being shown, so a bar closed over a backgrounded page costs nothing.
///
/// The target comes from `web_find::return_focus_to_lender`, which is also what
/// writes the give-back into the published ledger: the move this function makes
/// and the move the trace reports are produced by the same call, so a close
/// that skipped the give-back would show up as a MISSING entry rather than be
/// papered over by bookkeeping.
fn restore_focus_after_web_find(
    state: Signal<ShellState>,
    desktop: &dioxus::desktop::DesktopContext,
    session_path: &str,
    origin: web_find::FindFocusOrigin,
) {
    let target = web_find::return_focus_to_lender(&origin);
    let _ = document::eval(&format!(
        r#"
        (() => {{
          try {{
            const input = document.getElementById({WEB_FIND_INPUT_ID:?});
            if (input && typeof input.blur === 'function') {{ input.blur(); }}
            window.__yggtermUiFocusClaimUntilMs = 0;
          }} catch (_error) {{}}
        }})();
        "#
    ));
    if !web_find::give_back_moves_the_keyboard(&target) {
        return;
    }
    match target {
        web_find::FindFocusTarget::Terminal(session_path) => {
            refocus_terminal_session_input(&session_path);
        }
        web_find::FindFocusTarget::Page => {
            if let Ok((_, native_id)) = resolve_live_web_surface(&state, Some(session_path)) {
                desktop.web_surface_focus(native_id);
            }
        }
        // The borrow direction never reaches a give-back, and chrome is
        // filtered out above; both arms are here so a new target cannot be
        // added without deciding what closing means for it.
        web_find::FindFocusTarget::FindInput | web_find::FindFocusTarget::Chrome => {}
    }
}
/// THE one door to "open the find bar over whatever page owns the viewport".
///
/// **Two keyboards ask for this and they must reach the SAME decisions.** The
/// shell's own root `onkeydown` sees Ctrl+F only while the shell webview holds
/// focus; while a page holds it — the normal case while browsing, and the
/// reported bug ("no ctrl+F") — the key is claimed at the GTK level on the
/// TOPLEVEL WINDOW and relayed in through `keytip_apply_bridge_message`. Before this
/// function existed the refusals lived inline in the DOM handler's closure, so
/// the second door could only have got them by copying them, and a copied
/// refusal is a refusal that drifts.
///
/// Refuses, in order: no active session; no live overlay for it (no web surface
/// at all, or a stale one); a PICKER-phase surface, which has no page to
/// search. Records the LENDER — a typing-ready terminal underneath gets the
/// keyboard back on Escape, a page lender gets it back through the host focus
/// verb — and then borrows the keyboard into the field.
///
/// Idempotent, which is also what a browser does: Ctrl+F on an already-open bar
/// re-focuses and selects the field rather than forgetting who lent the
/// keyboard (`ShellState::open_web_find`). That is the ONLY way back into a bar
/// the user left by clicking into the page, because an unfocused bar claims no
/// keys at all — see `web_find::find_bar_blocks_terminal_input`.
///
/// Returns the session whose bar is now open and focused, or `None` if it
/// refused.
fn open_web_find_for_viewport(mut state: Signal<ShellState>) -> Option<String> {
    // The lender: whoever holds the keyboard right now. A terminal that is
    // typing-ready is the one that gets it back on Escape; a page lender is
    // given it back through the host focus verb when the bar closes.
    let session = state.with_mut_counted(|shell| {
        let session = shell.server.active_session_path()?.to_string();
        // A picker-phase surface has no page to search.
        let overlay = shell.web_surface_overlay_for_session(&session, current_millis())?;
        if overlay.picker_control_url.is_some() {
            return None;
        }
        let origin = if shell.terminal_input_override_active {
            web_find::FindFocusOrigin::Terminal(session.clone())
        } else {
            web_find::FindFocusOrigin::Page
        };
        shell.open_web_find(&session, origin).then_some(session)
    })?;
    focus_web_find_input();
    Some(session)
}
/// The bar's own close (Escape, ✕). Both halves live in
/// `close_web_find_everywhere`, which the agent verb's `--close` also calls —
/// one close, so "finish the search AND hand the keyboard back" cannot be got
/// half-right on one of the two paths.
fn close_web_find_bar(state: Signal<ShellState>, session_path: String) {
    let desktop = window();
    spawn(async move {
        let _ = close_web_find_everywhere(state, desktop, &session_path).await;
    });
}
/// Run one find step for the bar and let the render pick up the new count.
fn drive_web_find_bar(state: Signal<ShellState>, session_path: String, step: web_find::FindStep) {
    let desktop = window();
    spawn(async move {
        let _ = run_web_find_step(state, desktop, Some(&session_path), step).await;
    });
}
/// The find bar (Ctrl+F) over a web surface: field, `3/17`, prev, next, close.
///
/// **Where it sits and why it is not absolutely positioned.** It is a slim row
/// in normal flow directly above the page, with its contents pushed to the
/// right — the top-right anchor every browser puts a find bar at. An absolutely
/// positioned overlay would be INVISIBLE under legacy stacking, where a native
/// child webview paints above the shell's DOM; the flow-push is the idiom the
/// omnibox dropdown already uses, and it shrinks the `[data-ws-page]` rect so
/// the native surface follows it down. One mechanism, both stackings.
///
/// **Styling is the omnibox's, literally** — `web_chrome_input_style` and
/// `web_chrome_icon_button_style`, the same functions the address bar above it
/// wears. The only thing this bar varies is the pill's border on a no-match
/// query — a VALUE, not a key: `web_chrome_input_style` emits its key set from
/// a single unbranched format string, so the Dioxus property-by-property trap
/// (a key one branch emits and another drops stays applied forever) has no way
/// in.
#[component]
fn WebFindBar(
    state: Signal<ShellState>,
    session_path: String,
    find: WebFindBarView,
    foreground: String,
    background: String,
) -> Element {
    let has_matches = !find.no_matches && !find.query.is_empty();
    let border = if find.no_matches {
        WEB_CHROME_INPUT_BORDER_NO_MATCH
    } else {
        WEB_CHROME_INPUT_BORDER
    };
    // The pill, narrowed: a find field is not an address field, and letting it
    // grow to the full width would push the label and buttons off the right
    // edge it is supposed to be anchored to. The narrowing is the helper's
    // `flex` parameter, not an appended property, so the style attribute never
    // carries `flex` twice.
    let input_style =
        web_chrome_input_style(&foreground, false, border, WEB_CHROME_INPUT_FLEX_FIND);
    let step_style = web_chrome_icon_button_style(&foreground, has_matches);
    let close_style = web_chrome_icon_button_style(&foreground, true);
    let label_style = format!(
        "color:{foreground}; opacity:0.7; font-size:12px; font-variant-numeric:tabular-nums; \
         white-space:nowrap; flex:0 0 auto; padding:0 2px;",
    );
    rsx! {
        div {
            style: format!(
                "display:flex; align-items:center; justify-content:flex-end; gap:4px; \
                 padding:4px 10px; background:{background}; user-select:none; overflow:hidden; \
                 max-height:44px;",
            ),
            // Under glass the chrome DOM draws over pages but its INPUT region
            // is the shell's; declaring the bar a cover is what makes its
            // buttons clickable instead of falling through to the page.
            "data-covers-web-surface": "web-find",
            input {
                id: "{WEB_FIND_INPUT_ID}",
                "data-yggui-field": "pill",
                style: "{input_style}",
                value: "{find.query}",
                spellcheck: "false",
                autocomplete: "off",
                placeholder: "Find in page",
                onfocus: {
                    let session_path = session_path.clone();
                    move |_| {
                        state.with_mut_counted(|shell| shell.set_web_find_focus(&session_path, true));
                    }
                },
                onblur: {
                    let session_path = session_path.clone();
                    move |_| {
                        // The bar owns keys ONLY while its input is focused: the
                        // instant the user clicks away, the terminal beneath is
                        // typing-ready again and the bar claims nothing.
                        state.with_mut_counted(|shell| shell.set_web_find_focus(&session_path, false));
                    }
                },
                oninput: {
                    let session_path = session_path.clone();
                    move |evt: FormEvent| {
                        let value = evt.value();
                        let asked = state.with_mut_counted(|shell| {
                            shell.set_web_find_query(&session_path, value)
                        });
                        // Incremental: every keystroke re-searches from the top,
                        // which is what makes the count follow the query.
                        if asked.is_some() {
                            drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Search);
                        } else {
                            // An emptied field: finish the search so the page's
                            // highlights go with the text that made them.
                            drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Close);
                        }
                    }
                },
                onkeydown: {
                    let session_path = session_path.clone();
                    move |evt: KeyboardEvent| {
                        let shift = evt.modifiers().contains(Modifiers::SHIFT);
                        let key = match evt.key() {
                            Key::Enter if shift => web_find::FindKey::ShiftEnter,
                            Key::Enter => web_find::FindKey::Enter,
                            Key::Escape => web_find::FindKey::Escape,
                            Key::Character(text) => web_find::FindKey::Char(text),
                            other => web_find::FindKey::Other(format!("{other}")),
                        };
                        // THE router. Every key the bar sees goes through
                        // `web_find::route_key` — the same function the focus
                        // lock drives — so "the bar claims keys only while its
                        // input is focused" is one rule with one implementation,
                        // not a rule and a separate `match` that could drift
                        // from it.
                        let route = state
                            .peek()
                            .web_surfaces
                            .get(&session_path)
                            .and_then(|surface| surface.find.as_ref())
                            .map(|find| find.route_key(&key))
                            .unwrap_or(web_find::FindRoute::NotOurs);
                        match route {
                            web_find::FindRoute::Bar(web_find::FindKeyAction::Next) => {
                                evt.prevent_default();
                                drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Next);
                            }
                            web_find::FindRoute::Bar(web_find::FindKeyAction::Prev) => {
                                evt.prevent_default();
                                drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Prev);
                            }
                            web_find::FindRoute::Bar(web_find::FindKeyAction::Close) => {
                                evt.prevent_default();
                                close_web_find_bar(state, session_path.clone());
                            }
                            // A claimed character is the FIELD's to type — the
                            // verdict says the key is ours, and letting the
                            // input consume it natively is how it becomes text.
                            // `oninput` picks it up from there.
                            web_find::FindRoute::Bar(web_find::FindKeyAction::Type(_))
                            | web_find::FindRoute::NotOurs => {}
                        }
                    }
                },
            }
            span { style: "{label_style}", "{find.label}" }
            button {
                style: "{step_style}",
                title: "Previous match (Shift+Enter)",
                disabled: !has_matches,
                onclick: {
                    let session_path = session_path.clone();
                    move |_| drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Prev)
                },
                "↑"
            }
            button {
                style: "{step_style}",
                title: "Next match (Enter)",
                disabled: !has_matches,
                onclick: {
                    let session_path = session_path.clone();
                    move |_| drive_web_find_bar(state, session_path.clone(), web_find::FindStep::Next)
                },
                "↓"
            }
            button {
                style: "{close_style}",
                title: "Close find bar (Escape)",
                onclick: {
                    let session_path = session_path.clone();
                    move |_| close_web_find_bar(state, session_path.clone())
                },
                "✕"
            }
        }
    }
}
/// The TAB TREE: the active web surface's tabs and the user's virtual folders.
/// yggterm's own chrome (it owns the tabs) — and it IS the cwd tree's row
/// grammar, not a fourth tree beside it (user-reported 2026-07-30, twice).
///
/// Everything structural here is SHARED: rows are [`SessionStyleRow`]s indented
/// by `depth`, the disclosure glyph is [`RowDisclosureChevron`], the drop bands
/// come from [`row_drop_placement_for_offset`], and a drop is resolved by
/// [`yggui::reorder_row_tree`] — the same engine a contributed pane's
/// `list-row`s use. The rail's own hand-rolled folder tree used to live here,
/// and its drag could only ever RE-PARENT: it never re-indexed anything.
///
/// Folders come FIRST, then the loose tabs ([`web_tab_rail_rows`]).
///
/// Vertical-tabs mode IS this rail: the viewport's tab strip collapses and the
/// tabs live here. Turning the mode off retires the rail (see
/// `set_web_surface_vertical_tabs`), so tabs always have exactly one home.
#[component]
fn WebTabsRailBody(snapshot: SharedSnapshot, state: Signal<ShellState>) -> Element {
    let palette = snapshot.palette;
    let Some(overlay) = snapshot.active_web_surface_overlay.clone() else {
        return rsx! {
            RailHeader { title: "Tabs".to_string(), color: palette.text.to_string() }
            div {
                style: format!("font-size:11px; line-height:1.5; color:{}; padding:0 2px;", palette.muted),
                "No web surface is open in this session."
            }
        };
    };
    let session_path = snapshot.active_session_path.clone().unwrap_or_default();
    // The omnibox at the rail top (vertical-tabs mode) drives the SAME active
    // surface as the tab rows below it, so it needs the surface's egress target
    // exactly as the viewport nav bar does.
    let omni_ssh = state.with(|shell| shell.web_surface_session_ssh_target(&session_path));
    let omni_overlay = overlay.clone();
    let omni_session_path = session_path.clone();
    let omni_panel_bg = palette.panel.to_string();
    let omni_text = palette.text.to_string();
    let rename = snapshot.web_tab_rename.clone();
    // The surface's identity, from the ONE owner of it (the app tab). The rail
    // renders the active session, so this is that session's profile.
    let overlay_profile = snapshot.active_web_surface_profile.clone();

    // THE MODEL — one ordered list of TABS, a group's members one level under
    // their head, a collapsed group's members present but not visible.
    // `row_tree` is that same list as the shared reorder engine sees it and is
    // what a drop resolves against, which is why a folded-away tab survives a
    // reorder elsewhere in the rail.
    let rail_rows = web_tab_rail_rows(&overlay.tabs);
    let row_tree = web_tab_rail_row_tree(&rail_rows);
    let tabs: HashMap<u64, WebSurfaceOverlayTabView> = overlay
        .tabs
        .iter()
        .map(|tab| (tab.id, tab.clone()))
        .collect();

    // ⭐ ONE row renderer, and now literally one ARM. Under folders this was a
    // match with two arms that had drifted apart; a group is headed by a TAB, so
    // there is no second kind of row left to render. A head differs from any
    // other row by three optional slots — a chevron, a glyph and a count — and
    // by one extra verb.
    let row_view = {
        let session_path = session_path.clone();
        let rename = rename.clone();
        move |row: &WebTabRailRow| -> Element {
            let row_id = row.id();
            let depth = row.depth;
            let is_app_tab = row.row == WebTabMenuTarget::Tab(WEB_TAB_APP_TAB_ID);
            let (drop_edge, row_is_dragging) = state.with(|shell| {
                (
                    shell.web_tab_row_drop_edge(&row_id),
                    shell.web_tab_row_is_dragging(&row_id),
                )
            });
            let renaming = rename
                .as_ref()
                .filter(|(id, _)| id == &row_id)
                .map(|(_, draft)| draft.clone());

            // RENAMING replaces the row body in place — the cwd tree's own
            // rename shape, for a FOLDER and a TAB alike.
            if let Some(draft) = renaming {
                let (commit_path, blur_path) = (session_path.clone(), session_path.clone());
                let field_row = row_id.clone();
                return rsx! {
                    div {
                        key: "webrow-{row_id}",
                        "data-web-tab-row-id": "{row_id}",
                        style: format!(
                            "display:flex; align-items:center; gap:6px; padding:2px 0 2px {}px;",
                            8 + depth * 12,
                        ),
                        input {
                            "data-web-tab-row-rename": "{row_id}",
                            style: format!(
                                "flex:1 1 auto; min-width:0; padding:3px 6px; border-radius:6px; \
                                 border:1px solid {}; background:rgba(127,127,127,0.12); color:{}; \
                                 font-size:12px; font-weight:600; outline:none;",
                                palette.accent, palette.text,
                            ),
                            initial_value: "{draft}",
                            // Focus AND SELECT. A row born with a placeholder
                            // name ("New folder") must take the user's first
                            // keystroke as a REPLACEMENT, not append to it.
                            onmounted: move |evt| {
                                let field_row = field_row.clone();
                                async move {
                                    let _ = evt.set_focus(true).await;
                                    select_rename_field(&format!(
                                        "[data-web-tab-row-rename=\"{field_row}\"]"
                                    ));
                                }
                            },
                            onclick: move |evt: MouseEvent| evt.stop_propagation(),
                            onmousedown: move |evt: MouseEvent| evt.stop_propagation(),
                            oninput: move |evt: FormEvent| {
                                let value = evt.value();
                                state.with_mut_counted(|shell| shell.web_tab_set_rename_draft(value));
                            },
                            onkeydown: move |evt: KeyboardEvent| {
                                let commit_path = commit_path.clone();
                                match evt.key() {
                                    Key::Enter => state.with_mut_counted(|shell| shell.web_tab_commit_rename(&commit_path)),
                                    Key::Escape => state.with_mut_counted(|shell| shell.web_tab_cancel_rename()),
                                    _ => {}
                                }
                            },
                            onblur: move |_| {
                                let blur_path = blur_path.clone();
                                state.with_mut_counted(|shell| shell.web_tab_commit_rename(&blur_path));
                            },
                        }
                    }
                };
            }

            // The row's own vocabulary, resolved once so the rsx below has one
            // shape whether or not this row heads a group.
            let WebTabMenuTarget::Tab(tab_id) = row.row;
            let tab = tabs.get(&tab_id).cloned();
            let heads_group = row.heads_group;
            let group_size = tab.as_ref().map(|tab| tab.group_size).unwrap_or(0);
            let expanded = heads_group.then(|| tab.as_ref().is_none_or(|tab| !tab.group_collapsed));
            let loading = tab.as_ref().is_some_and(|tab| tab.loading);
            // BACKGROUND-only, and the view already resolved that — see
            // `WebSurfaceOverlayTabView::media_playing`. The rail asks
            // no question about it here, so the rail and the classic
            // strip cannot answer it differently.
            let media_playing = tab.as_ref().is_some_and(|tab| tab.media_playing);
            let (select_path, close_path) = (session_path.clone(), session_path.clone());
            let (add_path, chevron_path) = (session_path.clone(), session_path.clone());
            // The app tab's ✕ is shown only while it holds a saved page;
            // when closable it despawns like any tab (user request: first
            // tab must despawn, not navigate home to Brave).
            let app_tab_can_go_home = tab.as_ref().is_some_and(|tab| tab.holds_saved_page);
            let label = tab
                .as_ref()
                .map(|tab| tab.label.clone())
                .unwrap_or_default();
            let selected = tab.as_ref().is_some_and(|tab| tab.active);
            // How many rows this one heads — the folder header's count, on the
            // row that replaced it. `None` on a row that heads nothing, so an
            // ordinary tab spends no badge box on a "0".
            let badge = heads_group.then(|| group_size.to_string());
            // A tab's ONE leading mark is its activity dot; a head carries it
            // too, because a head is a real page that can be loading or making
            // noise. Two causes light it — the page is loading, or the engine
            // says this background tab is playing media — and each gets its own
            // attribute so a probe (and the falsifier screenshot's companion
            // read) can tell which.
            let dot = Some(rsx! {
                span {
                    "data-web-tab-loading": if loading { "true" } else { "false" },
                    "data-web-tab-media": if media_playing { "true" } else { "false" },
                    title: web_tab_activity_dot_title(loading, media_playing).unwrap_or_default(),
                    style: web_tab_activity_dot_style(loading, media_playing),
                }
            });
            // THE ROW'S LEADING MARK is the page's FAVICON, from the engine's
            // own database — every row wears its page's icon, a group's head
            // row included, which is exactly what the old folder glyph was
            // standing in for (user report: "there should not be a folder
            // icon in the row header"). `None` on a row the database has
            // served nothing for yet — never loaded, still loading, or an
            // ephemeral profile, which keeps no icons — and an ABSENT slot,
            // not an empty element: an empty element reserved a 20px icon box
            // plus its gap on every tab row in the rail (user report
            // 2026-07-31). The always-laid-out mark COLUMN in
            // `SessionStyleRow` keeps every title aligned either way, and the
            // loading/media dot rides it as before — a corner badge once an
            // icon shares the column, centered when it does not.
            let icon = tab
                .as_ref()
                .and_then(|tab| tab.favicon_png.as_deref())
                .map(|png| rsx! { WebTabFaviconIcon { png: png.to_vec() } });
            let expander = expanded.map(|expanded| rsx! {
                button {
                    "data-web-tab-group-expand": "{tab_id}",
                    style: row_disclosure_button_style(palette.muted),
                    title: if expanded { "Collapse group" } else { "Expand group" },
                    onmousedown: |evt: MouseEvent| evt.stop_propagation(),
                    onclick: move |evt: MouseEvent| {
                        evt.stop_propagation();
                        let path = chevron_path.clone();
                        state.with_mut_counted(|shell| shell.web_tab_toggle_group(&path, tab_id));
                    },
                    RowDisclosureChevron { expanded }
                }
            });
            // ⛔ AN ABSENT SLOT, not an empty element. A row with neither verb
            // must hand the shared row `None`: `Some(rsx!{})` draws nothing and
            // still reserves the box plus its gap on every row in the rail
            // (DESIGN.md, 2026-07-31).
            let shows_close = !is_app_tab || app_tab_can_go_home;
            let actions = (heads_group || shows_close).then(|| {
                rsx! {
                    // The head row's "+", which is what replaced the folder
                    // header's: it fills the GROUP, not the window, and it is
                    // typing-ready like every other "+".
                    if heads_group {
                        button {
                            "data-web-tab-group-add": "{tab_id}",
                            style: session_row_action_button_style(palette.text),
                            title: "New tab in this group",
                            onmousedown: |evt: MouseEvent| evt.stop_propagation(),
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                let path = add_path.clone();
                                open_web_surface_tab(
                                    state,
                                    &path,
                                    WebTabOpenRequest::blank_in_group(tab_id),
                                );
                            },
                            "+"
                        }
                    }
                    // The app tab's ✕ once QUIT ychrome (a Ctrl+C to the
                    // app) while this same row's menu refused to close it
                    // and said why — two affordances on one row
                    // disagreeing. So it lost the ✕ entirely, and the user
                    // then had a first tab with no close button at all
                    // (report + screenshot, 2026-08-01).
                    //
                    // Both are avoidable, because the app tab has TWO
                    // states and only one of them is "the app". While it
                    // shows a real page it is a real tab — it gets a ✕ that
                    // despawns the row (user request: first tab must close
                    // and despawn, not just navigate home to Brave). The
                    // surface keeps its home via the next heartbeat if
                    // needed; the closed entry goes to the undo stack.
                    // Quitting the app still lives where quitting an app
                    // lives.
                    if shows_close {
                        button {
                            "data-web-tab-close": "{tab_id}",
                            style: session_row_action_button_style(palette.text),
                            // ⛔ Says what it does NOT do. Closing a head must
                            // never read as closing the group: its members move up
                            // one level, exactly as Ungroup would leave them.
                            title: if heads_group {
                                "Close tab (its group's tabs move up one level)"
                            } else {
                                "Close tab"
                            },
                            onmousedown: |evt: MouseEvent| evt.stop_propagation(),
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                let close_path = close_path.clone();
                                state.with_mut_counted(|shell| {
                                    shell.web_surface_close_tab(&close_path, tab_id);
                                    shell.persist_web_tabs(&close_path, WebTabSave::TreeEdit);
                                });
                            },
                            "✕"
                        }
                    }
                }
            });
            let on_activate = EventHandler::new(move |_| {
                // A drag's own release is also a click: moving a
                // tab must not also switch to it.
                if state.with_mut_counted(|shell| shell.consume_suppressed_row_click()) {
                    return;
                }
                select_web_surface_tab(state, select_path.clone(), tab_id, WebTabSelect::User);
            });

            let menu_path = session_path.clone();
            let menu_target = row.row.clone();
            let rename_target = row_id.clone();
            let (down_row, move_row) = (row_id.clone(), row_id.clone());
            let (down_label, move_label) = (label.clone(), label.clone());
            // A SHUT group is what spring-load opens under a hovering drag.
            let row_collapsed = expanded == Some(false);
            let spring_path = session_path.clone();
            rsx! {
                div {
                    key: "webrow-{row_id}",
                    // The row's identity and state, in ONE vocabulary for both
                    // kinds — a probe asks the same questions of a folder and a
                    // tab. Drag state is read on this OUTER box so the drop line
                    // is not clipped by the row's own border radius, the same
                    // reason the contributed rail draws it here.
                    "data-web-tab-row-id": "{row_id}",
                    // ONE kind of row now. The attribute survives — probes and
                    // the falsifier screenshots key on it — and says whether this
                    // tab HEADS a group, which is the only distinction left.
                    "data-web-tab-row-kind": if heads_group { "group" } else { "tab" },
                    "data-web-tab-row-depth": "{depth}",
                    "data-web-tab-row-active": if selected { "true" } else { "false" },
                    "data-web-tab-row-expanded": match expanded {
                        Some(true) => "true",
                        Some(false) => "false",
                        None => "",
                    },
                    "data-web-tab-row-dragging": if row_is_dragging { "1" } else { "0" },
                    "data-web-tab-row-drop-edge": match drop_edge {
                        Some(DragDropPlacement::Before) => "before",
                        Some(DragDropPlacement::Into) => "into",
                        Some(DragDropPlacement::After) => "after",
                        None => "",
                    },
                    // The WHOLE `Option` goes in: a `.map(…).unwrap_or_default()`
                    // here emitted NOTHING when the drag left the row, and Dioxus
                    // never clears a property a later render omits — so the
                    // accent line stayed on every row the pointer had crossed.
                    style: app_pane_row_drop_line_style(drop_edge, palette.accent),
                    onmousedown: move |evt: MouseEvent| {
                        if evt.trigger_button() != Some(MouseButton::Primary) {
                            return;
                        }
                        // ARM only: a press that has not travelled is a click.
                        let pointer = evt.client_coordinates();
                        let (down_row, down_label) = (down_row.clone(), down_label.clone());
                        state.with_mut_counted(|shell| {
                            shell.arm_web_tab_row_drag(down_row, down_label, (pointer.x, pointer.y));
                        });
                    },
                    onmousemove: move |evt: MouseEvent| {
                        // No button held ⇒ a hover, not a drag.
                        if !evt.held_buttons().contains(MouseButton::Primary) {
                            return;
                        }
                        let pointer = evt.client_coordinates();
                        // Before / inside / after, from the ONE band rule.
                        // ⭐ EVERY row has an inside now, not only the ones that
                        // already head a group — dropping onto a plain tab is
                        // how a group is made at all.
                        let placement = row_drop_placement_for_offset(
                            evt.element_coordinates().y,
                            true,
                        );
                        let (move_row, spring_path) = (move_row.clone(), spring_path.clone());
                        state.with_mut_counted(|shell| {
                            if !shell.maybe_begin_web_tab_row_drag((pointer.x, pointer.y)) {
                                return;
                            }
                            // SPRING-LOAD, from the shared engine's dwell: rest
                            // on a shut group and it opens under the drag, so
                            // filing a tab two levels down is one gesture
                            // instead of drop-open-drag-again per level.
                            if let Some(sprung) = shell.hover_web_tab_row_drop(
                                &move_row,
                                &move_label,
                                placement,
                                row_collapsed,
                            )
                                && let Some(WebTabMenuTarget::Tab(head)) =
                                    web_tab_row_target(&sprung)
                            {
                                shell.web_tab_toggle_group(&spring_path, head);
                            }
                        });
                    },
                    SessionStyleRow {
                        density: SessionRowDensity::Rail,
                        depth,
                        selected,
                        dimmed: row_is_dragging,
                        text_color: palette.text.to_string(),
                        selected_bg: palette.accent_soft.to_string(),
                        label,
                        badge,
                        badge_color: Some(palette.muted.to_string()),
                        dot,
                        icon,
                        // The cwdtree's icon rule: muted at rest, text color on
                        // the selected row.
                        icon_color: Some(
                            if selected { palette.text } else { palette.muted }.to_string(),
                        ),
                        expander,
                        actions,
                        onclick: on_activate,
                        // Double-click renames, exactly as the cwd tree's rows
                        // do. The app tab is the app's, not the tree's.
                        ondoubleclick: (!is_app_tab).then(|| {
                            let rename_target = rename_target.clone();
                            EventHandler::new(move |_| {
                                let rename_target = rename_target.clone();
                                state.with_mut_counted(|shell| shell.web_tab_begin_rename(&rename_target));
                            })
                        }),
                        // Right-click raises the SHARED `ContextMenuOverlay`,
                        // through the ONE opener both row kinds have always
                        // gone through.
                        oncontextmenu: EventHandler::new(move |evt: MouseEvent| {
                            open_web_tab_menu_from_event(
                                state,
                                &menu_path,
                                menu_target.clone(),
                                WebSurfaceChromeAnchor::Rail,
                                evt,
                            );
                        }),
                    }
                }
            }
        }
    };

    let new_tab_path = session_path.clone();
    let end_drag_tree = row_tree.clone();
    rsx! {
        // The tab loading dot blinks with the same keyframes the live-session
        // status dot does. Declared here too so the rail carries its own signal
        // vocabulary even when the left sidebar (its other declaration site) is
        // not mounted.
        style { "{STATUS_DOT_BLINK_CSS}" }
        // The scrollbar contract: a scrollbar that LAYS OUT beside the rows
        // instead of overlaying them — the fix that gives the right-edge verbs
        // their clicks back. Declared with the rail so it can never be
        // unmounted separately from the scroller it styles.
        style { "{WEB_TABS_SCROLL_CSS}" }
        div {
            "data-web-tabs-rail": "1",
            style: "display:flex; flex-direction:column; gap:8px; min-height:0; flex:1 1 auto;",
            // A drag that ends anywhere in the rail commits against whatever row
            // it was last over; ending over nothing is a no-op, never a silent
            // move to the root.
            onmouseup: move |_| {
                let end_drag_tree = end_drag_tree.clone();
                state.with_mut_counted(|shell| shell.end_web_tab_row_drag(&end_drag_tree));
            },
            // A drag that wanders out of the rail forgets its TARGET, so a
            // release outside lands nothing — but the gesture itself is ended
            // by the shell root's release, never here. Ending it on leave is
            // not the same question: row-to-row movement inside the rail
            // produces leave events too, and abandoning on those made the drag
            // impossible to complete.
            onmouseleave: move |_| {
                state.with_mut_counted(|shell| shell.forget_row_drag_target());
            },
            // Zen-style omnibox: in vertical-tabs mode the address bar leaves the
            // viewport and lives here, at the top of the tab tree. Same component
            // and same active surface as the classic viewport nav bar.
            WebOmniboxBar {
                state,
                session_path: omni_session_path.clone(),
                // The id OWNER, not a second spelling of it.
                input_id: web_omnibox_input_id(true, ""),
                ssh_target: omni_ssh.clone(),
                overlay: omni_overlay.clone(),
                foreground: omni_text.clone(),
                background: omni_panel_bg.clone(),
                compact: true,
            }
            // The two verbs that act on the tab tree ride the heading itself —
            // an icon each, next to the noun they act on. The old full-width
            // "+ New tab" pill and "🗂 Folder" button spent a whole band of the
            // rail restating what a "+" says on its own.
            RailHeader {
                title: "Tabs".to_string(),
                color: palette.text.to_string(),
                actions: rsx! {
                    // The PROFILE pill's vertical-mode home (Phase 5): the
                    // classic strip is collapsed here, so the surface-level
                    // identity badge rides the rail header instead. It is a
                    // BUTTON now — anchor site 1 of the profile switcher (the
                    // user's recorded design: "profile switching as a dropdown
                    // on the vertical-tab rail"). Drawn for EVERY profile,
                    // including "default": a switcher you cannot reach because
                    // you are on the default identity is not a switcher.
                    if let Some(profile) = overlay_profile.clone() {
                        {
                            let badge_path = session_path.clone();
                            rsx! {
                                button {
                                    "data-ws-rail-profile-badge": "{profile}",
                                    "data-ws-profile-switch": "rail",
                                    title: "ychrome profile: {web_profile_display_name(&profile)} — click to switch",
                                    // One style expression, so every profile paints
                                    // the same key set (the Dioxus property-by-property
                                    // trap: a key one branch drops never clears).
                                    style: format!(
                                        "border:0; cursor:pointer; display:inline-flex; align-items:center; gap:4px; {}",
                                        session_row_badge_style(palette.accent),
                                    ),
                                    onclick: move |evt: MouseEvent| {
                                        open_web_profile_switcher_from_event(
                                            state,
                                            &badge_path,
                                            WebSurfaceChromeAnchor::Rail,
                                            evt,
                                        );
                                    },
                                    "{web_surface_profile_badge_label(&profile)} ⌄"
                                }
                            }
                        }
                    }
                    button {
                        "data-web-tab-new": "1",
                        style: format!(
                            "display:inline-flex; align-items:center; justify-content:center; width:22px; height:22px; \
                             border:0; border-radius:7px; background:{}; color:#fff; cursor:pointer; padding:0;",
                            palette.accent,
                        ),
                        title: "New tab",
                        onclick: move |_| {
                            open_web_surface_tab(state, &new_tab_path, WebTabOpenRequest::blank());
                        },
                        svg {
                            width: "12",
                            height: "12",
                            view_box: "0 0 12 12",
                            fill: "none",
                            path {
                                d: "M6 1.75V10.25",
                                stroke: "currentColor",
                                stroke_width: "1.8",
                                stroke_linecap: "round",
                            }
                            path {
                                d: "M1.75 6H10.25",
                                stroke: "currentColor",
                                stroke_width: "1.8",
                                stroke_linecap: "round",
                            }
                        }
                    }
                },
            }
            div {
                // gap:2px replaces the old per-row margins — list rhythm is the
                // LIST's job now that rows come from the shared engine.
                //
                // `data-web-tabs-scroll` is the SCROLLBAR CONTRACT'S hook (see
                // `WEB_TABS_SCROLL_CSS`): the rows' verbs — a group's ✕ and +
                // and the collapse chevron — sit at this container's right
                // edge, exactly where an overlay scrollbar paints and HIT-TESTS,
                // so with the default themed scrollbar the last ~14px of every
                // row swallowed clicks and the user could not close a row or
                // collapse a group at all (user report 2026-08-29). Styling the
                // scrollbar switches WebKit to a scrollbar that LAYS OUT —
                // content shrinks beside it, nothing sits under it, and every
                // verb is clickable all the way to the edge.
                "data-web-tabs-scroll": "1",
                style: "flex:1 1 auto; min-height:0; overflow-y:auto; overflow-x:hidden; padding-right:2px; \
                        display:flex; flex-direction:column; gap:2px;",
                // ONE list, in the model's order: folders (with their tabs)
                // above the loose tabs. There is no separate "Root" drop band
                // any more — it was a second drop path beside the reorder
                // engine. Un-filing is what dropping beside a ROOT row means,
                // and the app tab is always one, so the gesture is always there.
                for row in rail_rows.iter().filter(|row| row.visible) {
                    {row_view(row)}
                }
            }
        }
    }
}
/// A web tab's FAVICON, as a leading mark: the engine's PNG bytes in a
/// data URL, drawn at 16px in the row's 20px mark column. The rounded clip is
/// the browser favicon shape; `pointer-events:none` keeps the mark from
/// becoming a dead click-shadow over the row's own hit area — the row is the
/// click target, the mark is paint.
#[component]
fn WebTabFaviconIcon(png: Vec<u8>) -> Element {
    let data_url = format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png));
    rsx! {
        img {
            src: "{data_url}",
            width: "16",
            height: "16",
            style: "display:block; border-radius:4px; pointer-events:none;",
        }
    }
}
use emd_renderer::components::{
    AgentFindingSpec, DataGridSpec, EmdComponent, EvidenceSpec, EvidenceState, MetricSpec,
    MetricTone, PanelSpec, PlotMark, PlotSpec, QuerySpec, SparklineSpec, build_plot_scene,
    sparkline_paths,
};
/// Renders a schema an APP declared, with generic widgets. yggterm knows nothing
/// about what any of it means: a click just POSTs the widget's action id back to
/// the app's control endpoint, and whatever schema comes back is drawn next.
///
/// This component is the whole reason `RightPanelMode::Vault` and `::AppSidebar`
/// are gone. Adding an app-specific branch here defeats it.
// ===== markdown → native DOM (the document surface's body widget) =====
//
// Parsed with pulldown-cmark into a small block tree, rendered to VNodes.
// NEVER innerHTML: note-derived content must not reach the shell's JS context.
// Raw HTML blocks/spans in the source are dropped by construction; images
// render as their alt text + a link (asset transport is a follow-up; the GUI
// cannot assume a note's relative path is fetchable from its own host).
use emd_renderer::{MdBlock, MdInline, parse_markdown_blocks, top_level_block_ranges};
use yggui::prose::AnalyticalTextRole;

// The typed markdown model + parser live in `emd-renderer`, which is no longer
// in this tree: it moved to libyggterm on 2026-08-02, because the
// markdown-superset engine is a platform organ every pipeline app links —
// yedit and ztlkasten's document surfaces, breezed, charts-webapp — not a part of
// the terminal. Its spec moved with it and is the one owner of how it behaves:
// docs/spec-emd-renderer.md in github.com/yggdrasilhq/libyggterm. This file
// keeps only the Dioxus RENDER of those blocks; extracting the render into
// libyggterm as well is the spec's next seam.

/// The document reading typography (user spec 2026-07-18 "readability like
/// The New York Times", REFINED 2026-07-23: **sans-serif**, very legible,
/// generous spacing between paragraphs and around headings, no decoration the
/// markdown didn't ask for). ONE owner: the markdown reader root and the
/// block click-to-edit reader both use exactly this string. The stack is the
/// DESIGN.md "document reading font" entry — change it there first.
/// The document reader's body type — now a token, not a literal.
///
/// One owner for every reading surface's typography: `yggui::prose` in
/// libyggterm. It lived here for as long as yggterm was the only host, and
/// stopped being tenable the moment the Web View, the document reader and a
/// chat app each needed the same answer.
fn document_reading_typography() -> String {
    ProseTokens::document().root_style()
}

/// A markdown image `src` as something a webview will actually load.
///
/// An absolute path is a local file the agent pasted; anything already carrying
/// a scheme is left alone, because rewriting `https://` to `file://` would turn
/// a working image into a broken one.
fn preview_image_file_url(src: &str) -> String {
    let trimmed = src.trim();
    if trimmed.starts_with('/') {
        format!("file://{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// The plain text under a run of inline nodes — for an `alt`, which is an
/// attribute and cannot hold markup.
fn md_inline_plain_text(items: &[MdInline]) -> String {
    let mut out = String::new();
    for item in items {
        match item {
            MdInline::Text(text) => out.push_str(text),
            MdInline::Code(code) => out.push_str(code),
            MdInline::Strong(children)
            | MdInline::Emphasis(children)
            | MdInline::Strikethrough(children) => out.push_str(&md_inline_plain_text(children)),
            MdInline::Link { children, .. } => out.push_str(&md_inline_plain_text(children)),
            MdInline::Image { alt, .. } => out.push_str(&md_inline_plain_text(alt)),
            MdInline::HardBreak => out.push(' '),
        }
    }
    out
}

fn md_inline_nodes(items: &[MdInline], prose: &ProseTokens, ink: &ProseInk) -> Element {
    let code_style = prose.inline_code_style(ink);
    let link_style = prose.link_style(ink);
    let image_frame_style = prose.image_frame_style();
    let image_style = prose.image_style();
    rsx! {
        for (index, item) in items.iter().enumerate() {
            match item {
                MdInline::Text(text) => rsx! {
                    span { key: "t{index}", {md_text_with_inline_images(text, prose, ink)} }
                },
                MdInline::Code(code) => rsx! { code { key: "c{index}", style: "{code_style}", "{code}" } },
                MdInline::Strong(children) => rsx! { b { key: "b{index}", {md_inline_nodes(children, prose, ink)} } },
                MdInline::Emphasis(children) => rsx! { i { key: "i{index}", {md_inline_nodes(children, prose, ink)} } },
                MdInline::Strikethrough(children) => rsx! { s { key: "s{index}", {md_inline_nodes(children, prose, ink)} } },
                MdInline::Link { href, children } => rsx! {
                    a {
                        key: "a{index}",
                        style: "{link_style}",
                        title: "{href}",
                        href: "{href}",
                        prevent_default: "onclick",
                        {md_inline_nodes(children, prose, ink)}
                    }
                },
                // An image is DISPLAYED, not linked. This is the one place a
                // transcript full of pasted screenshots differs from a document,
                // and it is why `MdInline::Image` is a typed node in
                // emd-renderer rather than a 🖼 glyph plus a link.
                MdInline::Image { src, alt } => {
                    let file_url = preview_image_file_url(src);
                    let alt_text = md_inline_plain_text(alt);
                    rsx! {
                        span {
                            key: "img{index}",
                            style: "{image_frame_style}",
                            img {
                                src: "{file_url}",
                                alt: "{alt_text}",
                                style: "{image_style}",
                            }
                        }
                    }
                }
                MdInline::HardBreak => rsx! { br { key: "br{index}" } },
            }
        }
    }
}

/// Plain text, with any bare image PATH in it drawn as the image.
///
/// A markdown `![alt](src)` already arrives as `MdInline::Image`. This is the
/// other case, and on this product it is the common one: an agent pastes
/// `/home/user/.yggterm/clipboard/clipboard-….png` as ordinary prose, because
/// that is what a clipboard capture IS — a path someone typed. It rendered as a
/// 90-character filename, which is the least useful representation of a
/// screenshot available.
///
/// ⚠ Only ABSOLUTE paths with an image extension, and the surrounding text is
/// preserved rather than swallowed: a sentence that happens to mention a `.png`
/// still reads as a sentence, with the picture under it.
fn md_text_with_inline_images(text: &str, prose: &ProseTokens, ink: &ProseInk) -> Element {
    let mut segments: Vec<(String, Option<String>)> = Vec::new();
    let mut pending = String::new();
    for token in text.split_inclusive(char::is_whitespace) {
        let cleaned = token
            .trim()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']'));
        if cleaned.starts_with('/') && looks_like_image_path(cleaned) {
            segments.push((std::mem::take(&mut pending), Some(cleaned.to_string())));
            continue;
        }
        pending.push_str(token);
    }
    if segments.is_empty() {
        return rsx! { "{text}" };
    }
    if !pending.is_empty() {
        segments.push((std::mem::take(&mut pending), None));
    }
    let image_style = prose.image_style();
    let frame_style = prose.image_frame_style();
    // The caption is the PATH, and a path is code — so it wears the type
    // system's inline-code treatment rather than a face and a size spelled
    // here. `the_markdown_adapter_owns_no_typography_of_its_own` enforces that,
    // and it caught this on the first run.
    let caption_style = format!(
        "{} display:inline-block; margin-top:4px; max-width:100%; overflow:hidden; \
         text-overflow:ellipsis; white-space:nowrap; vertical-align:top;",
        prose.inline_code_style(ink),
    );
    rsx! {
        for (segment_index, (lead, path)) in segments.into_iter().enumerate() {
            span {
                key: "seg{segment_index}",
                if !lead.is_empty() {
                    "{lead}"
                }
                if let Some(path) = path {
                    span {
                        style: "{frame_style}",
                        img {
                            src: "{preview_image_file_url(&path)}",
                            alt: "{path}",
                            title: "{path}",
                            style: "{image_style} cursor:zoom-in;",
                            "data-preview-image-path": "{path}",
                        }
                        span { style: "{caption_style}", "{path}" }
                    }
                }
            }
        }
    }
}

fn evidence_state_label(state: EvidenceState) -> &'static str {
    match state {
        EvidenceState::Observed => "observed",
        EvidenceState::Collecting => "collecting",
        EvidenceState::Silent => "silent",
        EvidenceState::Unavailable => "unavailable",
        EvidenceState::Stale => "stale",
        EvidenceState::Uninstrumented => "uninstrumented",
    }
}

fn evidence_badge_style(state: EvidenceState, prose: &ProseTokens, ink: &ProseInk) -> String {
    let color = match state {
        EvidenceState::Observed => "#009E73",
        EvidenceState::Collecting => ink.accent.as_str(),
        EvidenceState::Silent | EvidenceState::Stale => "#E69F00",
        EvidenceState::Unavailable => "#D55E00",
        EvidenceState::Uninstrumented => ink.muted.as_str(),
    };
    format!(
        "display:inline-flex; align-items:center; border:1px solid {color}; border-radius:999px; \
         padding:2px 7px; color:{color}; text-transform:uppercase; {}",
        prose.analytical_text_style(AnalyticalTextRole::Badge),
    )
}

fn evidence_footer(evidence: &EvidenceSpec, prose: &ProseTokens, ink: &ProseInk) -> Element {
    let badge_style = evidence_badge_style(evidence.state, prose, ink);
    let label = evidence_state_label(evidence.state);
    rsx! {
        div {
            style: format!(
                "display:flex; flex-wrap:wrap; align-items:center; gap:5px 10px; \
                 border-top:1px solid {}; margin-top:12px; padding-top:9px; \
                 color:{}; {}",
                ink.hairline,
                ink.muted,
                prose.analytical_text_style(AnalyticalTextRole::Evidence),
            ),
            span { style: "{badge_style}", "{label}" }
            span { title: "Question", "{evidence.question}" }
            span { title: "Window", "{evidence.window}" }
            span { title: "Freshness", "↻ {evidence.freshness}" }
            span { title: "Units", "{evidence.units}" }
            span { title: "Source", "source: {evidence.source}" }
            span { title: "Reproduce", "reproduce: {evidence.reproduction}" }
        }
    }
}

fn emd_plot_node(spec: &PlotSpec, prose: &ProseTokens, ink: &ProseInk, index: usize) -> Element {
    let scene = build_plot_scene(spec);
    let plot_style = format!(
        "border:1px solid {}; border-radius:12px; padding:14px 15px 11px; \
         margin:14px 0; background:color-mix(in srgb, {} 3%, transparent); \
         min-width:0; overflow:hidden;",
        ink.hairline, ink.ink,
    );
    let title_style = format!(
        "color:{}; {}",
        ink.ink,
        prose.analytical_text_style(AnalyticalTextRole::Title),
    );
    let subtitle_style = format!(
        "color:{}; margin-top:2px; {}",
        ink.muted,
        prose.analytical_text_style(AnalyticalTextRole::Subtitle),
    );
    let axis_text = format!(
        "{} fill:{};",
        prose.analytical_text_style(AnalyticalTextRole::Axis),
        ink.muted,
    );
    match scene {
        Err(message) => rsx! {
            div {
                key: "plot-error{index}",
                style: "{plot_style}",
                div { style: "{title_style}", "{spec.title}" }
                div {
                    style: "{subtitle_style}",
                    if spec.evidence.state == EvidenceState::Collecting {
                        "Collecting observations…"
                    } else {
                        "{message}"
                    }
                }
                {evidence_footer(&spec.evidence, prose, ink)}
            }
        },
        Ok(scene) => {
            let view_box = format!("0 0 {} {}", scene.width, scene.height);
            let grid_stroke = ink.hairline.clone();
            let legend_style = format!(
                "display:flex; flex-wrap:wrap; gap:6px 14px; margin:5px 0 1px; \
                 color:{}; {}",
                ink.muted,
                prose.analytical_text_style(AnalyticalTextRole::Legend),
            );
            rsx! {
                div {
                    key: "plot{index}",
                    style: "{plot_style}",
                    div {
                        style: "display:flex; justify-content:space-between; align-items:flex-start; gap:12px;",
                        div {
                            div { style: "{title_style}", "{spec.title}" }
                            if let Some(subtitle) = &spec.subtitle {
                                div { style: "{subtitle_style}", "{subtitle}" }
                            }
                        }
                        span {
                            style: evidence_badge_style(spec.evidence.state, prose, ink),
                            "{evidence_state_label(spec.evidence.state)}"
                        }
                    }
                    if spec.legend && scene.series.len() > 1 {
                        div {
                            style: "{legend_style}",
                            for series in &scene.series {
                                span {
                                    style: "display:inline-flex; align-items:center; gap:5px;",
                                    span {
                                        style: format!(
                                            "display:inline-block; width:9px; height:9px; border-radius:50%; background:{};",
                                            series.color,
                                        ),
                                    }
                                    "{series.name}"
                                }
                            }
                        }
                    }
                    svg {
                        view_box: "{view_box}",
                        preserve_aspect_ratio: "xMidYMid meet",
                        role: "img",
                        style: "display:block; width:100%; height:auto; overflow:visible; margin-top:5px;",
                        title { "{spec.title}: {spec.evidence.question}" }
                        for tick in &scene.y_ticks {
                            line {
                                x1: "{scene.left}", y1: "{tick.position}",
                                x2: "{scene.right}", y2: "{tick.position}",
                                stroke: "{grid_stroke}", stroke_width: "1",
                            }
                            text {
                                x: "{scene.left - 10.0}", y: "{tick.position + 4.0}",
                                text_anchor: "end", style: "{axis_text}", "{tick.label}"
                            }
                        }
                        for tick in &scene.x_ticks {
                            text {
                                x: "{tick.position}", y: "{scene.bottom + 24.0}",
                                text_anchor: "middle", style: "{axis_text}", "{tick.label}"
                            }
                        }
                        line {
                            x1: "{scene.left}", y1: "{scene.bottom}",
                            x2: "{scene.right}", y2: "{scene.bottom}",
                            stroke: "{ink.muted}", stroke_width: "1",
                        }
                        for series in &scene.series {
                            for path_data in &series.area_paths {
                                path {
                                    d: "{path_data}", fill: "{series.color}", fill_opacity: "0.14",
                                    stroke: "none",
                                }
                            }
                            if spec.mark != PlotMark::Bar && spec.mark != PlotMark::Point {
                                for path_data in &series.line_paths {
                                    path {
                                        d: "{path_data}", fill: "none", stroke: "{series.color}",
                                        stroke_width: "2.4", stroke_linecap: "round", stroke_linejoin: "round",
                                        vector_effect: "non-scaling-stroke",
                                    }
                                }
                            }
                            for bar in &series.bars {
                                rect {
                                    x: "{bar.x}", y: "{bar.y}", width: "{bar.width}", height: "{bar.height}",
                                    rx: "2", fill: "{series.color}", fill_opacity: "0.86",
                                    title { "{bar.tooltip}" }
                                }
                            }
                            for point in &series.points {
                                circle {
                                    cx: "{point.x}", cy: "{point.y}", r: "3.6",
                                    fill: "{series.color}", stroke: "{ink.code_surface}", stroke_width: "1.4",
                                    style: "transition:r 100ms ease; cursor:crosshair;",
                                    title { "{point.tooltip}" }
                                }
                            }
                        }
                    }
                    {evidence_footer(&spec.evidence, prose, ink)}
                }
            }
        }
    }
}

fn emd_sparkline_node(
    spec: &SparklineSpec,
    prose: &ProseTokens,
    ink: &ProseInk,
    index: usize,
) -> Element {
    let paths = sparkline_paths(&spec.values, 220.0, 42.0);
    let color = spec.color.as_deref().unwrap_or("#0072B2");
    rsx! {
        div {
            key: "spark{index}",
            style: format!(
                "display:grid; grid-template-columns:minmax(90px,auto) minmax(120px,1fr) auto; \
                 align-items:center; gap:12px; border-top:1px solid {}; padding:9px 0; min-width:0;",
                ink.hairline,
            ),
            div {
                div { style: format!("color:{}; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::Label)), "{spec.label}" }
                if let Some(delta) = &spec.delta {
                    div { style: format!("color:{}; {}", ink.muted, prose.analytical_text_style(AnalyticalTextRole::Caption)), "{delta}" }
                }
            }
            svg {
                view_box: "0 0 220 42", preserve_aspect_ratio: "none",
                style: "display:block; width:100%; height:42px; overflow:visible;",
                title { "{spec.evidence.question}" }
                for path_data in paths {
                    path {
                        d: "{path_data}", fill: "none", stroke: "{color}", stroke_width: "2.2",
                        stroke_linecap: "round", stroke_linejoin: "round", vector_effect: "non-scaling-stroke",
                    }
                }
            }
            div {
                style: format!("color:{}; text-align:right; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::ExactValue)),
                "{spec.value.as_deref().unwrap_or(evidence_state_label(spec.evidence.state))}"
            }
        }
    }
}

fn emd_metric_node(
    spec: &MetricSpec,
    prose: &ProseTokens,
    ink: &ProseInk,
    index: usize,
) -> Element {
    let tone = match spec.tone {
        MetricTone::Neutral => ink.accent.as_str(),
        MetricTone::Good => "#009E73",
        MetricTone::Warning => "#E69F00",
        MetricTone::Critical => "#D55E00",
    };
    rsx! {
        div {
            key: "metric{index}",
            style: format!(
                "border:1px solid {}; border-top:2px solid {}; border-radius:10px; padding:12px 13px; min-width:0;",
                ink.hairline, tone,
            ),
            div { style: format!("color:{}; {}", ink.muted, prose.analytical_text_style(AnalyticalTextRole::Evidence)), "{spec.label}" }
            div { style: format!("color:{}; margin-top:4px; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::MetricValue)), "{spec.value}" }
            if let Some(delta) = &spec.delta {
                div { style: format!("color:{tone}; margin-top:3px; {}", prose.analytical_text_style(AnalyticalTextRole::Evidence)), "{delta}" }
            }
            if let Some(detail) = &spec.detail {
                div { style: format!("color:{}; margin-top:6px; {}", ink.muted, prose.analytical_text_style(AnalyticalTextRole::Caption)), "{detail}" }
            }
        }
    }
}

fn emd_query_node(spec: &QuerySpec, prose: &ProseTokens, ink: &ProseInk, index: usize) -> Element {
    rsx! {
        div {
            key: "query{index}",
            style: format!("border:1px solid {}; border-radius:10px; overflow:hidden; min-width:0;", ink.hairline),
            div {
                style: format!(
                    "display:flex; justify-content:space-between; align-items:center; gap:10px; \
                     border-bottom:1px solid {}; padding:8px 10px; color:{}; {}",
                    ink.hairline,
                    ink.ink,
                    prose.analytical_text_style(AnalyticalTextRole::QueryHeader),
                ),
                span { "{spec.title}" }
                span { style: format!("color:{}; {}", ink.muted, prose.analytical_text_style(AnalyticalTextRole::MonoLabel)), "{spec.language}" }
            }
            pre {
                style: format!(
                    "margin:0; padding:12px; overflow:auto; background:{}; color:{}; \
                     min-height:72px; {}",
                    ink.code_surface,
                    ink.ink,
                    prose.analytical_text_style(AnalyticalTextRole::MonoBody),
                ),
                "{spec.source}"
            }
            if let Some(status) = &spec.status {
                div { style: format!("border-top:1px solid {}; padding:6px 10px; color:{}; {}", ink.hairline, ink.muted, prose.analytical_text_style(AnalyticalTextRole::Caption)), "{status}" }
            }
        }
    }
}

fn emd_data_grid_node(
    spec: &DataGridSpec,
    prose: &ProseTokens,
    ink: &ProseInk,
    index: usize,
) -> Element {
    let pad = if spec.compact { "6px 9px" } else { "9px 11px" };
    rsx! {
        div {
            key: "data{index}",
            style: "overflow:auto; min-width:0;",
            div { style: format!("color:{}; margin-bottom:7px; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::Label)), "{spec.title}" }
            table {
                style: format!("width:100%; border-collapse:collapse; {}", prose.analytical_text_style(AnalyticalTextRole::DataTable)),
                thead {
                    tr {
                        for (column_index, column) in spec.columns.iter().enumerate() {
                            th {
                                key: "dc{column_index}",
                                style: format!(
                                    "padding:{pad}; border-bottom:2px solid {}; color:{}; text-align:left; \
                                     white-space:nowrap; {}",
                                    ink.hairline,
                                    ink.ink,
                                    prose.analytical_text_style(AnalyticalTextRole::DataHeader),
                                ),
                                "{column}"
                            }
                        }
                    }
                }
                tbody {
                    for (row_index, row) in spec.rows.iter().enumerate() {
                        tr {
                            key: "dr{row_index}",
                            for (cell_index, cell) in row.iter().enumerate() {
                                td {
                                    key: "dd{cell_index}",
                                    style: format!(
                                        "padding:{pad}; border-bottom:1px solid {}; color:{}; \
                                         white-space:nowrap; {}",
                                        ink.hairline,
                                        ink.ink,
                                        prose.analytical_text_style(AnalyticalTextRole::MonoBody),
                                    ),
                                    "{cell}"
                                }
                            }
                        }
                    }
                }
            }
            {evidence_footer(&spec.evidence, prose, ink)}
        }
    }
}

fn emd_agent_finding_node(
    spec: &AgentFindingSpec,
    prose: &ProseTokens,
    ink: &ProseInk,
    index: usize,
) -> Element {
    rsx! {
        div {
            key: "agent{index}",
            style: format!(
                "border:1px solid {}; border-radius:11px; padding:13px 14px; \
                 background:color-mix(in srgb, {} 5%, transparent); min-width:0;",
                ink.hairline, ink.accent,
            ),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                span { style: format!("color:{}; text-transform:uppercase; {}", ink.accent, prose.analytical_text_style(AnalyticalTextRole::Eyebrow)), "Agent analysis" }
                if let Some(status) = &spec.status {
                    span { style: evidence_badge_style(spec.evidence.state, prose, ink), "{status}" }
                }
            }
            div { style: format!("color:{}; margin-top:5px; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::CardTitle)), "{spec.title}" }
            div { style: format!("color:{}; margin-top:7px; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::Body)), "{spec.summary}" }
            if !spec.findings.is_empty() {
                ul { style: format!("color:{}; margin:9px 0 0; padding-left:19px; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::CompactBody)),
                    for finding in &spec.findings { li { style: "margin:4px 0;", "{finding}" } }
                }
            }
            if let Some(question) = &spec.next_question {
                div { style: format!("border-left:1px solid {}; margin-top:10px; padding-left:9px; color:{}; {}", ink.ink, ink.muted, prose.analytical_text_style(AnalyticalTextRole::CompactBody)), em { "{question}" } }
            }
            {evidence_footer(&spec.evidence, prose, ink)}
        }
    }
}

fn emd_panel_node(spec: &PanelSpec, prose: &ProseTokens, ink: &ProseInk, index: usize) -> Element {
    rsx! {
        section {
            key: "panel{index}",
            style: format!("border:1px solid {}; border-radius:12px; padding:13px 14px; min-width:0;", ink.hairline),
            div {
                style: "display:flex; justify-content:space-between; align-items:flex-start; gap:10px; margin-bottom:10px;",
                div {
                    div { style: format!("color:{}; {}", ink.ink, prose.analytical_text_style(AnalyticalTextRole::PanelTitle)), "{spec.title}" }
                    if let Some(subtitle) = &spec.subtitle {
                        div { style: format!("color:{}; margin-top:2px; {}", ink.muted, prose.analytical_text_style(AnalyticalTextRole::Caption)), "{subtitle}" }
                    }
                }
                if !spec.controls.is_empty() {
                    div { style: "display:flex; flex-wrap:wrap; gap:5px; justify-content:flex-end;",
                        for control in &spec.controls {
                            button {
                                disabled: true,
                                title: "Declarative EMD action; this document surface has no action route",
                                "data-emd-action": control.action.as_deref().unwrap_or(""),
                                style: format!(
                                    "border:1px solid {}; border-radius:7px; padding:4px 8px; \
                                     background:{}; color:{}; opacity:0.86; {}",
                                    if control.primary { ink.accent.as_str() } else { ink.hairline.as_str() },
                                    if control.primary { ink.accent.as_str() } else { "transparent" },
                                    if control.primary { "#ffffff" } else { ink.ink.as_str() },
                                    prose.analytical_text_style(AnalyticalTextRole::Control),
                                ),
                                "{control.label}"
                            }
                        }
                    }
                }
            }
            div { style: "display:flex; flex-direction:column; gap:10px; min-width:0;",
                for (child_index, child) in spec.children.iter().enumerate() {
                    {emd_component_node(child, prose, ink, child_index)}
                }
            }
        }
    }
}

fn emd_component_node(
    component: &EmdComponent,
    prose: &ProseTokens,
    ink: &ProseInk,
    index: usize,
) -> Element {
    match component {
        EmdComponent::Grid(spec) => rsx! {
            div {
                key: "grid{index}",
                style: format!(
                    "display:grid; grid-template-columns:repeat({}, minmax(0, 1fr)); gap:{}px; \
                     margin:14px 0; align-items:start; min-width:0;",
                    spec.columns, spec.gap_px,
                ),
                for (child_index, child) in spec.children.iter().enumerate() {
                    {emd_component_node(child, prose, ink, child_index)}
                }
            }
        },
        EmdComponent::Panel(spec) => emd_panel_node(spec, prose, ink, index),
        EmdComponent::Plot(spec) => emd_plot_node(spec, prose, ink, index),
        EmdComponent::Sparkline(spec) => emd_sparkline_node(spec, prose, ink, index),
        EmdComponent::Metric(spec) => emd_metric_node(spec, prose, ink, index),
        EmdComponent::Query(spec) => emd_query_node(spec, prose, ink, index),
        EmdComponent::DataGrid(spec) => emd_data_grid_node(spec, prose, ink, index),
        EmdComponent::AgentFinding(spec) => emd_agent_finding_node(spec, prose, ink, index),
    }
}

fn md_block_node(block: &MdBlock, prose: &ProseTokens, ink: &ProseInk, index: usize) -> Element {
    match block {
        MdBlock::Heading { level, children } => {
            let style = prose.heading_style(*level, ink);
            rsx! { div { key: "h{index}", style: "{style}", {md_inline_nodes(children, prose, ink)} } }
        }
        MdBlock::Paragraph(children) => rsx! {
            p {
                key: "p{index}",
                style: prose.paragraph_style(ink),
                {md_inline_nodes(children, prose, ink)}
            }
        },
        MdBlock::CodeBlock(code) => rsx! {
            pre {
                key: "pre{index}",
                style: prose.code_block_style(ink),
                "{code}"
            }
        },
        MdBlock::Component(document) => emd_component_node(&document.component, prose, ink, index),
        MdBlock::ComponentError { message, source } => rsx! {
            div {
                key: "emd-error{index}",
                role: "alert",
                style: format!(
                    "border:1px solid #D55E00; border-left:3px solid #D55E00; border-radius:8px; \
                     padding:10px 12px; margin:{}px 0; color:{}; background:{};",
                    prose.block_gap_px, ink.ink, ink.code_surface,
                ),
                div { style: format!("color:#D55E00; {}", prose.analytical_text_style(AnalyticalTextRole::ErrorTitle)), "Extended component could not render" }
                div { style: format!("margin-top:4px; {}", prose.analytical_text_style(AnalyticalTextRole::Subtitle)), "{message}" }
                pre { style: format!("overflow:auto; margin:8px 0 0; {}", prose.analytical_text_style(AnalyticalTextRole::MonoBody)), "{source}" }
            }
        },
        MdBlock::BlockQuote(body) => rsx! {
            div {
                key: "q{index}",
                style: prose.blockquote_style(ink),
                for (child_index, child) in body.iter().enumerate() {
                    {md_block_node(child, prose, ink, child_index)}
                }
            }
        },
        MdBlock::List { ordered, items } => {
            let item_style = prose.list_item_style();
            let list_body = rsx! {
                for (item_index, item) in items.iter().enumerate() {
                    li {
                        key: "li{item_index}",
                        style: "{item_style}",
                        for (child_index, child) in item.iter().enumerate() {
                            {md_block_node(child, prose, ink, child_index)}
                        }
                    }
                }
            };
            if *ordered {
                rsx! { ol { key: "ol{index}", style: prose.list_style(ink), {list_body} } }
            } else {
                rsx! { ul { key: "ul{index}", style: prose.list_style(ink), {list_body} } }
            }
        }
        MdBlock::Table { header, rows } => {
            let cell_style = prose.table_cell_style(ink);
            let head_style = prose.table_head_cell_style(ink);
            rsx! {
                // Wide tables scroll inside their own container; the document
                // never scrolls horizontally (the triage-board acceptance rule).
                div {
                    key: "tw{index}",
                    style: prose.table_wrap_style(),
                    table {
                        style: prose.table_style(),
                        if !header.is_empty() {
                            thead {
                                tr {
                                    for (cell_index, cell) in header.iter().enumerate() {
                                        th { key: "th{cell_index}", style: "{head_style}", {md_inline_nodes(cell, prose, ink)} }
                                    }
                                }
                            }
                        }
                        tbody {
                            for (row_index, row) in rows.iter().enumerate() {
                                tr {
                                    key: "tr{row_index}",
                                    for (cell_index, cell) in row.iter().enumerate() {
                                        td { key: "td{cell_index}", style: "{cell_style}", {md_inline_nodes(cell, prose, ink)} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        MdBlock::Rule => rsx! {
            div { key: "hr{index}", style: prose.rule_style(ink) }
        },
    }
}

/// Parse + render a markdown source onto ONE of the three reading surfaces.
///
/// The surface is named by the caller and never inferred. It used to be a
/// `compact: bool`, which conflated two surfaces that only look alike: the
/// 300px rail pane, which must keep its caller's interface size, and the Web
/// View's transcript, which must keep the TURN's reading size. Sharing one flag
/// meant the transcript silently inherited the rail's leading — every answer on
/// the reading surface drew at line-height 1.55 while the turn around it, and
/// the token set, said 1.72. Three surfaces, three names, no guessing.
fn markdown_widget_body(source: &str, palette: &DocTheme, prose: ProseTokens) -> Element {
    let blocks = parse_markdown_blocks(source);
    let ink = palette.prose_ink();
    rsx! {
        div {
            style: prose.root_style(),
            for (index, block) in blocks.iter().enumerate() {
                {md_block_node(block, &prose, &ink, index)}
            }
        }
    }
}

/// The pure-Markdown reader with BLOCK CLICK-TO-EDIT ([[campaign-libyggterm]]
/// Phase 4, Typora-lite — full WYSIWYG is out of scope, settled): click a
/// block and exactly that block's source swaps in as a mini-editor; commit on
/// Ctrl+Enter or blur splices it back and rides the draft channel to the app
/// (so the sqlite draft row updates and the schema refetch repaints); Esc
/// cancels. If the fold's block count and the offset-iter's range count ever
/// disagree, editing silently disables — a wrong splice is worse than none.
#[component]
fn EditableMarkdownBody(
    source: String,
    doc: DocTheme,
    state: Signal<ShellState>,
    session_path: String,
    pane_id: String,
) -> Element {
    let blocks = parse_markdown_blocks(&source);
    let ranges = top_level_block_ranges(&source);
    let editable = blocks.len() == ranges.len();
    let mut editing = use_signal(|| None::<usize>);
    let mut edit_text = use_signal(String::new);
    // A schema change (new source) invalidates any open block index.
    let mut editing_source_key = use_signal(String::new);
    if *editing_source_key.read() != source {
        editing_source_key.set(source.clone());
        editing.set(None);
    }
    let commit = {
        let source = source.clone();
        let ranges = ranges.clone();
        let session_path = session_path.clone();
        let pane_id = pane_id.clone();
        move |index: usize, new_text: String| {
            let Some(range) = ranges.get(index).cloned() else {
                return;
            };
            let mut spliced =
                String::with_capacity(source.len() + new_text.len().saturating_sub(range.len()));
            spliced.push_str(&source[..range.start]);
            spliced.push_str(&new_text);
            spliced.push_str(&source[range.end..]);
            let mut state = state;
            let (session_path, pane_id) = (session_path.clone(), pane_id.clone());
            state.with_mut_counted(|shell| {
                // The splice IS an editor draft: same value id, same channel,
                // same draft action the split editor uses — one write path.
                shell.set_document_pane_value(&session_path, "editor", spliced);
            });
            spawn(document_pane_run_action(
                state,
                session_path,
                pane_id,
                "draft".to_string(),
                None,
            ));
        }
    };
    let document_prose = ProseTokens::document();
    let document_ink = doc.prose_ink();
    rsx! {
        div {
            style: document_reading_typography(),
            for (index, block) in blocks.iter().enumerate() {
                if editable && *editing.read() == Some(index) {
                    {
                    let commit_block = commit.clone();
                    rsx! {
                    LiveMarkdownBlockEditor {
                        key: "md-edit-{index}",
                        index: index,
                        block: block.clone(),
                        draft: edit_text,
                        doc: doc.clone(),
                        on_commit: move |text: String| {
                            commit_block(index, text);
                            editing.set(None);
                        },
                        on_cancel: move |_| editing.set(None),
                    }
                    }
                    }
                } else {
                    div {
                        key: "md-block-{index}",
                        "data-md-editable-block": if editable { "{index}" },
                        // The ONLY hover affordance is the text cursor. No bar,
                        // no outline — a hover that announces "this may become
                        // an editor" with a painted edge was read as damage
                        // (user, 2026-08-28: "the line is SO UGLY"), and an
                        // accent left bar is the BLOCKQUOTE vocabulary anyway.
                        style: if editable { "cursor:text;" } else { "" },
                        onclick: {
                            let source = source.clone();
                            let ranges = ranges.clone();
                            move |_| {
                                if !editable {
                                    return;
                                }
                                if let Some(range) = ranges.get(index) {
                                    edit_text.set(source[range.clone()].to_string());
                                    editing.set(Some(index));
                                }
                            }
                        },
                        {md_block_node(block, &document_prose, &document_ink, index)}
                    }
                }
            }
        }
    }
}

/// ── Live markdown editing: the styled mirror under a transparent textarea ─
///
/// The reader's block editor is no longer a raw-ascii detour. The draft
/// renders STYLED while you type, and a syntax marker turns transparent the
/// moment its form is complete: type `#`, see the heading; the space that
/// completes it takes the hash out of view. Commit is the same splice the
/// split editor uses (one write path), Esc cancels, blur commits.
///
/// ⛔ METRICS ARE THE CONTRACT. The textarea and the mirror render the SAME
/// characters on the SAME box — font, size, weight, line-height,
/// letter-spacing, padding, pre-wrap — so the caret the textarea owns always
/// sits on the glyph the mirror paints. Styling that changes advance width is
/// therefore FORBIDDEN inside a line: markers hide with `color:transparent`
/// (width kept), bold is `-webkit-text-stroke` (paint, not layout), emphasis
/// is colour. Weight, family and size changes happen only at BLOCK
/// granularity, where the textarea wears the same typography as the mirror —
/// which is also what makes `#` feel like the heading view: a heading block
/// edits AT heading size and weight.
#[component]
fn LiveMarkdownBlockEditor(
    index: usize,
    block: MdBlock,
    mut draft: Signal<String>,
    doc: DocTheme,
    on_commit: EventHandler<String>,
    on_cancel: EventHandler<()>,
) -> Element {
    let prose = ProseTokens::document();
    let ink = doc.prose_ink();

    // The typography the WHOLE editor wears — decided live from the draft's
    // first line, so typing `# ` into a paragraph block promotes the block to
    // heading typography on the next keystroke, exactly as the reveal
    // promises. Code blocks stay code.
    let first_line = draft.read().lines().next().unwrap_or("").to_string();
    let (kind, prefix_len) = live_line_kind(&first_line);
    let base = match &block {
        MdBlock::CodeBlock(_) => {
            let mut s = prose.code_block_style(&ink);
            s.push_str(" margin:0; padding:0; border:0; border-radius:0;");
            s
        }
        _ => {
            if kind == LiveLineKind::Heading {
                let level = first_line.chars().take_while(|c| *c == '#').count() as u8;
                format!(
                    "{} margin:0; padding:0;",
                    prose.heading_style(level.clamp(1, 4), &ink)
                )
            } else if matches!(&block, MdBlock::BlockQuote(_)) || kind == LiveLineKind::Quote {
                format!(
                    "color:{}; margin:0; padding:0 0 0 12px; border-left:3px solid {};",
                    color_mix(doc.muted.clone(), 100),
                    doc.accent
                )
            } else {
                let mut s = prose.paragraph_style(&ink);
                s.push_str(" font-size:1em; line-height:1.7; margin:0; padding:0;");
                s
            }
        }
    };
    let is_code = matches!(&block, MdBlock::CodeBlock(_));

    // The metrics box: ONE string worn by the mirror and the textarea alike.
    let metrics = format!(
        "{base} font-family:inherit; text-align:left; white-space:pre-wrap; overflow-wrap:anywhere; \
         letter-spacing:normal;"
    );
    let mirror_lines: Vec<Element> = render_live_mirror_lines(&draft.read(), doc.clone(), is_code);
    let mirror_is_empty = mirror_lines.is_empty();
    let tint = format!("background:color-mix(in srgb, {} 9%, transparent);", doc.accent);
    rsx! {
        div {
            key: "live-md-{index}",
            "data-md-live-editor": "1",
            style: format!(
                "position:relative; {tint} border-radius:8px; padding:6px 10px; margin:0 0 14px 0; \
                 cursor:text;",
            ),
            onkeydown: move |evt: KeyboardEvent| {
                if evt.key() == Key::Escape {
                    evt.prevent_default();
                    on_cancel.call(());
                } else if evt.key() == Key::Enter && evt.modifiers().contains(Modifiers::CONTROL) {
                    evt.prevent_default();
                    on_commit.call(draft.peek().clone());
                }
            },
            // THE MIRROR — in flow, so it sizes the block to the draft.
            div {
                "aria-hidden": "true",
                style: format!("{metrics} color:{};", doc.fg),
                for line in mirror_lines {
                    {line}
                }
                // An empty draft renders one empty line so the caret's row
                // exists before the first keystroke.
                if mirror_is_empty {
                    div { style: "{metrics}", "\u{200b}" }
                }
            },
            // THE TEXTAREA — absolute over the mirror, text transparent, real
            // caret. It owns typing, selection and the IME; the mirror is the
            // only visible ink.
            textarea {
                key: "live-md-ta",
                style: format!(
                    "position:absolute; inset:0; width:100%; height:100%; box-sizing:border-box; \
                     padding:6px 10px; margin:0; border:0; {metrics} background:transparent; \
                     color:transparent; caret-color:{}; outline:none; resize:none; overflow:hidden; \
                     cursor:text;",
                    doc.accent
                ),
                value: "{draft.read()}",
                autofocus: true,
                oninput: move |evt: FormEvent| draft.set(evt.value()),
                onblur: move |_| on_commit.call(draft.peek().clone()),
            }
        }
    }
}

/// A width-safe colour-mix helper for the mirror's quiet tones.
fn color_mix(color: String, pct: u32) -> String {
    format!("color-mix(in srgb, {color} {pct}%, transparent)")
}

/// ONE line of a live-edited block, classified. Pure — the editor's decisions
/// are these two answers and both are unit-tested.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum LiveLineKind {
    Heading,
    Quote,
    Bullet,
    Ordered,
    Task,
    Text,
}

/// (kind, prefix length in chars). The prefix is the SYNTAX prefix — `#`, `>`,
/// `- `, `1. `, `- [ ] ` — that the mirror may quiet or hide. A form is
/// complete only with its separating space, which is what makes `#` + space
/// the reveal moment the user asked for.
pub(crate) fn live_line_kind(line: &str) -> (LiveLineKind, usize) {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let spaces = line[hashes..].len() - line[hashes..].trim_start().len();
        // The space IS the reveal: `# ` is a heading the instant it lands,
        // even before a title character exists (the user's example — the
        // hash leaves the view on the space that completes the form).
        if spaces >= 1 {
            let spaces_len = line[hashes..].chars().take_while(|c| *c == ' ').count();
            return (LiveLineKind::Heading, hashes + spaces_len);
        }
    }
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    if rest.starts_with("> ") || rest == ">" {
        return (LiveLineKind::Quote, indent + 1);
    }
    if let Some(body) = rest.strip_prefix("- [") {
        let checked = body.starts_with('x') || body.starts_with('X');
        let unchecked = body.starts_with(' ');
        if (checked || unchecked) && body[1..].starts_with(']') && body[2..].starts_with(' ') {
            return (LiveLineKind::Task, indent + 6);
        }
    }
    for marker in ["- ", "* ", "+ "] {
        if rest.starts_with(marker) {
            return (LiveLineKind::Bullet, indent + 2);
        }
    }
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && rest[digits..].starts_with(". ") {
        return (LiveLineKind::Ordered, indent + digits + 2);
    }
    (LiveLineKind::Text, 0)
}

/// Inline markdown of one line, scanned into segments. Markers travel as
/// `LiveSeg::Marker` so the mirror can paint them transparent while keeping
/// their width. A marker with no closer on its line stays visible: an
/// unclosed form is not a form.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum LiveSeg {
    Marker(String),
    Plain(String),
    Strong(String),
    Em(String),
    Code(String),
}

pub(crate) fn live_inline_segments(line: &str) -> Vec<LiveSeg> {
    fn push_seg(out: &mut Vec<LiveSeg>, st: LiveInlineState, buf: &mut String) {
        if buf.is_empty() {
            return;
        }
        out.push(match st {
            LiveInlineState::Plain => LiveSeg::Plain(std::mem::take(buf)),
            LiveInlineState::Strong => LiveSeg::Strong(std::mem::take(buf)),
            LiveInlineState::Em => LiveSeg::Em(std::mem::take(buf)),
            LiveInlineState::Code => LiveSeg::Code(std::mem::take(buf)),
        });
    }

    #[derive(PartialEq, Clone, Copy)]
    enum LiveInlineState {
        Plain,
        Strong,
        Em,
        Code,
    }

    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<LiveSeg> = Vec::new();
    let mut buf = String::new();
    let mut st = LiveInlineState::Plain;
    let mut i = 0;
    while i < chars.len() {
        let two: String = chars[i..].iter().take(2).collect();
        if st != LiveInlineState::Code && two == "**" {
            push_seg(&mut out, st, &mut buf);
            out.push(LiveSeg::Marker("**".into()));
            st = if st == LiveInlineState::Strong {
                LiveInlineState::Plain
            } else {
                LiveInlineState::Strong
            };
            i += 2;
            continue;
        }
        let c = chars[i];
        if st != LiveInlineState::Code
            && st != LiveInlineState::Strong
            && (c == '*' || c == '_')
        {
            push_seg(&mut out, st, &mut buf);
            out.push(LiveSeg::Marker(c.to_string()));
            st = if st == LiveInlineState::Em {
                LiveInlineState::Plain
            } else {
                LiveInlineState::Em
            };
            i += 1;
            continue;
        }
        if c == '`' {
            push_seg(&mut out, st, &mut buf);
            out.push(LiveSeg::Marker("`".into()));
            st = if st == LiveInlineState::Code {
                LiveInlineState::Plain
            } else {
                LiveInlineState::Code
            };
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    push_seg(&mut out, st, &mut buf);
    out
}

/// Whether an inline STATE is open at end-of-line — an unclosed `**`, `*` or
/// backtick means its marker(s) must paint VISIBLE (the form is not a form).
pub(crate) fn live_line_has_open_form(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut strong = false;
    let mut em = false;
    let mut code = false;
    let mut i = 0;
    while i < chars.len() {
        let two: String = chars[i..].iter().take(2).collect();
        if !code && two == "**" {
            strong = !strong;
            i += 2;
            continue;
        }
        let c = chars[i];
        if !code && !strong && (c == '*' || c == '_') {
            em = !em;
            i += 1;
            continue;
        }
        if c == '`' {
            code = !code;
        }
        i += 1;
    }
    strong || em || code
}

/// Render the draft's lines as the mirror. Returns one ELEMENT per line; an
/// empty draft returns none (the caller lays one blank row).
fn render_live_mirror_lines(
    draft: &str,
    doc: DocTheme,
    is_code_block: bool,
) -> Vec<Element> {
    let mut lines = Vec::new();
    for (index, line) in draft.lines().enumerate() {
        let open_form = live_line_has_open_form(line);
        let (kind, prefix_len) = live_line_kind(line);
        let muted = color_mix(doc.fg.clone(), 52);
        let hidden = "color:transparent;";
        let mut spans: Vec<Element> = Vec::new();
        let char_offset = |n: usize| -> String {
            line.chars().take(n).collect::<String>()
        };
        let body: String = line.chars().skip(prefix_len).collect();
        if is_code_block {
            spans.push(rsx! { span { "{line}" } });
        } else if prefix_len > 0 {
            let prefix_text = char_offset(prefix_len);
            let prefix_style = match kind {
                LiveLineKind::Heading => hidden.to_string(),
                LiveLineKind::Quote => format!("color:{muted};"),
                LiveLineKind::Bullet | LiveLineKind::Ordered | LiveLineKind::Task => {
                    format!("color:{muted};")
                }
                LiveLineKind::Text => String::new(),
            };
            spans.push(rsx! {
                span { style: "{prefix_style}", "{prefix_text}" }
            });
            let segs = live_inline_segments(&body);
            for (si, seg) in segs.iter().enumerate() {
                let key = format!("{index}-{si}");
                let element = match seg {
                    LiveSeg::Marker(m) => rsx! {
                        span { key: "{key}", style: if open_form { format!("color:{muted};") } else { hidden.to_string() }, "{m}" }
                    },
                    LiveSeg::Plain(t) => rsx! {
                        span { key: "{key}", "{t}" }
                    },
                    LiveSeg::Strong(t) => rsx! {
                        span { key: "{key}", style: format!("-webkit-text-stroke:0.55px {};", doc.fg), "{t}" }
                    },
                    LiveSeg::Em(t) => rsx! {
                        span { key: "{key}", style: format!("color:{};", doc.accent), "{t}" }
                    },
                    LiveSeg::Code(t) => rsx! {
                        span { key: "{key}", style: format!("background:{}; font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;", color_mix(doc.muted.clone(), 22)), "{t}" }
                    },
                };
                spans.push(element);
            }
        } else {
            let segs = live_inline_segments(line);
            for (si, seg) in segs.iter().enumerate() {
                let key = format!("{index}-{si}");
                let element = match seg {
                    LiveSeg::Marker(m) => rsx! {
                        span { key: "{key}", style: if open_form { format!("color:{muted};") } else { hidden.to_string() }, "{m}" }
                    },
                    LiveSeg::Plain(t) => rsx! {
                        span { key: "{key}", "{t}" }
                    },
                    LiveSeg::Strong(t) => rsx! {
                        span { key: "{key}", style: format!("-webkit-text-stroke:0.55px {};", doc.fg), "{t}" }
                    },
                    LiveSeg::Em(t) => rsx! {
                        span { key: "{key}", style: format!("color:{};", doc.accent), "{t}" }
                    },
                    LiveSeg::Code(t) => rsx! {
                        span { key: "{key}", style: format!("background:{}; font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;", color_mix(doc.muted.clone(), 22)), "{t}" }
                    },
                };
                spans.push(element);
            }
        }
        let _ = muted;
        lines.push(rsx! {
            div {
                key: "live-line-{index}",
                style: "min-height:1em;",
                for span in spans {
                    {span}
                }
            }
        });
    }
    lines
}

/// The DOCUMENT SURFACE body: the viewport-placement pane's schema rendered
/// at document scale. Chrome widgets (tabs, buttons, toggles, labels) form a
/// top bar; `markdown` and multiline `text-input` widgets are the scrolling
/// body. Same ownership contract as the rail: the app declares, yggterm
/// renders generic widgets and knows nothing about notes.
/// The document surface's color system, derived from the ACTIVE TERMINAL
/// theme so a document reads as part of the terminal workspace the user
/// themed — not as a foreign light panel over a dark terminal (user
/// direction 2026-07-17). `color-mix` derives the soft tones so any of the
/// ~400 catalog themes works without per-theme tuning.
#[derive(Clone, PartialEq)]
struct DocTheme {
    bg: String,
    fg: String,
    muted: String,
    accent: String,
    border: String,
    chrome: String,
}

impl DocTheme {
    /// The document palette for a SHELL surface (the Web View's conversation),
    /// as opposed to a terminal's. One constructor per palette kind so the
    /// markdown renderer never has to guess which one it was handed.
    fn from_palette(palette: &Palette) -> Self {
        Self {
            bg: palette.panel.to_string(),
            fg: palette.text.to_string(),
            muted: palette.muted.to_string(),
            accent: palette.accent.to_string(),
            border: palette.border.to_string(),
            chrome: palette.panel_alt.to_string(),
        }
    }

    fn from_terminal(palette: &crate::terminal_themes::TerminalPaletteSpec) -> Self {
        let fg = palette.foreground.clone();
        let bg = palette.background.clone();
        Self {
            muted: format!("color-mix(in srgb, {fg} 55%, {bg})"),
            border: format!("color-mix(in srgb, {fg} 22%, transparent)"),
            chrome: format!("color-mix(in srgb, {fg} 7%, {bg})"),
            accent: palette.blue.clone(),
            bg,
            fg,
        }
    }

    /// This theme as the brand half of the prose type system.
    ///
    /// The ONE crossing point between yggterm's document colours and
    /// [`ProseTokens`]: a host overrides brand and nothing else, so every face,
    /// size and rhythm below comes from libyggterm and only these five colours
    /// come from here.
    fn prose_ink(&self) -> ProseInk {
        ProseInk::new(
            self.fg.clone(),
            self.muted.clone(),
            self.accent.clone(),
            self.border.clone(),
            self.chrome.clone(),
        )
    }
}

#[component]
fn DocumentSurfaceBody(
    snapshot: SharedSnapshot,
    state: Signal<ShellState>,
    session_path: String,
) -> Element {
    let doc = DocTheme::from_palette(&snapshot.palette);
    let Some(surface) = snapshot.document_surfaces.get(&session_path).cloned() else {
        return rsx! {};
    };
    let document_pane = surface.pane.clone();
    let document_values = surface.values.clone();
    let pane_id = document_pane.pane_id.clone();
    let surface_stale = document_pane.stale;
    let stale_app_name = document_pane
        .app_name
        .clone()
        .unwrap_or_else(|| "The app".to_string());
    let stale_overlay_session_path = session_path.clone();
    let pane_state = surface
        .schema
        .as_ref()
        .filter(|state| state.pane_id == pane_id);
    let value_epochs = pane_state
        .map(|state| state.value_epochs.clone())
        .unwrap_or_default();
    let schema = pane_state.map(|state| state.schema.clone());
    let error = surface.error.clone();

    let run_action = {
        let session_path = session_path.clone();
        let pane_id = pane_id.clone();
        move |action: String, value: Option<String>| {
            spawn(document_pane_run_action(
                state,
                session_path.clone(),
                pane_id.clone(),
                action,
                value,
            ));
        }
    };

    // Outer column: transparent, so the ribbon sits on the shell's own
    // background (seamless with startpage) and the card starts below it.
    // The card keeps the old layer look; only its positioning changed
    // from absolute-fill to flex-fill inside this column.
    let layer_style = "position:absolute; inset:0; z-index:20; display:flex; \
         flex-direction:column; min-width:0; min-height:0; overflow:hidden; \
         background:transparent;";
    let card_style = format!(
        "flex:1 1 auto; min-height:0; display:flex; flex-direction:column; \
         min-width:0; overflow:hidden; background:{}; color:{}; border-radius:11px;",
        doc.bg, doc.fg
    );
    // Opaque chrome, never transparent: the terminal layer paints UNDER
    // this column, and a transparent strip shows the dark terminal
    // bleeding through (the black-bar abomination, 2026-09-03). panel_alt
    // is the same chrome the in-card bar always wore — seamless with the
    // gaps around the card, never the viewport's background.
    let ribbon_style = format!(
        "display:flex; align-items:center; gap:10px; flex-wrap:wrap; \
         padding:4px 14px 8px; flex:0 0 auto; background:{};",
        doc.chrome
    );
    // The ribbon: toolbar strip above the card. Subset vocabulary
    // (AppPaneSchema::ribbon) — label/toolbar/button; the body keeps
    // everything else.
    let ribbon_widgets: Vec<AppPaneWidget> = schema
        .as_ref()
        .map(|schema| schema.ribbon.iter().cloned().collect())
        .unwrap_or_default();
    let bar_style = format!(
        "display:flex; align-items:center; gap:8px; flex-wrap:wrap; padding:7px 14px; \
         border-bottom:1px solid {}; background:{}; flex:0 0 auto;",
        doc.border, doc.chrome
    );
    let bar_button_style = format!(
        "padding:4px 11px; border:1px solid {}; border-radius:7px; \
         background:transparent; color:{}; font-size:11px; font-weight:600; cursor:pointer;",
        doc.border, doc.fg
    );
    let bar_button_primary_style = format!(
        "padding:4px 11px; border:0; border-radius:7px; background:{}; color:{}; \
         font-size:11px; font-weight:700; cursor:pointer;",
        doc.accent, doc.bg
    );
    let bar_label_style = format!("font-size:11px; color:{};", doc.muted);
    let bar_title_style = format!(
        "font-size:12px; font-weight:700; color:{}; overflow:hidden; \
         text-overflow:ellipsis; white-space:nowrap; min-width:0;",
        doc.fg
    );
    // The yggterm-owned way back to the terminal. In the bar when the app
    // declared bar widgets; floating top-right when the viewport is pure body.
    let terminal_toggle_style = format!(
        "padding:4px 11px; border:1px solid {}; border-radius:7px; cursor:pointer; \
         background:{}; color:{}; font-size:11px; font-weight:600;",
        doc.border, doc.chrome, doc.fg
    );

    // Chrome widgets go to the bar; markdown, multiline inputs and rows are
    // the body.
    let (bar_widgets, body_widgets): (Vec<AppPaneWidget>, Vec<AppPaneWidget>) = schema
        .as_ref()
        .map(|schema| {
            schema.widgets.iter().cloned().partition(|widget| {
                !matches!(
                    widget,
                    AppPaneWidget::Markdown { .. }
                        | AppPaneWidget::TextInput {
                            multiline: true,
                            ..
                        }
                        | AppPaneWidget::ListRow { .. }
                )
            })
        })
        .unwrap_or_default();
    let has_bar = !bar_widgets.is_empty();
    // Editor + markdown together = the SPLIT VIEW (markdown-mode editing):
    // editor left, live preview right, each scrolling independently.
    let split_view = body_widgets.iter().any(|w| {
        matches!(
            w,
            AppPaneWidget::TextInput {
                multiline: true,
                ..
            }
        )
    }) && body_widgets
        .iter()
        .any(|w| matches!(w, AppPaneWidget::Markdown { .. }));
    // Where the gutter sits. The app may declare it; absent means centred.
    // Clamped through the contract so host and app cannot disagree about the
    // limit — a disagreement shows up as a gutter that snaps back mid-drag.
    let split_ratio = yggui_contract::clamp_document_split_ratio(
        schema
            .as_ref()
            .and_then(|s| s.split_ratio)
            .unwrap_or(yggui_contract::DOCUMENT_SPLIT_DEFAULT_RATIO),
    );
    let split_first_pct = split_ratio * 100.0;
    // yfiles Dolphin grid: a bar `tabs` with id=view_mode active=icons flips
    // the document body's list into a grid of cards (their polish lane).
    let is_grid = bar_widgets
        .iter()
        .any(|w| matches!(w, AppPaneWidget::Tabs { id, active, .. } if id == "view_mode" && active == "icons"));

    rsx! {
        div {
            "data-document-surface": "{pane_id}",
            style: "{layer_style}",
            // Each viewport type owns its context menu (user call 2026-07-24).
            // A document surface gets the EDITOR menu — Copy/Cut/Paste/Select All
            // — not the Live Session row menu that used to appear over every
            // viewport, and not a native menu we cannot verify raises here.
            oncontextmenu: {
                let mut state = state;
                move |evt: MouseEvent| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    let coords = evt.client_coordinates();
                    // The overlay needs an anchor row; the surface's own items are
                    // what render, so any row for the active session serves.
                    let row = state.with(|shell| {
                        let active = shell.server.active_session_path().map(str::to_string)?;
                        shell
                            .snapshot()
                            .rows
                            .iter()
                            .find(|row| row.full_path == active)
                            .cloned()
                    });
                    if let Some(row) = row {
                        state.with_mut_counted(|shell| {
                            shell.open_viewport_context_menu(
                                ViewportMenuKind::Document,
                                row,
                                (coords.x, coords.y),
                            );
                        });
                    }
                }
            },
            // The ribbon lives OUTSIDE the card: app toolbar strip on
            // shell background, viewport below. Buttons POST on the same
            // document channel as the bar's.
            if !ribbon_widgets.is_empty() {
                div {
                    "data-document-ribbon": "{pane_id}",
                    style: "{ribbon_style}",
                    for (index, widget) in ribbon_widgets.iter().enumerate() {
                        {
                            let widget_key = widget.key(index, &value_epochs);
                            match widget {
                                AppPaneWidget::Section { text, .. } | AppPaneWidget::Label { text, .. } => rsx! {
                                    span {
                                        key: "{widget_key}",
                                        style: "{bar_title_style}",
                                        "{text}"
                                    }
                                },
                                AppPaneWidget::Toolbar { id, buttons } => rsx! {
                                    div {
                                        key: "{widget_key}",
                                        "data-document-ribbon-toolbar": "{id}",
                                        style: "display:flex; align-items:center; gap:6px; flex-wrap:wrap;",
                                        for toolbar_button in buttons.iter().cloned() {
                                            button {
                                                key: "{toolbar_button.action}",
                                                "data-document-button": "{toolbar_button.action}",
                                                style: if toolbar_button.primary {
                                                    bar_button_primary_style.clone()
                                                } else {
                                                    bar_button_style.clone()
                                                },
                                                title: "{toolbar_button.title}",
                                                onclick: {
                                                    let run_action = run_action.clone();
                                                    let action = toolbar_button.action.clone();
                                                    move |_| run_action(action.clone(), None)
                                                },
                                                {shell_glyph(&toolbar_button.label, 13)}
                                            }
                                        }
                                    }
                                },
                                AppPaneWidget::Button { id, label, action, primary, title, .. } => rsx! {
                                    button {
                                        key: "{widget_key}",
                                        "data-document-button": "{id}",
                                        title: "{title}",
                                        style: if *primary { bar_button_primary_style.clone() } else { bar_button_style.clone() },
                                        onclick: {
                                            let run_action = run_action.clone();
                                            let action = action.clone();
                                            move |_| run_action(action.clone(), None)
                                        },
                                        {shell_glyph(label, 13)}
                                    }
                                },
                                AppPaneWidget::RibbonBar { id, action, active, tabs, groups } => rsx! {
                                    div {
                                        key: "{widget_key}",
                                        "data-document-ribbon-bar": "{id}",
                                        style: "flex:1 1 auto; min-width:0; display:flex; flex-direction:column; gap:0;",
                                        // tab strip: text tabs, active tab carries the accent underline
                                        div {
                                            style: "display:flex; align-items:flex-end; gap:1px; padding:1px 6px 0; border-bottom:1px solid {doc.border};",
                                            for tab in tabs.iter().cloned() {
                                                button {
                                                    key: "{id}-{tab.id}",
                                                    style: if tab.id == *active {
                                                        format!("padding:4px 13px 5px; background:transparent; color:{}; font-size:11.5px; border:0; border-bottom:2px solid {}; font-weight:600; cursor:pointer;", doc.fg, doc.accent)
                                                    } else {
                                                        format!("padding:4px 13px 5px; background:transparent; color:{}; font-size:11.5px; border:0; border-bottom:2px solid transparent; cursor:pointer;", doc.muted)
                                                    },
                                                    title: "{tab.label}",
                                                    onclick: {
                                                        let run_action = run_action.clone();
                                                        let action = action.clone();
                                                        let tab_id = tab.id.clone();
                                                        move |_| run_action(action.clone(), Some(tab_id.clone()))
                                                    },
                                                    "{tab.label}"
                                                }
                                            }
                                        },
                                        // command row: groups with caption captions, divider-separated
                                        div {
                                            style: "display:flex; align-items:stretch; flex-wrap:wrap; padding:3px 8px 4px; row-gap:4px;",
                                            for (gi, group) in groups.iter().enumerate() {
                                                div {
                                                    key: "{id}-group{gi}",
                                                    style: if group.right {
                                                        "margin-left:auto; display:flex; flex-direction:column; gap:2px; padding:1px 10px; justify-content:center;".to_string()
                                                    } else if gi > 0 {
                                                        format!("display:flex; flex-direction:column; gap:2px; padding:1px 10px; border-left:1px solid {}; justify-content:center;", doc.border)
                                                    } else {
                                                        "display:flex; flex-direction:column; gap:2px; padding:1px 10px; justify-content:center;".to_string()
                                                    },
                                                    div {
                                                        style: "display:flex; gap:4px; flex-wrap:wrap;",
                                                        for ribbon_group_button in group.buttons.iter().cloned() {
                                                            button {
                                                                key: "{ribbon_group_button.action}",
                                                                "data-document-button": "{ribbon_group_button.action}",
                                                                title: "{ribbon_group_button.title}",
                                                                style: if ribbon_group_button.primary {
                                                                    bar_button_primary_style.clone()
                                                                } else {
                                                                    format!("padding:3px 10px; border:1px solid transparent; border-radius:6px; background:transparent; color:{}; font-size:11px; cursor:pointer;", doc.fg)
                                                                },
                                                                onclick: {
                                                                    let run_action = run_action.clone();
                                                                    let action = ribbon_group_button.action.clone();
                                                                    move |_| run_action(action.clone(), None)
                                                                },
                                                                {shell_glyph(&ribbon_group_button.label, 13)}
                                                            }
                                                        }
                                                    },
                                                    {(!group.label.is_empty()).then(|| {
                                                        let label_style = format!("font-size:9px; color:{}; text-align:center;", doc.muted);
                                                        rsx! { span { style: "{label_style}", "{group.label}" } }
                                                    })}
                                                }
                                            }
                                        }
                                    }
                                },
                                // Ribbon subset is label/toolbar/button; anything
                                // else is the same empty span an unmatched bar
                                // widget renders — never a hole in the strip.
                                _ => rsx! { span { key: "{widget_key}" } },
                            }
                        }
                    }
                }
            }
            div {
                style: "{card_style}",
                if has_bar {
                    div {
                        style: "{bar_style}",
                        for (index, widget) in bar_widgets.iter().enumerate() {
                            {
                                let widget_key = widget.key(index, &value_epochs);
                                match widget {
                                    AppPaneWidget::Section { text, .. } | AppPaneWidget::Label { text, muted: _ } => rsx! {
                                        span {
                                            key: "{widget_key}",
                                            style: if matches!(widget, AppPaneWidget::Section { .. }) { bar_title_style.clone() } else { bar_label_style.clone() },
                                            "{text}"
                                        }
                                    },
                                    AppPaneWidget::Tabs { id, action, tabs, active } => rsx! {
                                        div {
                                            key: "{widget_key}",
                                            style: "display:flex; gap:4px; flex-wrap:wrap; min-width:0;",
                                            for tab in tabs.iter().cloned() {
                                                button {
                                                    key: "{id}-{tab.id}",
                                                    style: if tab.id == *active { bar_button_primary_style.clone() } else { bar_button_style.clone() },
                                                    title: "{tab.label}",
                                                    onclick: {
                                                        let run_action = run_action.clone();
                                                        let action = action.clone();
                                                        let tab_id = tab.id.clone();
                                                        move |_| run_action(action.clone(), Some(tab_id.clone()))
                                                    },
                                                    "{tab.label}"
                                                }
                                            }
                                        }
                                    },
                                    AppPaneWidget::Toggle { id: _, label, action, value } => rsx! {
                                        button {
                                            key: "{widget_key}",
                                            style: if *value { bar_button_primary_style.clone() } else { bar_button_style.clone() },
                                            onclick: {
                                                let run_action = run_action.clone();
                                                let action = action.clone();
                                                let next = (!*value).to_string();
                                                move |_| run_action(action.clone(), Some(next.clone()))
                                            },
                                            "{label}"
                                        }
                                    },
                                    AppPaneWidget::Button { id, label, action, primary, title, .. } => rsx! {
                                        button {
                                            key: "{widget_key}",
                                            "data-document-button": "{id}",
                                            title: "{title}",
                                            style: if *primary { bar_button_primary_style.clone() } else { bar_button_style.clone() },
                                            onclick: {
                                                let run_action = run_action.clone();
                                                let action = action.clone();
                                                move |_| run_action(action.clone(), None)
                                            },
                                            {shell_glyph(label, 13)}
                                        }
                                    },
                                    // A SEARCH BOX IS A TEXT INPUT HERE, and it has to
                                    // be, because a widget this placement does not
                                    // match is dropped in SILENCE — no error, no
                                    // fallback, nothing in the app's transcript. The
                                    // rail rendered `search-box` and the document
                                    // surface did not, so yRDP's chooser had a filter
                                    // in the rail and none in the viewport; the moment
                                    // the rail went away the filter went with it, and
                                    // the app had no way to find that out.
                                    AppPaneWidget::TextInput { id, placeholder, value, action, .. }
                                    | AppPaneWidget::SearchBox { id, placeholder, value, action } => rsx! {
                                        input {
                                            key: "{widget_key}",
                                            "data-document-input": "{id}",
                                            style: format!(
                                                "padding:4px 10px; border:1px solid {}; border-radius:7px; \
                                                 background:{}; color:{}; font-size:11px; outline:none; \
                                                 min-width:200px; flex:0 1 340px;",
                                                doc.border, doc.bg, doc.fg
                                            ),
                                            placeholder: "{placeholder}",
                                            initial_value: "{value}",
                                            oninput: {
                                                let mut state = state;
                                                let id = id.clone();
                                                let session_path = session_path.clone();
                                                move |evt: FormEvent| {
                                                    state.with_mut_counted(|shell| {
                                                        shell.set_document_pane_value(
                                                            &session_path,
                                                            &id,
                                                            evt.value(),
                                                        );
                                                    });
                                                }
                                            },
                                            onkeydown: {
                                                let run_action = run_action.clone();
                                                let action = action.clone();
                                                move |evt: KeyboardEvent| {
                                                    if evt.key() == Key::Enter && !action.is_empty() {
                                                        run_action(action.clone(), None);
                                                    }
                                                }
                                            },
                                        }
                                    },
                                    // Several buttons the app groups together. The bar
                                    // is already a row, so a toolbar is its buttons
                                    // wearing the bar's own skin rather than a second
                                    // container inside it.
                                    AppPaneWidget::Toolbar { id, buttons } => rsx! {
                                        div {
                                            key: "{widget_key}",
                                            "data-document-toolbar": "{id}",
                                            style: "display:flex; align-items:center; gap:6px; flex-wrap:wrap;",
                                            for toolbar_button in buttons.iter().cloned() {
                                                button {
                                                    key: "{toolbar_button.action}",
                                                    "data-document-button": "{toolbar_button.action}",
                                                    style: if toolbar_button.primary {
                                                        bar_button_primary_style.clone()
                                                    } else {
                                                        bar_button_style.clone()
                                                    },
                                                    title: "{toolbar_button.title}",
                                                    onclick: {
                                                        let run_action = run_action.clone();
                                                        let action = toolbar_button.action.clone();
                                                        move |_| run_action(action.clone(), None)
                                                    },
                                                    "{toolbar_button.label}"
                                                }
                                            }
                                        }
                                    },
                                    AppPaneWidget::NumberInput { id, label, value, min, max } => rsx! {
                                        div {
                                            key: "{widget_key}",
                                            style: "display:flex; align-items:center; gap:6px;",
                                            if !label.is_empty() {
                                                span { style: "{bar_label_style}", "{label}" }
                                            }
                                            input {
                                                "data-document-input": "{id}",
                                                style: format!(
                                                    "padding:4px 8px; border:1px solid {}; border-radius:7px; \
                                                     background:{}; color:{}; font-size:11px; outline:none; \
                                                     max-width:88px;",
                                                    doc.border, doc.bg, doc.fg
                                                ),
                                                r#type: "number",
                                                min: "{min}",
                                                max: "{max}",
                                                initial_value: "{value}",
                                                oninput: {
                                                    let mut state = state;
                                                    let id = id.clone();
                                                    let session_path = session_path.clone();
                                                    move |evt: FormEvent| {
                                                        state.with_mut_counted(|shell| {
                                                            shell.set_document_pane_value(
                                                                &session_path,
                                                                &id,
                                                                evt.value(),
                                                            );
                                                        });
                                                    }
                                                },
                                            }
                                        }
                                    },
                                    // ⛔ The silent drop. Everything a document surface
                                    // can be handed is matched ABOVE — locked by
                                    // `every_app_pane_widget_reaches_the_document_surface`
                                    // — so this arm exists for the variant somebody
                                    // adds next, and it is the reason that lock has to
                                    // be structural: an unmatched widget renders as an
                                    // empty span, which looks exactly like an app that
                                    // never declared it.
                                    _ => rsx! { span { key: "{widget_key}" } },
                                }
                            }
                        }
                        div { style: "flex:1 1 auto;" }
                        // ⛔ The Document|Terminal switch does NOT live here. It is
                        // the titlebar's surface-switch slot
                        // (`TitlebarSurfaceSwitch`), which is where every other
                        // "what is this viewport showing" switch already lived.
                        // A copy here floated over the editor on a pure-body
                        // document — the surface reserves no space for chrome, so
                        // the pill drew straight over the first line of the text.
                    }
                }
                if let Some(error) = error {
                    div {
                        style: format!("padding:10px 16px; color:{}; font-size:12px;", doc.muted),
                        "Document unavailable: {error}"
                    }
                }
                div {
                    style: if split_view {
                        "flex:1 1 auto; min-height:0; display:flex; flex-direction:row; overflow:hidden;"
                    } else if is_grid {
                        "flex:1 1 auto; min-height:0; display:grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap:16px; padding:20px; overflow:auto; align-content:start;"
                    } else {
                        "flex:1 1 auto; min-height:0; display:flex; flex-direction:column; overflow:auto;"
                    },
                    if body_widgets.is_empty() && schema.is_some() {
                        div {
                            style: format!("padding:26px; color:{}; font-size:13px;", doc.muted),
                            "The app declared no document body."
                        }
                    } else if schema.is_none() {
                        div {
                            style: format!("padding:26px; color:{}; font-size:13px;", doc.muted),
                            "Loading document…"
                        }
                    }
                    for (index, widget) in body_widgets.iter().enumerate() {
                        div {
                            key: "half-{widget.key(index, &value_epochs)}",
                            // ⛔ SPELLED OUT, NOT INTERPOLATED, AND THAT IS LOAD-BEARING.
                            // An RSX attribute NAME is a literal: `"{EXPR}"` interpolates
                            // a VALUE, never a name, so this written as
                            // `"{yggui_contract::document_split_stamps::HALF}"` emitted an
                            // attribute called `{yggui_contract::…::HALF}` — braces, colons
                            // and all — which `setAttribute` refuses outright
                            // (`InvalidCharacterError: Invalid qualified name`). The throw
                            // killed the whole edit batch, so EVERY mutation after it was
                            // dropped and never re-sent: the halves, the gutter, the editor
                            // and the reader all failed to mount while the container's own
                            // `style` (emitted before the throw) kept tracking the data.
                            // That is why the viewport painted nothing while the rail, the
                            // footer counts and `document_surfaces.has_schema` all reported
                            // success — they come from a different pane and a different
                            // batch. Locked by `document_split_stamp_attribute_names_are_literal`.
                            "data-yggui-doc-split-half": if split_view {
                                if index == 0 { "first" } else { "second" }
                            } else { "" },
                            style: if split_view {
                                // flex-basis carries the ratio; grow is 0 so the
                                // gutter's position is the ratio and nothing else.
                                // The border is gone — the gutter IS the divider now.
                                format!(
                                    "order:{}; flex:0 1 calc({}% - 3px); min-width:0; min-height:0; overflow:auto;",
                                    index * 2,
                                    if index == 0 { split_first_pct } else { 100.0 - split_first_pct },
                                )
                            } else {
                                "display:contents;".to_string()
                            },
                        {
                            let widget_key = widget.key(index, &value_epochs);
                            match widget {
                                AppPaneWidget::Markdown { id, source, live_from } => rsx! {
                                    div {
                                        key: "{widget_key}",
                                        "data-document-markdown": "{id}",
                                        style: "padding:16px 28px 40px 28px; max-width:980px; width:100%; margin:0 auto; box-sizing:border-box;",
                                        if live_from.is_empty() {
                                            // The pure READER: no sibling editor, so
                                            // blocks are click-to-edit in place
                                            // (Phase 4 — Typora-lite, not WYSIWYG).
                                            EditableMarkdownBody {
                                                source: source.clone(),
                                                doc: doc.clone(),
                                                state,
                                                session_path: session_path.clone(),
                                                pane_id: pane_id.clone(),
                                            }
                                        } else {
                                            // Live preview beside an editor: the
                                            // sibling's draft renders per keystroke,
                                            // no app round trip — and edits belong
                                            // in the editor, not here.
                                            {
                                                let live = document_values.get(live_from).cloned();
                                                markdown_widget_body(
                                                    live.as_deref().unwrap_or(source),
                                                    &doc,
                                                    ProseTokens::document(),
                                                )
                                            }
                                        }
                                    }
                                },
                                AppPaneWidget::TextInput { id, value, line_numbers, word_wrap, .. } => {
                                    // The gutter tracks the LIVE draft, not the last
                                    // declared value, so typing a newline never
                                    // desyncs the numbers.
                                    let live = document_values
                                        .get(id)
                                        .cloned()
                                        .unwrap_or_else(|| value.clone());
                                    let line_count = live.split('\n').count().max(1);
                                    // Text-mode editor is one point larger than the
                                    // 12.5px chrome baseline (user call 2026-07-24):
                                    // source editing wants a touch more air than a
                                    // dense sidebar row. The gutter shares it so the
                                    // numbers sit on the same baseline as the text.
                                    let editor_font = "font-family:ui-monospace, monospace; font-size:13.5px; line-height:1.55;";
                                    // Wrap mode keeps its line numbers now: a hidden
                                    // mirror measures each logical line's visual-row
                                    // count, and the gutter draws the number on a
                                    // line's first row + a continuation arrow (↪) on
                                    // each wrapped row, KDE-Kate style (see
                                    // DOCUMENT_WRAP_GUTTER_SCRIPT). The textarea owns
                                    // its own scroll; the gutter's inner block tracks
                                    // its scrollTop.
                                    let wrapped = *word_wrap;
                                    let show_gutter = *line_numbers;
                                    // The wrap gutter's JS pairs textarea↔gutter by this
                                    // marker; absent (None) in non-wrap mode.
                                    let wrap_editor_marker = wrapped.then(|| id.clone());
                                    rsx! {
                                        div {
                                            key: "{widget_key}",
                                            style: if wrapped {
                                                "display:flex; align-items:stretch; min-height:100%; height:100%;"
                                            } else {
                                                "display:flex; align-items:flex-start; min-height:100%;"
                                            },
                                            if show_gutter && !wrapped {
                                                div {
                                                    "data-document-gutter": "{id}",
                                                    style: format!(
                                                        "flex:0 0 auto; text-align:right; padding:14px 10px 40px 16px; \
                                                         color:{}; border-right:1px solid {}; user-select:none; \
                                                         -webkit-user-select:none; white-space:pre; {editor_font}",
                                                        doc.muted, doc.border
                                                    ),
                                                    {(1..=line_count).map(|n| n.to_string()).collect::<Vec<_>>().join("\n")}
                                                }
                                            }
                                            if show_gutter && wrapped {
                                                // JS-maintained gutter: overflow-hidden
                                                // frame, inner block translated by the
                                                // textarea's scrollTop. Padding-top
                                                // matches the textarea so line 1 aligns.
                                                div {
                                                    "data-document-wrap-gutter": "{id}",
                                                    style: format!(
                                                        "flex:0 0 auto; overflow:hidden; text-align:right; \
                                                         padding:14px 10px 40px 16px; color:{}; border-right:1px solid {}; \
                                                         user-select:none; -webkit-user-select:none; white-space:pre; {editor_font}",
                                                        doc.muted, doc.border
                                                    ),
                                                    div {}
                                                }
                                            }
                                            textarea {
                                                "data-document-editor": "{id}",
                                                "data-document-wrap-editor": wrap_editor_marker,
                                                style: if wrapped {
                                                    format!(
                                                        "flex:1 1 auto; min-width:0; border:0; outline:none; resize:none; \
                                                         background:transparent; color:{}; caret-color:{}; \
                                                         padding:14px 20px 40px 20px; white-space:pre-wrap; overflow-wrap:anywhere; \
                                                         overflow-x:hidden; overflow-y:auto; tab-size:4; {editor_font}",
                                                        doc.fg, doc.accent
                                                    )
                                                } else {
                                                    format!(
                                                        "flex:1 1 auto; min-width:0; border:0; outline:none; resize:none; \
                                                         background:transparent; color:{}; caret-color:{}; \
                                                         padding:14px 20px 40px 14px; white-space:pre; overflow-x:auto; \
                                                         overflow-y:hidden; tab-size:4; {editor_font}",
                                                        doc.fg, doc.accent
                                                    )
                                                },
                                                spellcheck: "false",
                                                wrap: if wrapped { "soft" } else { "off" },
                                                rows: if wrapped { "2".to_string() } else { format!("{}", line_count + 1) },
                                                initial_value: "{value}",
                                                onmounted: move |_evt| async move {
                                                    // Install/refresh the wrap gutter maintainer
                                                    // once the textarea is in the DOM. Idempotent.
                                                    if wrapped {
                                                        let _ = document::eval(DOCUMENT_WRAP_GUTTER_SCRIPT);
                                                    }
                                                },
                                                oninput: {
                                                    let mut state = state;
                                                    let id = id.clone();
                                                    let session_path = session_path.clone();
                                                    move |evt: FormEvent| {
                                                        state.with_mut_counted(|shell| {
                                                            shell.set_document_pane_value(
                                                                &session_path,
                                                                &id,
                                                                evt.value(),
                                                            );
                                                        });
                                                    }
                                                },
                                            }
                                        }
                                    }
                                },
                                AppPaneWidget::ListRow { id, title, subtitle, icon, selected, row_action, actions, .. } => {
                                    if is_grid {
                                        rsx! {
                                            div {
                                                key: "{widget_key}",
                                                "data-document-row": "{id}",
                                                "data-selected": if *selected { "true" } else { "false" },
                                                style: format!(
                                                    "display:flex; flex-direction:column; align-items:center; gap:8px; padding:14px 8px;                                                  border-radius:12px; background:{}; color:{}; cursor:pointer; text-align:center;                                                  border:1px solid {}; transition: background 120ms; min-width:0; overflow:hidden;",
                                                    if *selected { format!("color-mix(in srgb, {} 14%, transparent)", doc.accent) } else { "transparent".to_string() },
                                                    doc.fg,
                                                    if *selected { format!("color-mix(in srgb, {} 22%, transparent)", doc.accent) } else { "transparent".to_string() },
                                                ),
                                                onclick: {
                                                    let run_action = run_action.clone();
                                                    let ra = row_action.clone();
                                                    let row_id = id.clone();
                                                    move |_| {
                                                        if !ra.is_empty() {
                                                            run_action(ra.clone(), Some(row_id.clone()));
                                                        }
                                                    }
                                                },
                                                {
                                                    let icon_str = icon.clone();
                                                    if icon_str.starts_with("file:") {
                                                        let ext = icon_str.strip_prefix("file:").unwrap_or("").to_string();
                                                        let label = if ext.is_empty() || ext == "·" { "·".to_string() } else { ext.chars().take(4).collect::<String>().to_lowercase() };
                                                        rsx! {
                                                            span {
                                                                style: "display:inline-flex; align-items:center; justify-content:center; width:48px; height:48px; flex:0 0 48px; border-radius:9px; background:rgba(127,127,127,0.13); border:1px solid rgba(127,127,127,0.18);",
                                                                span {
                                                                    style: "font-size:11px; font-weight:700; letter-spacing:0.02em; color:rgba(90,90,90,0.85); text-transform:uppercase;",
                                                                    "{label}"
                                                                }
                                                            }
                                                        }
                                                    } else if let Some(si) = ShellIcon::from_token(&icon_str) {
                                                        rsx! {
                                                            span {
                                                                style: format!("display:inline-flex; align-items:center; justify-content:center; width:48px; height:48px; flex:0 0 48px; border-radius:12px; background:color-mix(in srgb, {} 9%, transparent);", doc.accent),
                                                                ShellIconMark { icon: si, size: 22 }
                                                            }
                                                        }
                                                    } else if !icon_str.is_empty() {
                                                        rsx! {
                                                            span {
                                                                style: "display:inline-flex; align-items:center; justify-content:center; width:48px; height:48px; flex:0 0 48px; border-radius:12px; background:rgba(127,127,127,0.14); font-size:20px;",
                                                                "{icon_str}"
                                                            }
                                                        }
                                                    } else {
                                                        rsx! { span { style: "width:48px; height:48px; flex:0 0 48px;" } }
                                                    }
                                                }
                                                div {
                                                    style: "font-size:12px; font-weight:500; max-width:100%; text-align:center; line-height:1.3; word-break:break-word; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden;",
                                                    "{title}"
                                                }
                                                if !subtitle.is_empty() {
                                                    div {
                                                        style: format!("font-size:10px; color:{}; max-width:100%; text-align:center; line-height:1.2; word-break:break-word; display:-webkit-box; -webkit-line-clamp:1; -webkit-box-orient:vertical; overflow:hidden;", doc.muted),
                                                        "{subtitle}"
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        rsx! {
                                            div {
                                                key: "{widget_key}",
                                                "data-document-row": "{id}",
                                                "data-selected": if *selected { "true" } else { "false" },
                                                style: format!(
                                                    "display:flex; align-items:center; gap:12px; margin:2px 20px; padding:8px 12px;                                                  border-radius:8px; background:{}; color:{}; max-width:860px; cursor:pointer;                                                  border:1px solid {}; transition: background 120ms, border-color 120ms;",
                                                    if *selected { format!("color-mix(in srgb, {} 14%, transparent)", doc.accent) } else { "transparent".to_string() },
                                                    doc.fg,
                                                    if *selected { format!("color-mix(in srgb, {} 22%, transparent)", doc.accent) } else { "transparent".to_string() },
                                                ),
                                                onclick: {
                                                    let run_action = run_action.clone();
                                                    let ra = row_action.clone();
                                                    let row_id = id.clone();
                                                    move |_| {
                                                        if !ra.is_empty() {
                                                            run_action(ra.clone(), Some(row_id.clone()));
                                                        }
                                                    }
                                                },
                                                {
                                                    let icon_str = icon.clone();
                                                    if icon_str.starts_with("file:") {
                                                        let ext = icon_str.strip_prefix("file:").unwrap_or("").to_string();
                                                        let label = if ext.is_empty() || ext == "·" { "·".to_string() } else { ext.chars().take(4).collect::<String>().to_lowercase() };
                                                        rsx! {
                                                            span {
                                                                style: "display:inline-flex; align-items:center; justify-content:center; width:32px; height:32px; flex:0 0 32px; border-radius:7px; background:rgba(127,127,127,0.13); border:1px solid rgba(127,127,127,0.18);",
                                                                span {
                                                                    style: "font-size:9px; font-weight:700; letter-spacing:0.02em; color:rgba(90,90,90,0.85); text-transform:uppercase;",
                                                                    "{label}"
                                                                }
                                                            }
                                                        }
                                                    } else if let Some(si) = ShellIcon::from_token(&icon_str) {
                                                        rsx! {
                                                            span {
                                                                style: "display:inline-flex; align-items:center; justify-content:center; width:32px; height:32px; flex:0 0 32px; border-radius:50%; background:rgba(127,127,127,0.14);",
                                                                ShellIconMark { icon: si, size: 14 }
                                                            }
                                                        }
                                                    } else if !icon_str.is_empty() {
                                                        rsx! {
                                                            span {
                                                                style: "display:inline-flex; align-items:center; justify-content:center; width:32px; height:32px; flex:0 0 32px; border-radius:50%; background:rgba(127,127,127,0.14); font-size:14px;",
                                                                "{icon_str}"
                                                            }
                                                        }
                                                    } else {
                                                        rsx! { span { style: "width:32px; height:32px; flex:0 0 32px;" } }
                                                    }
                                                }
                                                div {
                                                    style: "display:flex; flex-direction:column; gap:1px; min-width:0; flex:1 1 auto;",
                                                    div {
                                                        style: "font-size:13px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; line-height:1.2;",
                                                        "{title}"
                                                    }
                                                    if !subtitle.is_empty() {
                                                        div {
                                                            style: format!("font-size:11.5px; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", doc.muted),
                                                            "{subtitle}"
                                                        }
                                                    }
                                                }
                                                for row_action in actions.iter().cloned() {
                                                    button {
                                                        key: "{row_action.action}",
                                                        style: format!(
                                                            "border:0; border-radius:7px; background:transparent; color:{};                                                          font-size:13px; cursor:pointer; padding:4px 8px; display:inline-flex; align-items:center; justify-content:center; min-width:28px; height:28px;",
                                                            doc.muted
                                                        ),
                                                        title: "{row_action.title}",
                                                        onclick: {
                                                            let run_action = run_action.clone();
                                                            let (action, row_id) = (row_action.action.clone(), id.clone());
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                run_action(action.clone(), Some(row_id.clone()))
                                                            }
                                                        },
                                                        if let Some(si) = ShellIcon::from_token(&row_action.label) {
                                                            ShellIconMark { icon: si, size: 13 }
                                                        } else {
                                                            "{row_action.label}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                _ => rsx! { span { key: "{widget_key}" } },
                            }
                        }
                        }
                    }
                    // THE SPLIT GUTTER. Rendered after the halves and pulled
                    // back over the seam, so it is one element regardless of how
                    // many widgets the app declared. Dragging it sets a CSS var the
                    // halves are sized from, and releases POST the ratio to the app
                    // — the app owns the value, the host only reports the gesture.
                    //
                    // It carries the contract stamps so an agent can find and drive
                    // it through yggui exactly as a pointer does; that is the point
                    // of naming them in yggui-contract rather than inline here.
                    if split_view {
                        div {
                            // Literal names — see the half stamp above for why an
                            // interpolated attribute NAME silently destroys this subtree.
                            "data-yggui-doc-split-gutter": "1",
                            "data-yggui-doc-split-ratio": "{split_ratio}",
                            role: "separator",
                            "aria-orientation": "vertical",
                            "aria-valuenow": "{(split_ratio * 100.0) as i64}",
                            title: "Drag to resize · double-click to centre",
                            style: format!(
                                "order:1; flex:0 0 6px; z-index:2; cursor:col-resize;                              background:{}; opacity:0.55; transition:opacity 120ms;                              touch-action:none; user-select:none;",
                                doc.border,
                            ),
                        }
                    }
                }
            }
            // The NOT-RESPONDING overlay: the app's declares stopped while
            // this session's reads were live (a Ctrl+Z'd app is the canonical
            // case). It covers the whole surface — actions would only hang at
            // the frozen control endpoint — and offers the terminal back,
            // which is where `fg` gets typed. A declare arriving clears it
            // instantly; contribution expiry closes the surface entirely.
            if surface_stale {
                div {
                    "data-document-surface-stale": "1",
                    style: format!(
                        "position:absolute; inset:0; z-index:40; display:flex; align-items:center; \
                         justify-content:center; background:color-mix(in srgb, {} 65%, transparent);",
                        doc.bg
                    ),
                    div {
                        style: format!(
                            "display:flex; flex-direction:column; gap:8px; align-items:center; \
                             padding:18px 26px; border-radius:11px; border:1px solid {}; \
                             background:{}; color:{}; max-width:420px; text-align:center;",
                            doc.border, doc.chrome, doc.fg
                        ),
                        div {
                            style: "font-size:13px; font-weight:700;",
                            "{stale_app_name} is not responding"
                        }
                        div {
                            style: format!("font-size:11px; line-height:1.5; color:{};", doc.muted),
                            "Suspended (Ctrl+Z)? Resume it with fg — this surface closes on its own if the app is gone."
                        }
                        button {
                            style: "{terminal_toggle_style}",
                            onclick: {
                                let mut state = state;
                                move |_| {
                                    let session_path = stale_overlay_session_path.clone();
                                    state.with_mut_counted(|shell| {
                                        shell.document_surface_hidden.insert(session_path);
                                    });
                                }
                            },
                            "Show terminal"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AppPaneRailBody(
    snapshot: SharedSnapshot,
    pane_id: String,
    /// Draw the APP-RAISED MODAL's schema instead of the rail's, without its
    /// rail header (the dialog card draws its own title, subtitle and ✕).
    ///
    /// ⛔ ONE renderer for both on purpose. The widget vocabulary is the
    /// platform's contract with every app; a second copy of it for dialogs would
    /// be a second thing to grow, and the two would diverge on the first widget
    /// somebody added to only one of them.
    #[props(default = false)]
    modal: bool,
    on_app_pane_action: EventHandler<(String, String, Option<String>)>,
    /// `(pane id, action, moved row id, the pane's new row order)` — fired
    /// when a reorderable rail row is dropped somewhere that changes the order.
    on_app_pane_reorder: EventHandler<(String, String, String, Option<String>, Vec<String>)>,
    on_app_pane_value: EventHandler<(String, String)>,
    /// A row's right-click opens its context menu on the shell state directly —
    /// a GUI-owned floating overlay, not an app schema.
    state: Signal<ShellState>,
) -> Element {
    let palette = snapshot.palette;
    let muted_style = format!(
        "font-size:10.5px; line-height:1.55; color:{}; text-wrap:pretty;",
        palette.muted
    );
    // A section heading is the pane's structural voice: caps, tracked out, in
    // the TEXT colour rather than the muted one. It used to be muted-on-muted,
    // which made a form read as one undifferentiated grey column.
    let section_style = format!(
        "font-size:10px; font-weight:800; letter-spacing:0.07em; text-transform:uppercase; color:{};",
        palette.text
    );
    let text_style = format!("font-size:11.5px; color:{};", palette.text);
    // A field's own name. Bitwarden's shape: small, quiet, sitting directly on
    // the box it names — but WEIGHTED, so a column of fields reads as a form
    // rather than as prose with boxes in it.
    let field_label_style = format!(
        "font-size:10.5px; font-weight:650; line-height:1.3; color:{};",
        palette.muted
    );
    // ⛔ NO `background`, NO `box-shadow`: the SKIN belongs to `text_field_css`
    // (hover, focus, placeholder are CSS states an inline style cannot reach),
    // and an inline background would out-specify it and kill the whole thing.
    // This string is the BOX only.
    let field_style = format!(
        "flex:1 1 auto; width:100%; min-width:0; box-sizing:border-box; padding:8px 11px; \
         border-radius:10px; border:none; color:{}; font-size:12px; line-height:1.4; \
         outline:none; font-family:inherit;",
        palette.text,
    );
    let primary_button_style = format!(
        "align-self:flex-start; padding:8px 15px; border:0; border-radius:10px; \
         background:{}; color:#fff; font-size:11.5px; font-weight:700; cursor:pointer; \
         box-shadow:0 1px 2px color-mix(in srgb, {} 45%, transparent); transition:{};",
        palette.accent,
        palette.accent,
        standard_transition(&["background-color", "box-shadow", "transform"])
    );
    let plain_button_style = format!(
        "align-self:flex-start; padding:8px 15px; border:1px solid color-mix(in srgb, {} 30%, rgba(127,127,127,0.34)); \
         border-radius:10px; background:transparent; color:{}; font-size:11.5px; \
         font-weight:600; cursor:pointer; transition:{};",
        palette.accent,
        palette.text,
        standard_transition(&["background-color", "border-color"])
    );
    // A DESTRUCTIVE button wears the product's one red, filled when it is the
    // primary act and outlined when it sits beside one. Same metrics as the
    // pair above, so an action bar does not change height when one of its
    // buttons happens to destroy something.
    let danger_button_style = format!(
        "align-self:flex-start; padding:8px 15px; border:1px solid color-mix(in srgb, {red} 42%, transparent); \
         border-radius:10px; background:color-mix(in srgb, {red} 12%, transparent); color:{red}; \
         font-size:11.5px; font-weight:700; cursor:pointer; transition:{};",
        standard_transition(&["background-color", "border-color"]),
        red = DESTRUCTIVE_RED,
    );
    let pane_state = if modal {
        snapshot.app_pane_modal.as_ref().map(|dialog| &dialog.pane)
    } else {
        snapshot
            .app_pane_schema
            .as_ref()
            .filter(|state| state.pane_id == pane_id)
    };
    let value_epochs = pane_state
        .map(|state| state.value_epochs.clone())
        .unwrap_or_default();
    let schema = pane_state.map(|state| state.schema.clone());
    // A modal's content arrived WITH the action reply that raised it — there is
    // no fetch of its own that could have failed, so the rail's fetch error is
    // not this dialog's to report.
    let error = if modal {
        None
    } else {
        snapshot.app_pane_error.clone()
    };
    let title = schema
        .as_ref()
        .map(|schema| schema.title.clone())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| pane_id.clone());
    let footer_widgets: Vec<AppPaneWidget> = schema
        .as_ref()
        .map(|schema| schema.footer.clone())
        .unwrap_or_default();
    // The pane's reorderable rows, in the order they are drawn — the list a
    // drop permutes. Computed once per render, so every row's drop resolves
    // against the same sequence the user is looking at.
    let reorderable_row_ids: Vec<String> = schema
        .as_ref()
        .map(|schema| app_pane_reorderable_row_ids(&schema.widgets))
        .unwrap_or_default();
    // The pane's SHAPE, for the shared reorder engine. Every list row is in it,
    // groups included, because a drop lands relative to rows the app may not
    // have made draggable; `reorderable_row_ids` stays the PERMISSION list and
    // narrows what actually reaches the app.
    let row_tree: Vec<RowTreeRow> = schema
        .as_ref()
        .map(|schema| app_pane_row_tree(&schema.widgets))
        .unwrap_or_default();

    rsx! {
        if !modal {
            RailHeader { title: title, color: palette.text.to_string() }
        }
        RailScrollBody {
            content: rsx!{
            div {
                style: "display:flex; flex-direction:column; gap:10px;",
                // A release ANYWHERE in the pane ends the gesture, exactly as
                // the WebTabs rail container does. Without it the only exit was
                // the per-row handler, so letting go over a section heading, the
                // toolbar or the footer left the drag standing: the row kept its
                // dim forever and the next pointer move re-armed a drop target.
                onmouseup: {
                    let mut state = state;
                    move |_: MouseEvent| {
                        state.with_mut_counted(|shell| shell.clear_app_pane_row_drag());
                    }
                },
                if let Some(error) = error {
                    div { style: "{muted_style}", "{error}" }
                } else if let Some(schema) = schema {
                    for (band_ix, band) in app_pane_bands(&schema.widgets).into_iter().enumerate() {
                    div {
                        key: "band-{band_ix}",
                        style: app_pane_band_style(palette, band.card),
                    for index in band.indices.iter().copied() {
                        {
                        let widget = schema.widgets[index].clone();
                        let widget_key = widget.key(index, &value_epochs);
                        // Consecutive list-rows form one tight LIST: the 10px
                        // widget gap between them collapses to ~2px, so a file
                        // list has the cwdtree's rhythm, not a stack of cards.
                        let follows_row = index > 0
                            && matches!(
                                (&schema.widgets[index - 1], &widget),
                                (AppPaneWidget::ListRow { .. }, AppPaneWidget::ListRow { .. })
                            );
                        match widget {
                            AppPaneWidget::Section { text, action, action_label, action_title, .. } => rsx! {
                                div {
                                    key: "{widget_key}",
                                    style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                                    div { style: "{section_style}", "{text}" }
                                    if !action.is_empty() && !action_label.is_empty() {
                                        button {
                                            style: format!(
                                                "border:0; border-radius:6px; background:{}; color:#fff; \
                                                 font-size:12px; font-weight:700; cursor:pointer; \
                                                 width:20px; height:20px; line-height:1; padding:0;",
                                                palette.accent
                                            ),
                                            title: "{action_title}",
                                            onclick: {
                                                let (pane_id, action) = (pane_id.clone(), action.clone());
                                                let on_app_pane_action = on_app_pane_action.clone();
                                                move |_| on_app_pane_action.call((pane_id.clone(), action.clone(), None))
                                            },
                                            "{action_label}"
                                        }
                                    }
                                }
                            },
                            AppPaneWidget::Toolbar { id, buttons } => rsx! {
                                div {
                                    key: "{widget_key}",
                                    "data-app-pane-toolbar": "{id}",
                                    style: "display:flex; align-items:center; gap:6px; flex-wrap:wrap;",
                                    for toolbar_button in buttons.iter().cloned() {
                                        button {
                                            key: "{toolbar_button.action}",
                                            "data-toolbar-action": "{toolbar_button.action}",
                                            style: format!(
                                                "border:1px solid rgba(127,127,127,0.30); border-radius:7px; \
                                                 padding:5px 9px; font-size:12px; line-height:1; cursor:pointer; \
                                                 background:{}; color:{}; font-weight:600;",
                                                if toolbar_button.primary { palette.accent } else { "transparent" },
                                                if toolbar_button.primary { "#fff" } else { palette.text },
                                            ),
                                            title: "{toolbar_button.title}",
                                            onclick: {
                                                let (pane_id, action) = (pane_id.clone(), toolbar_button.action.clone());
                                                let on_app_pane_action = on_app_pane_action.clone();
                                                move |_| on_app_pane_action.call((pane_id.clone(), action.clone(), None))
                                            },
                                            "{toolbar_button.label}"
                                        }
                                    }
                                }
                            },
                            AppPaneWidget::Label { text, muted } => {
                                let style = if muted { muted_style.clone() } else { text_style.clone() };
                                rsx! {
                                    div { key: "{widget_key}", style: "{style}", "{text}" }
                                }
                            },
                            AppPaneWidget::Tabs { id, action, tabs, active } => rsx! {
                                div {
                                    key: "{widget_key}",
                                    style: segmented_control_track_style(palette),
                                    for tab in tabs.iter().cloned() {
                                        button {
                                            key: "{tab.id}",
                                            "data-app-pane-tab": "{tab.id}",
                                            style: segmented_control_segment_style(palette, tab.id == active, true, false),
                                            onclick: {
                                                let (pane_id, action, id, tab_id) =
                                                    (pane_id.clone(), action.clone(), id.clone(), tab.id.clone());
                                                let on_app_pane_action = on_app_pane_action.clone();
                                                let on_app_pane_value = on_app_pane_value.clone();
                                                move |_| {
                                                    on_app_pane_value.call((id.clone(), tab_id.clone()));
                                                    if !action.is_empty() {
                                                        on_app_pane_action.call((
                                                            pane_id.clone(),
                                                            action.clone(),
                                                            Some(tab_id.clone()),
                                                        ));
                                                    }
                                                }
                                            },
                                            "{tab.label}"
                                        }
                                    }
                                }
                            },
                            AppPaneWidget::SearchBox { id, placeholder, action, value } => rsx! {
                                input {
                                    key: "{widget_key}",
                                    "data-app-pane-input": "{id}",
                                    "data-yggui-field": "true",
                                    style: "{field_style}",
                                    r#type: "text",
                                    placeholder: "{placeholder}",
                                    initial_value: "{value}",
                                    oninput: {
                                        let id = id.clone();
                                        let on_app_pane_value = on_app_pane_value.clone();
                                        move |evt: FormEvent| on_app_pane_value.call((id.clone(), evt.value()))
                                    },
                                    onkeydown: {
                                        let (pane_id, action) = (pane_id.clone(), action.clone());
                                        let on_app_pane_action = on_app_pane_action.clone();
                                        move |evt: KeyboardEvent| {
                                            // Rail fields own their keys: a key-capture
                                            // document pane must never see chords typed
                                            // into a rail filter (the buffers search
                                            // box stays native under capture).
                                            evt.stop_propagation();
                                            if evt.key() == Key::Enter && !action.is_empty() {
                                                on_app_pane_action.call((pane_id.clone(), action.clone(), None));
                                            }
                                        }
                                    },
                                }
                            },
                            AppPaneWidget::TextInput { id, label, placeholder, value, action, secret, multiline, rows, actions, stored, .. } => {
                                // A field that HOLDS something it is not showing
                                // draws mask dots — decorative, fixed length, and
                                // a PLACEHOLDER, so nothing about them can be
                                // typed over, submitted or read back. Once a
                                // value is present there is nothing left to mask.
                                // Dots when the app said nothing else. A stored
                                // field that DECLARES a placeholder gets it —
                                // masking a notes box would be theatre, and the
                                // app knows which of its fields is a secret.
                                let masked = stored && value.is_empty() && placeholder.is_empty();
                                let placeholder = if masked {
                                    APP_PANE_FIELD_MASK.to_string()
                                } else {
                                    placeholder.clone()
                                };
                                // The verbs sit ON the box, so the box gives up
                                // the room. Emitted ALWAYS (the no-action case is
                                // the base padding), never conditionally — a
                                // dropped `padding-right` would never clear.
                                let field_box = format!(
                                    "{field_style} padding-right:{}px;",
                                    11 + actions.len() * 22
                                );
                                rsx! {
                                div {
                                    key: "{widget_key}",
                                    style: "display:flex; flex-direction:column; gap:5px; min-width:0;",
                                    if !label.is_empty() {
                                        div { style: "{field_label_style}", "{label}" }
                                    }
                                    div {
                                        style: "position:relative; display:flex; align-items:stretch; min-width:0;",
                                        // A masked textarea is nonsense, so `secret`
                                        // always wins and forces a single-line input.
                                        if multiline && !secret {
                                            textarea {
                                                "data-app-pane-input": "{id}",
                                                "data-yggui-field": "true",
                                                "data-yggui-field-stored": if masked { "true" } else { "false" },
                                                // `resize:vertical` is the drag-to-expand handle;
                                                // `field_style` gives it the same skin as an input.
                                                style: "{field_box} resize:vertical; min-height:1.6em;",
                                                rows: "{rows.max(1)}",
                                                placeholder: "{placeholder}",
                                                initial_value: "{value}",
                                                oninput: {
                                                    let id = id.clone();
                                                    let on_app_pane_value = on_app_pane_value.clone();
                                                    move |evt: FormEvent| on_app_pane_value.call((id.clone(), evt.value()))
                                                },
                                                onkeydown: move |evt: KeyboardEvent| {
                                                    // Rail fields own their keys (see the
                                                    // search box above): no bubbling into a
                                                    // key-capture document pane's channel.
                                                    evt.stop_propagation();
                                                },
                                            }
                                        } else {
                                            input {
                                                "data-app-pane-input": "{id}",
                                                "data-yggui-field": "true",
                                                "data-yggui-field-stored": if masked { "true" } else { "false" },
                                                style: "{field_box}",
                                                r#type: if secret { "password" } else { "text" },
                                                placeholder: "{placeholder}",
                                                initial_value: "{value}",
                                                oninput: {
                                                    let id = id.clone();
                                                    let on_app_pane_value = on_app_pane_value.clone();
                                                    move |evt: FormEvent| on_app_pane_value.call((id.clone(), evt.value()))
                                                },
                                                onkeydown: {
                                                    let (pane_id, action) = (pane_id.clone(), action.clone());
                                                    let on_app_pane_action = on_app_pane_action.clone();
                                                    move |evt: KeyboardEvent| {
                                                        // Rail fields own their keys (see the
                                                        // search box above): no bubbling into a
                                                        // key-capture document pane's channel.
                                                        evt.stop_propagation();
                                                        if evt.key() == Key::Enter && !action.is_empty() {
                                                            on_app_pane_action.call((pane_id.clone(), action.clone(), None));
                                                        }
                                                    }
                                                },
                                            }
                                        }
                                        // The field's own verbs — Bitwarden's eye
                                        // and copy, on the box they act on. Each
                                        // fires with the FIELD's id as its value,
                                        // so the app needs no second encoding of
                                        // "which field".
                                        div {
                                            style: app_pane_field_action_bar_style(multiline && !secret),
                                            for field_action in actions.iter().cloned() {
                                                button {
                                                    key: "{field_action.action}",
                                                    r#type: "button",
                                                    "data-app-pane-field-action": "{field_action.action}",
                                                    "data-yggui-field-action": "true",
                                                    style: app_pane_field_action_button_style(palette),
                                                    title: "{field_action.title}",
                                                    onclick: {
                                                        let (pane_id, action, field_id) =
                                                            (pane_id.clone(), field_action.action.clone(), id.clone());
                                                        let on_app_pane_action = on_app_pane_action.clone();
                                                        move |_| on_app_pane_action.call((
                                                            pane_id.clone(),
                                                            action.clone(),
                                                            Some(field_id.clone()),
                                                        ))
                                                    },
                                                    // An in-field action names its mark the same
                                                    // way a footer button does. Rendering the raw
                                                    // label here printed `icon:copy` / `icon:eye` /
                                                    // `icon:dice` as TEXT inside the box, on top of
                                                    // the value — one slot that forgot the shared
                                                    // resolver every other slot uses.
                                                    {shell_glyph(&field_action.label, 14)}
                                                }
                                            }
                                        }
                                    }
                                }
                                }
                            },
                            AppPaneWidget::NumberInput { id, label, value, min, max } => rsx! {
                                div {
                                    key: "{widget_key}",
                                    style: "display:flex; align-items:center; gap:6px;",
                                    if !label.is_empty() {
                                        div { style: "{field_label_style}", "{label}" }
                                    }
                                    input {
                                        "data-app-pane-input": "{id}",
                                        "data-yggui-field": "true",
                                        style: "{field_style} max-width:88px;",
                                        r#type: "number",
                                        min: "{min}",
                                        max: "{max}",
                                        initial_value: "{value}",
                                        oninput: {
                                            let id = id.clone();
                                            let on_app_pane_value = on_app_pane_value.clone();
                                            move |evt: FormEvent| on_app_pane_value.call((id.clone(), evt.value()))
                                        },
                                        onkeydown: move |evt: KeyboardEvent| {
                                            // Rail fields own their keys (see the search
                                            // box above): no bubbling into a key-capture
                                            // document pane's channel.
                                            evt.stop_propagation();
                                        },
                                    }
                                }
                            },
                            // A contributed toggle wears the SAME switch as yggterm's own
                            // settings (`InlineSettingsToggleRow`) — one toggle vocabulary
                            // across the app, no checkbox anywhere. It is a button, not an
                            // <input type=checkbox>: the whole row is the hit target, and
                            // the value the app receives is the string it declared values
                            // in ("true"/"false"), unchanged from the checkbox era.
                            AppPaneWidget::Toggle { id, label, action, value } => rsx! {
                                button {
                                    key: "{widget_key}",
                                    r#type: "button",
                                    "data-app-pane-toggle": "{id}",
                                    "data-app-pane-toggle-enabled": if value { "true" } else { "false" },
                                    aria_pressed: if value { "true" } else { "false" },
                                    style: app_pane_toggle_row_style(palette, value),
                                    onclick: {
                                        let (pane_id, action, id) = (pane_id.clone(), action.clone(), id.clone());
                                        let on_app_pane_action = on_app_pane_action.clone();
                                        let on_app_pane_value = on_app_pane_value.clone();
                                        move |_| {
                                            let next = (!value).to_string();
                                            on_app_pane_value.call((id.clone(), next.clone()));
                                            if !action.is_empty() {
                                                on_app_pane_action.call((pane_id.clone(), action.clone(), Some(next.clone())));
                                            }
                                        }
                                    },
                                    div {
                                        style: "flex:1 1 auto; min-width:0; text-align:left; pointer-events:none; text-wrap:pretty;",
                                        "{label}"
                                    }
                                    div {
                                        style: inline_toggle_track_style(palette, value),
                                        div { style: inline_toggle_thumb_style(value) }
                                    }
                                }
                            },
                            AppPaneWidget::Button { id, label, action, primary, danger, title } => rsx! {
                                button {
                                    key: "{widget_key}",
                                    "data-app-pane-button": "{id}",
                                    "data-app-pane-button-danger": if danger { "true" } else { "false" },
                                    title: "{title}",
                                    style: if danger {
                                        danger_button_style.clone()
                                    } else if primary {
                                        primary_button_style.clone()
                                    } else {
                                        plain_button_style.clone()
                                    },
                                    onclick: {
                                        let (pane_id, action) = (pane_id.clone(), action.clone());
                                        let on_app_pane_action = on_app_pane_action.clone();
                                        move |_| on_app_pane_action.call((pane_id.clone(), action.clone(), None))
                                    },
                                    {shell_glyph(&label, 13)}
                                }
                            },
                            AppPaneWidget::ListRow { id, title, subtitle, icon, status, selected, row_action, actions, menu, rename, reorder_action, depth, expanded, expand_action } => {
                                // The SHARED row engine (Phase 1): same anatomy
                                // and metrics as the cwdtree rows and WebTabs
                                // rail — whole-row clickable, selected tinted,
                                // tiny trailing actions.
                                let clickable = !row_action.is_empty();
                                let has_menu = !menu.is_empty();
                                // Renaming replaces the row BODY in place — the
                                // cwd tree's "Rename session" shape, which is
                                // what the user asked contributed rows to match.
                                // The draft lives under `rename:<row id>` so a
                                // switch of which row is being renamed remounts
                                // the field with the new name (value epochs).
                                if let Some(rename) = rename {
                                    let draft_id = format!("rename:{id}");
                                    return rsx! {
                                        div {
                                            key: "{widget_key}",
                                            "data-app-pane-row-rename": "{id}",
                                            style: format!(
                                                "{}display:flex; align-items:center; gap:6px; padding:2px 0;",
                                                if follows_row { "margin-top:-8px; " } else { "" },
                                            ),
                                            input {
                                                "data-app-pane-input": "{draft_id}",
                                                // The SHARED field skin (text_field_css) — it
                                                // used to hand-roll a white fill and a grey
                                                // hairline, which was a light-theme-only box
                                                // that never answered focus.
                                                "data-yggui-field": "true",
                                                style: format!(
                                                    "flex:1; min-width:0; height:29px; border:none; border-radius:10px; \
                                                     color:{}; font-size:12px; font-weight:600; padding:0 10px; \
                                                     outline:none;",
                                                    palette.text
                                                ),
                                                r#type: "text",
                                                placeholder: "{rename.placeholder}",
                                                initial_value: "{rename.value}",
                                                onmounted: move |evt| async move {
                                                    let _ = evt.set_focus(true).await;
                                                },
                                                oninput: {
                                                    let (draft_id, on_app_pane_value) =
                                                        (draft_id.clone(), on_app_pane_value.clone());
                                                    move |evt: FormEvent| {
                                                        on_app_pane_value.call((draft_id.clone(), evt.value()))
                                                    }
                                                },
                                                onkeydown: {
                                                    let (pane_id, apply, cancel, row_id) = (
                                                        pane_id.clone(),
                                                        rename.action.clone(),
                                                        rename.cancel_action.clone(),
                                                        id.clone(),
                                                    );
                                                    let on_app_pane_action = on_app_pane_action.clone();
                                                    move |evt: KeyboardEvent| {
                                                        evt.stop_propagation();
                                                        match evt.key() {
                                                            Key::Enter if !apply.is_empty() => {
                                                                evt.prevent_default();
                                                                on_app_pane_action.call((
                                                                    pane_id.clone(),
                                                                    apply.clone(),
                                                                    Some(row_id.clone()),
                                                                ));
                                                            }
                                                            Key::Escape if !cancel.is_empty() => {
                                                                evt.prevent_default();
                                                                on_app_pane_action.call((
                                                                    pane_id.clone(),
                                                                    cancel.clone(),
                                                                    Some(row_id.clone()),
                                                                ));
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                },
                                                onclick: |evt: MouseEvent| evt.stop_propagation(),
                                            }
                                            if !rename.ai_source.is_empty() {
                                                button {
                                                    "data-app-pane-row-rename-ai": "{id}",
                                                    title: "Use an AI-generated name",
                                                    style: rename_ai_action_button_style(palette),
                                                    // mousedown default would blur the
                                                    // field before the click lands.
                                                    onmousedown: |evt: MouseEvent| {
                                                        evt.prevent_default();
                                                        evt.stop_propagation();
                                                    },
                                                    onclick: {
                                                        let (pane_id, source, row_id) = (
                                                            pane_id.clone(),
                                                            rename.ai_source.clone(),
                                                            id.clone(),
                                                        );
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            queue_app_pane_row_rename_ai_name(
                                                                state,
                                                                pane_id.clone(),
                                                                row_id.clone(),
                                                                source.clone(),
                                                            );
                                                        }
                                                    },
                                                    AiSparkleIcon { size: 12 }
                                                }
                                            }
                                            // The visible way out. Escape alone was
                                            // the only cancel, which is not an
                                            // affordance (user, 2026-07-24).
                                            if !rename.cancel_action.is_empty() {
                                                button {
                                                    "data-app-pane-row-rename-cancel": "{id}",
                                                    title: "Cancel rename",
                                                    style: session_row_action_button_style(palette.muted),
                                                    onmousedown: |evt: MouseEvent| {
                                                        evt.prevent_default();
                                                        evt.stop_propagation();
                                                    },
                                                    onclick: {
                                                        let (pane_id, action, row_id) = (
                                                            pane_id.clone(),
                                                            rename.cancel_action.clone(),
                                                            id.clone(),
                                                        );
                                                        let on_app_pane_action = on_app_pane_action.clone();
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            on_app_pane_action.call((
                                                                pane_id.clone(),
                                                                action.clone(),
                                                                Some(row_id.clone()),
                                                            ));
                                                        }
                                                    },
                                                    "✕"
                                                }
                                            }
                                        }
                                    };
                                }
                                let reorderable = !reorder_action.is_empty();
                                // A row that declared `expanded` is a GROUP: it
                                // draws the disclosure triangle and it has an
                                // inside a drop can land in.
                                let row_is_group = expanded.is_some();
                                let row_expanded = expanded.unwrap_or(true);
                                let (drop_edge, row_is_dragging) = if reorderable {
                                    let shell = state.read();
                                    (
                                        shell.app_pane_row_drop_edge(&pane_id, &id),
                                        shell.app_pane_row_is_dragging(&pane_id, &id),
                                    )
                                } else {
                                    (None, false)
                                };
                                // Every row's leading slot is its status dot —
                                // groups included, so a folder and the rows
                                // under it start their titles at one x. The
                                // disclosure chevron rides the TRAILING
                                // expander slot, which is where the cwd tree
                                // has always put it.
                                let leading_slot: Option<Element> =
                                    app_pane_row_status_dot_style(palette, &status)
                                        .map(|dot| rsx! { span { style: "{dot}" } });
                                let expander_slot: Option<Element> = row_is_group.then(|| {
                                    let (expand_pane, expand_action, expand_row) =
                                        (pane_id.clone(), expand_action.clone(), id.clone());
                                    let on_expand = on_app_pane_action.clone();
                                    rsx! {
                                        button {
                                            "data-app-pane-row-expand": "{expand_row}",
                                            style: row_disclosure_button_style(palette.muted),
                                            title: if row_expanded { "Collapse" } else { "Expand" },
                                            onmousedown: |evt: MouseEvent| evt.stop_propagation(),
                                            onclick: move |evt: MouseEvent| {
                                                evt.stop_propagation();
                                                if expand_action.is_empty() {
                                                    return;
                                                }
                                                on_expand.call((
                                                    expand_pane.clone(),
                                                    expand_action.clone(),
                                                    Some(expand_row.clone()),
                                                ));
                                            },
                                            RowDisclosureChevron { expanded: row_expanded }
                                        }
                                    }
                                });
                                // A row that DECLARED itself a group and named
                                // no icon of its own gets the tree's folder
                                // glyph, filled or outline by its own
                                // `expanded`. The app said "I hold rows"; what
                                // that LOOKS like is yggterm's to answer, and
                                // it must be the one answer the cwd tree gives.
                                let icon_slot: Option<Element> = if !icon.is_empty() {
                                    Some(app_pane_row_icon(&icon))
                                } else if row_is_group {
                                    Some(rsx! { RowFolderIcon { expanded: row_expanded } })
                                } else {
                                    None
                                };
                                rsx! {
                                    div {
                                        key: "{widget_key}",
                                        // Drag state is read here (not inside
                                        // SessionStyleRow) so the drop line sits
                                        // on the row's OUTER box — an inset shadow
                                        // on the inner row would be clipped by its
                                        // own border radius.
                                        "data-app-pane-row-reorderable": if reorderable { "1" } else { "0" },
                                        "data-app-pane-row-drop-edge": match drop_edge {
                                            Some(DragDropPlacement::Before) => "before",
                                            Some(DragDropPlacement::Into) => "into",
                                            Some(DragDropPlacement::After) => "after",
                                            None => "",
                                        },
                                        "data-app-pane-row-dragging": if row_is_dragging { "1" } else { "0" },
                                        // THREE properties, and all three are
                                        // written in EVERY state — `margin-top`
                                        // included. An `else { "" }` here left
                                        // the -8px collapse stuck on a row that
                                        // stopped following one, for the same
                                        // property-by-property reason the drop
                                        // line leaked.
                                        style: format!(
                                            "margin-top:{}; {}",
                                            if follows_row { "-8px" } else { "0px" },
                                            app_pane_row_drop_line_style(drop_edge, palette.accent),
                                        ),
                                        // A reorder drag is a MOUSE gesture, like
                                        // the cwd tree's: HTML5 dnd never fires
                                        // reliably inside this webview, and the
                                        // tree already proved the pointer path.
                                        onmousedown: {
                                            let (pane_id, row_id) = (pane_id.clone(), id.clone());
                                            // The ghost says what is moving, so the
                                            // row's own title rides the gesture.
                                            let row_label = title.clone();
                                            let mut state = state;
                                            move |evt: MouseEvent| {
                                                if !reorderable
                                                    || evt.trigger_button() != Some(MouseButton::Primary)
                                                {
                                                    return;
                                                }
                                                // ARM only. The press becomes a drag
                                                // when it travels, so a plain click
                                                // stays a plain click.
                                                let pointer = evt.client_coordinates();
                                                state.with_mut_counted(|shell| {
                                                    shell.arm_app_pane_row_drag(
                                                        pane_id.clone(),
                                                        row_id.clone(),
                                                        row_label.clone(),
                                                        (pointer.x, pointer.y),
                                                    );
                                                });
                                            }
                                        },
                                        onmousemove: {
                                            let (pane_id, row_id) = (pane_id.clone(), id.clone());
                                            let row_title = title.clone();
                                            let expand_action = expand_action.clone();
                                            let on_spring = on_app_pane_action.clone();
                                            let mut state = state;
                                            move |evt: MouseEvent| {
                                                if !reorderable {
                                                    return;
                                                }
                                                // No button held ⇒ this is a hover,
                                                // not a drag. Without this gate a
                                                // stuck gesture re-armed a drop
                                                // target under a moving pointer and
                                                // the next ordinary click committed
                                                // a reorder nobody asked for. The
                                                // cwd tree's root handler has always
                                                // checked exactly this.
                                                if !evt.held_buttons().contains(MouseButton::Primary) {
                                                    return;
                                                }
                                                let pointer = evt.client_coordinates();
                                                // Before / inside / after, from the
                                                // ONE band rule. A GROUP row has an
                                                // inside; a leaf keeps two bands.
                                                let placement = row_drop_placement_for_offset(
                                                    evt.element_coordinates().y,
                                                    row_is_group,
                                                );
                                                let sprung = state.with_mut_counted(|shell| {
                                                    if !shell.maybe_begin_app_pane_row_drag((
                                                        pointer.x, pointer.y,
                                                    )) {
                                                        return None;
                                                    }
                                                    // SPRING-LOAD from the shared
                                                    // engine's dwell. yggterm decides
                                                    // WHEN; the app owns what
                                                    // "expand" means, so the answer
                                                    // is its own `expand_action`.
                                                    shell.hover_app_pane_row_drop_group(
                                                        &pane_id,
                                                        &row_id,
                                                        &row_title,
                                                        placement,
                                                        row_is_group && !row_expanded,
                                                    )
                                                });
                                                if sprung.is_some() && !expand_action.is_empty() {
                                                    on_spring.call((
                                                        pane_id.clone(),
                                                        expand_action.clone(),
                                                        Some(row_id.clone()),
                                                    ));
                                                }
                                            }
                                        },
                                        onmouseup: {
                                            let (pane_id, action, rows, reorderable_ids) = (
                                                pane_id.clone(),
                                                reorder_action.clone(),
                                                row_tree.clone(),
                                                reorderable_row_ids.clone(),
                                            );
                                            let on_app_pane_reorder = on_app_pane_reorder.clone();
                                            let mut state = state;
                                            move |_: MouseEvent| {
                                                let dropped = state
                                                    .with_mut(|shell| shell.take_app_pane_row_drop(&rows));
                                                let Some((drop_pane, moved, parent, order)) = dropped
                                                else {
                                                    return;
                                                };
                                                if action.is_empty() || drop_pane != pane_id {
                                                    return;
                                                }
                                                // `values["order"]` has always been the
                                                // REORDERABLE rows; a fixed row the app
                                                // pinned must not appear in a list the
                                                // app is about to adopt as its own.
                                                let order: Vec<String> = order
                                                    .into_iter()
                                                    .filter(|id| reorderable_ids.iter().any(|row| row == id))
                                                    .collect();
                                                on_app_pane_reorder.call((
                                                    pane_id.clone(),
                                                    action.clone(),
                                                    moved,
                                                    parent,
                                                    order,
                                                ));
                                            }
                                        },
                                        // A drag that leaves the list entirely is
                                        // abandoned, not applied to whatever row
                                        // the pointer happens to return over.
                                        onmouseleave: {
                                            let mut state = state;
                                            move |_: MouseEvent| {
                                                state.with_mut_counted(|shell| {
                                                    shell.forget_row_drag_target();
                                                });
                                            }
                                        },
                                        // Right-click opens the app-declared row
                                        // menu as a GUI-owned floating overlay.
                                        oncontextmenu: {
                                            let (pane_id, row_id, row_title, menu) =
                                                (pane_id.clone(), id.clone(), title.clone(), menu.clone());
                                            let mut state = state;
                                            move |evt: MouseEvent| {
                                                if !has_menu {
                                                    return;
                                                }
                                                evt.prevent_default();
                                                let pos = evt.client_coordinates();
                                                state.with_mut_counted(|shell| {
                                                    shell.open_app_pane_context_menu(
                                                        pane_id.clone(),
                                                        row_id.clone(),
                                                        row_title.clone(),
                                                        menu.clone(),
                                                        (pos.x, pos.y),
                                                    );
                                                });
                                            }
                                        },
                                    SessionStyleRow {
                                        "data-app-pane-row": "{id}",
                                        "data-app-pane-row-group": if row_is_group { "1" } else { "0" },
                                        "data-app-pane-row-expanded": if row_is_group && row_expanded { "1" } else { "0" },
                                        "data-app-pane-row-depth": "{depth}",
                                        density: SessionRowDensity::Rail,
                                        // The pane's tree, drawn by the SHARED row
                                        // engine's indent — the reason `depth` is a
                                        // schema field and not a component change.
                                        depth,
                                        selected,
                                        // ONE dim for a dragged row. The rail
                                        // used to fade its own outer box by a
                                        // hand-written amount while every other
                                        // row family took the shared engine's.
                                        dimmed: row_is_dragging,
                                        text_color: palette.text.to_string(),
                                        selected_bg: palette.accent_soft.to_string(),
                                        label: title.clone(),
                                        subtitle: (!subtitle.is_empty()).then(|| subtitle.clone()),
                                        subtitle_color: Some(palette.muted.to_string()),
                                        // The status slot the native rows have
                                        // always had. The app names the class,
                                        // yggterm paints it from the shared
                                        // traffic-signal vocabulary.
                                        // A GROUP's leading slot is its disclosure
                                        // chevron; a leaf's is its status dot. Same
                                        // slot either way, so a folder and the rows
                                        // under it start their titles at one x.
                                        dot: leading_slot,
                                        icon: icon_slot,
                                        // The cwdtree's icon rule: muted at
                                        // rest, text color on the selected row.
                                        icon_color: Some(
                                            if selected { palette.text } else { palette.muted }
                                                .to_string(),
                                        ),
                                        expander: expander_slot,
                                        onclick: clickable.then(|| {
                                            let (pane_id, action, row_id) =
                                                (pane_id.clone(), row_action.clone(), id.clone());
                                            let on_app_pane_action = on_app_pane_action.clone();
                                            let mut state = state;
                                            EventHandler::new(move |_| {
                                                // A drag's own release is also a
                                                // click. Moving a row must not
                                                // also open it — the cwd tree
                                                // has always swallowed this.
                                                if state.with_mut_counted(|shell| {
                                                    shell.consume_suppressed_row_click()
                                                }) {
                                                    return;
                                                }
                                                on_app_pane_action.call((
                                                    pane_id.clone(),
                                                    action.clone(),
                                                    Some(row_id.clone()),
                                                ));
                                            })
                                        }),
                                        actions: rsx! {
                                            for row_action in actions.iter().cloned() {
                                                button {
                                                    key: "{row_action.action}",
                                                    "data-app-pane-row-action": "{row_action.action}",
                                                    style: session_row_action_button_style(palette.muted),
                                                    title: "{row_action.title}",
                                                    // Pressing a row's ✕ must not
                                                    // arm the row's drag — the
                                                    // rename buttons already guard
                                                    // mousedown for the same reason.
                                                    onmousedown: |evt: MouseEvent| evt.stop_propagation(),
                                                    onclick: {
                                                        let (pane_id, action, row_id) =
                                                            (pane_id.clone(), row_action.action.clone(), id.clone());
                                                        let on_app_pane_action = on_app_pane_action.clone();
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            on_app_pane_action.call((
                                                                pane_id.clone(),
                                                                action.clone(),
                                                                Some(row_id.clone()),
                                                            ))
                                                        }
                                                    },
                                                    // `icon:<name>` draws the shell's own stroked
                                                    // mark; anything else is still the character
                                                    // the app sent.
                                                    {shell_glyph(&row_action.label, 13)}
                                                }
                                            }
                                        },
                                    }
                                    }
                                }
                            },
                            AppPaneWidget::Markdown { id, source, .. } => rsx! {
                                div {
                                    key: "{widget_key}",
                                    "data-app-pane-markdown": "{id}",
                                    style: "font-size:11px; min-width:0; overflow-wrap:anywhere;",
                                    // The RAIL surface: reading typography is
                                    // for document-scale surfaces, and this
                                    // pane is 300px wide at 11px.
                                    {markdown_widget_body(
                                        &source,
                                        &DocTheme::from_terminal(&snapshot.terminal_palette),
                                        ProseTokens::rail(),
                                    )}
                                }
                            },
                            // A RibbonBar in a BODY is a schema mistake — the
                            // ribbon region above the card owns it. Render
                            // nothing rather than a hole.
                            AppPaneWidget::RibbonBar { .. } => rsx! {},
                        }
                        }
                    }
                    }
                    }
                } else {
                    div { style: "{muted_style}", "Loading…" }
                }
            }
            }
        }
        // The rail STATUS FOOTER: pinned under the scroll area, separated, for
        // the app's status-bar data (schema `footer`). Subset vocabulary —
        // label / toggle / button — documented in the libyggterm-surfaces skill.
        if !footer_widgets.is_empty() {
            {
            let first_footer_icon_button = app_pane_first_icon_button(&footer_widgets);
            rsx! {
            div {
                "data-app-pane-footer": "{pane_id}",
                style: format!(
                    "flex:0 0 auto; display:flex; align-items:center; gap:8px; flex-wrap:wrap; \
                     padding:8px 16px 10px 16px; border-top:1px solid rgba(127,127,127,0.25); \
                     font-size:10px; color:{};",
                    palette.muted
                ),
                for (index, widget) in footer_widgets.iter().cloned().enumerate() {
                    {
                    let widget_key = widget.key(index, &value_epochs);
                    // WHERE THE TRAILING CLUSTER STARTS. Bitwarden's View Login
                    // bar is the shape: the named verb (Edit) at the leading
                    // edge, the icon-only ones (archive, delete) pushed to the
                    // trailing edge. An icon-only button is one whose whole
                    // label is an `icon:` token, so the app expresses the layout
                    // by choosing an icon — there is no second "align" field to
                    // disagree with it.
                    let starts_trailing_cluster = index == first_footer_icon_button;
                    match widget {
                        AppPaneWidget::Label { text, .. } => rsx! {
                            span {
                                key: "{widget_key}",
                                style: format!("color:{}; white-space:nowrap;", palette.muted),
                                "{text}"
                            }
                        },
                        AppPaneWidget::Toggle { id, label, action, value } => rsx! {
                            button {
                                key: "{widget_key}",
                                "data-app-pane-footer-toggle": "{id}",
                                style: if value {
                                    format!(
                                        "margin-left:auto; padding:3px 9px; border:0; border-radius:7px; \
                                         background:{}; color:#fff; font-size:10px; font-weight:700; cursor:pointer;",
                                        palette.accent
                                    )
                                } else {
                                    format!(
                                        "margin-left:auto; padding:3px 9px; border:1px solid rgba(127,127,127,0.35); \
                                         border-radius:7px; background:transparent; color:{}; font-size:10px; \
                                         font-weight:600; cursor:pointer;",
                                        palette.text
                                    )
                                },
                                onclick: {
                                    let action = action.clone();
                                    let next = (!value).to_string();
                                    let pane_id = pane_id.clone();
                                    move |_| on_app_pane_action.call((pane_id.clone(), action.clone(), Some(next.clone())))
                                },
                                "{label}"
                            }
                        },
                        // A footer button is an ACTION BAR button: pinned under
                        // the scroll area, always reachable. `primary` reaches it
                        // for the same reason it reaches the body — a form whose
                        // Save scrolls away is a form with no Save.
                        AppPaneWidget::Button { id, label, action, primary, danger, title } => rsx! {
                            button {
                                key: "{widget_key}",
                                "data-app-pane-footer-button": "{id}",
                                "data-app-pane-footer-button-primary": if primary { "true" } else { "false" },
                                "data-app-pane-footer-button-danger": if danger { "true" } else { "false" },
                                title: "{title}",
                                style: app_pane_footer_button_style(
                                    palette,
                                    primary,
                                    danger,
                                    ShellIcon::from_token(&label).is_some(),
                                    starts_trailing_cluster,
                                ),
                                onclick: {
                                    let action = action.clone();
                                    let pane_id = pane_id.clone();
                                    move |_| on_app_pane_action.call((pane_id.clone(), action.clone(), None))
                                },
                                {shell_glyph(&label, 15)}
                            }
                        },
                        // Anything else in a footer is a schema mistake; render
                        // nothing rather than a hole the app can't see.
                        _ => rsx! {},
                    }
                    }
                }
            }
            }
            }
        }
    }
}

/// Where a footer's TRAILING cluster begins: the index of its first icon-only
/// button, or `usize::MAX` when it has none (every button keeps the leading
/// edge, exactly as before this existed).
fn app_pane_first_icon_button(footer: &[AppPaneWidget]) -> usize {
    footer
        .iter()
        .position(|widget| {
            matches!(widget, AppPaneWidget::Button { label, .. }
                if ShellIcon::from_token(label).is_some())
        })
        .unwrap_or(usize::MAX)
}

/// A pinned action-bar button.
///
/// FOUR shapes from ONE owner, because a bar that mixes hand-written styles ends
/// up with buttons of different heights sitting next to each other:
///
/// * `primary` — filled in the accent. The act the bar exists for (Save, Edit).
/// * `danger` — the product's one red ([`DESTRUCTIVE_RED`]), outlined, filled
///   faintly. Never also primary: a destructive act is not the default one.
/// * icon-only — a square, so the mark is centred rather than sitting in a
///   text box that is wider than it is tall.
/// * plain — outlined in the accent at low strength.
fn app_pane_footer_button_style(
    palette: Palette,
    primary: bool,
    danger: bool,
    icon_only: bool,
    starts_trailing_cluster: bool,
) -> String {
    let (border, background, color, weight) = if danger {
        (
            format!("1px solid color-mix(in srgb, {DESTRUCTIVE_RED} 38%, transparent)"),
            format!("color-mix(in srgb, {DESTRUCTIVE_RED} 10%, transparent)"),
            DESTRUCTIVE_RED.to_string(),
            "700",
        )
    } else if primary {
        (
            "0".to_string(),
            palette.accent.to_string(),
            "#fff".to_string(),
            "700",
        )
    } else {
        (
            format!(
                "1px solid color-mix(in srgb, {} 30%, rgba(127,127,127,0.34))",
                palette.accent
            ),
            "transparent".to_string(),
            palette.text.to_string(),
            "600",
        )
    };
    format!(
        "display:inline-flex; align-items:center; justify-content:center; \
         {sizing} border:{border}; border-radius:9px; background:{background}; \
         color:{color}; font-size:11px; font-weight:{weight}; cursor:pointer; \
         margin-left:{margin};",
        // A square for a mark, a pill for words.
        sizing = if icon_only {
            "width:30px; height:30px; padding:0;"
        } else {
            "min-height:30px; padding:6px 13px;"
        },
        margin = if starts_trailing_cluster { "auto" } else { "0" },
    )
}

#[component]
fn MetadataRailBody(
    snapshot: SharedSnapshot,
    on_daemon_hot_restart: EventHandler<MouseEvent>,
) -> Element {
    let session = snapshot.active_session.clone();
    let palette = snapshot.palette;
    let daemon = snapshot.daemon.clone();
    rsx! {
        RailHeader { title: "Session Metadata".to_string(), color: palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
            if let Some(session) = session {
                {render_session_metadata(&session, palette)}
            } else {
                MetadataGroup {
                    title: "Session".to_string(),
                    entries: vec![SessionMetadataEntry {
                        label: "State",
                        value: "No session selected".to_string(),
                    }],
                    palette,
                }
            }
            // The CLIENT version is always knowable — it is this process. Rendering it
            // only when the daemon answers meant the one moment you most need to know
            // which build you are running (the daemon is unreachable / not answering) is
            // exactly the moment the rail went blank. So Client renders unconditionally,
            // and a missing daemon becomes a VISIBLE "unreachable" row rather than an
            // absent section that reads as "nothing to see here".
            MetadataGroup {
                title: "Client".to_string(),
                entries: client_metadata_entries(daemon.as_ref()),
                palette,
            }
            if let Some(daemon) = daemon {
                DaemonMetadataGroup { daemon, palette, on_daemon_hot_restart }
            } else {
                MetadataGroup {
                    title: "Daemon".to_string(),
                    entries: vec![SessionMetadataEntry {
                        label: "Status",
                        value: "not answering — this window has no daemon status yet".to_string(),
                    }],
                    palette,
                }
            }
            }
        }
    }
}

/// How the serving daemon's version stands against this window's.
///
/// Four answers, because the three the panel used to give could not tell "newer"
/// from "older" — it only knew "different". See the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonVersionRank {
    Newer,
    Older,
    Same,
    /// One of the two did not parse as a dotted triple. Never guess a direction
    /// from an unparseable version — that is how the inverted label happened.
    Unknown,
}

fn version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.trim().parse::<u64>().ok()?;
    let minor = parts.next()?.trim().parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").trim().parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn daemon_version_rank(daemon_version: &str, client_version: &str) -> DaemonVersionRank {
    let (Some(daemon), Some(client)) = (
        version_triple(daemon_version),
        version_triple(client_version),
    ) else {
        return DaemonVersionRank::Unknown;
    };
    match daemon.cmp(&client) {
        std::cmp::Ordering::Greater => DaemonVersionRank::Newer,
        std::cmp::Ordering::Less => DaemonVersionRank::Older,
        std::cmp::Ordering::Equal => DaemonVersionRank::Same,
    }
}

/// The Client group: which build THIS window is, and whether the daemon agrees.
/// Separate from the daemon group because it must survive the daemon being silent.
fn client_metadata_entries(daemon: Option<&DaemonPanelStatus>) -> Vec<SessionMetadataEntry> {
    let client_version = current_version();
    let value = match daemon {
        Some(daemon) if daemon.version == client_version => client_version,
        Some(daemon) => format!("{client_version} · daemon is on {}", daemon.version),
        None => format!("{client_version} · daemon not answering"),
    };
    vec![SessionMetadataEntry {
        label: "Version",
        value,
    }]
}
/// The daemon group: who is actually serving this window, and why a restart is (or
/// is not) being deferred. See [`DaemonPanelStatus`] for why this exists.
#[component]
fn DaemonMetadataGroup(
    daemon: DaemonPanelStatus,
    palette: Palette,
    on_daemon_hot_restart: EventHandler<MouseEvent>,
) -> Element {
    // The Client group is rendered by the parent, immediately above this one, so both
    // versions read in a single glance and a mismatch is something the user trips over
    // rather than goes looking for. See [[finding-stale-daemon-trap]].
    let versions_agree = daemon.client_version == daemon.version;
    // ⛔ WHICH ONE IS OLDER IS A QUESTION THIS USED TO SKIP. The arm below was
    // `(false, false) => "{version} · older than this client"`, on the bare fact
    // that the strings DIFFER — so a daemon NEWER than the window reported itself
    // as older. reported live 2026-08-10 with the numbers on screen: client
    // 3.0.89, daemon 3.0.91, label *"3.0.91 · older than this client"*. A panel
    // that inverts the one comparison it exists to make is worse than a blank one.
    //
    // ⛔ And `hot_restart_pending` no longer outranks the comparison. It meant the
    // panel offered an upgrade whenever the daemon merely THOUGHT a swap was
    // pending — the requirement, *"we daemon update when there is no daemon to update"*.
    // A pending flag is only worth saying when there is somewhere newer to go.
    let daemon_rank = daemon_version_rank(&daemon.version, &daemon.client_version);
    let mut entries = vec![
        SessionMetadataEntry {
            label: "Version",
            value: match daemon_rank {
                DaemonVersionRank::Newer => {
                    format!("{} · newer than this client", daemon.version)
                }
                DaemonVersionRank::Older if daemon.hot_restart_pending => {
                    format!("{} · newer build on disk", daemon.version)
                }
                DaemonVersionRank::Older => {
                    format!("{} · older than this client", daemon.version)
                }
                DaemonVersionRank::Same if daemon.hot_restart_pending => {
                    format!("{} · newer build on disk", daemon.version)
                }
                DaemonVersionRank::Same => daemon.version.clone(),
                DaemonVersionRank::Unknown if versions_agree => daemon.version.clone(),
                DaemonVersionRank::Unknown => {
                    format!("{} · differs from this client", daemon.version)
                }
            },
        },
        SessionMetadataEntry {
            label: "Uptime",
            value: friendly_duration_ms(daemon.uptime_ms),
        },
        SessionMetadataEntry {
            label: "PID",
            value: daemon.pid.to_string(),
        },
        SessionMetadataEntry {
            label: "Sessions",
            value: format!(
                "{} owned · {} total · {} preserved",
                daemon.owned_sessions, daemon.total_sessions, daemon.preserved_owners
            ),
        },
        // The transparency the whole group exists for: the daemon's OWN words for why
        // it is holding off, not a guess reconstructed in the UI.
        //
        // ⚠ A blocker only DEFERS something when a swap is actually wanted. The
        // daemon computes its blocker list unconditionally, and once a plain
        // shell became a permanent blocker (3.0.81) an idle, up-to-date daemon
        // started reporting "deferred — …" forever, on a machine with nothing to
        // deploy. Read `hot_restart_pending` first; it is the field that says
        // whether there is a swap to be deferred at all.
        SessionMetadataEntry {
            label: "Restart",
            value: match (&daemon.hot_restart_block_reason, daemon.hot_restart_pending) {
                (Some(reason), true) => format!("deferred — {reason}"),
                (None, true) => "ready — newer build waiting".to_string(),
                (_, false) => "nothing pending".to_string(),
            },
        },
    ];
    // Expanding the group lists EVERY blocker. The summary above names one; a swap pinned
    // by three agents used to read as an endless unexplained wait, because clearing the
    // named session just surfaced another. Now the user can see the whole set, and what
    // each one is doing, so "when will this land" has an answer.
    if daemon.hot_restart_pending {
        for blocker in &daemon.hot_restart_blockers {
            entries.push(SessionMetadataEntry {
                label: "Blocking",
                value: match (blocker.kind.as_str(), blocker.idle_ms) {
                    (yggterm_server::HOT_RESTART_BLOCKER_WORKING, _) => {
                        format!("{} — working now", blocker.session_key)
                    }
                    // A permanent blocker must never be drawn as a countdown. The
                    // idle time is real and irrelevant: waiting is what would let
                    // the daemon destroy this shell, not what would release it.
                    (yggterm_server::HOT_RESTART_BLOCKER_NOT_RESTORABLE, _) => format!(
                        "{} — a plain terminal; it stays open, so this daemon stays",
                        blocker.session_key
                    ),
                    (_, Some(idle_ms)) => format!(
                        "{} — active {} ago (idle window {})",
                        blocker.session_key,
                        friendly_duration_ms(idle_ms),
                        friendly_duration_ms(blocker.threshold_ms),
                    ),
                    (_, None) => format!("{} — recently active", blocker.session_key),
                },
            });
        }
    }
    rsx! {
        MetadataGroup { title: "Daemon".to_string(), entries, palette }
        button {
            "data-daemon-hot-restart-button": "1",
            // A real interactable is a keyboard target (§12.2): the old
            // `daemon-control` exemption is dissolved and this button is
            // derived by the overlay-open walk.
            style: format!(
                "display:inline-flex; align-items:center; justify-content:center; min-height:30px; \
                 margin-bottom:8px; padding:0 12px; border:none; border-radius:10px; background:{}; \
                 color:{}; font-size:11px; font-weight:700; box-shadow: inset 0 0 0 1px {}; \
                 cursor:pointer; white-space:nowrap;",
                palette.panel_alt,
                palette.text,
                chrome_chip_border(palette),
            ),
            onclick: move |evt| on_daemon_hot_restart.call(evt),
            "Hot-restart daemon"
        }
    }
}
/// Friendly, user-facing label for a session kind. The raw enum names
/// (`ClaudeCode`, `CodexLiteLlm`) are an implementation detail.
fn friendly_session_kind(kind: SessionKind) -> &'static str {
    if let Some(descriptor) = yggterm_core::agent_cli::agent_cli_descriptor(kind) {
        // ⚠ `Codex · LiteLLM` keeps its middot: this string is the metadata
        // rail's, and the rail reads as a sentence where the descriptor's
        // hyphenated form reads as an identifier.
        return match kind {
            SessionKind::CodexLiteLlm => "Codex · LiteLLM",
            _ => descriptor.display_name,
        };
    }
    match kind {
        SessionKind::Shell => "Shell",
        SessionKind::SshShell => "SSH Shell",
        SessionKind::Document => "Document",
        _ => "Shell",
    }
}
fn friendly_launch_phase(phase: TerminalLaunchPhase) -> &'static str {
    match phase {
        TerminalLaunchPhase::Queued => "queued",
        TerminalLaunchPhase::BridgePending => "starting",
        TerminalLaunchPhase::RemoteBootstrap => "bootstrapping",
        TerminalLaunchPhase::Running => "running",
        TerminalLaunchPhase::Failed => "launch failed",
    }
}
/// "How to connect to that PTY" — the product's core value, surfaced verbatim.
/// The daemon already computes an authoritative `Restore` handoff command
/// (`ssh <machine> 'yggterm server remote resume-… <uuid> --require-existing'`);
/// prefer it. Plain shells carry no daemon restore, so fall back to the literal
/// manual handoff a human would type (ssh into the box, cd into the cwd).
fn session_connect_command(session: &ManagedSessionView, cwd: &str) -> String {
    let restore = metadata_value(session, "Restore");
    if !restore.trim().is_empty() {
        return restore.trim().to_string();
    }
    let cwd = cwd.trim();
    if let Some(target) = session
        .ssh_target
        .as_ref()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
    {
        if cwd.is_empty() {
            return format!("ssh {target}");
        }
        return format!("ssh {target}\ncd {cwd}");
    }
    if !cwd.is_empty() {
        return format!("cd {cwd}");
    }
    String::new()
}
/// ⛔ THE SSOT TITLE (owner law, 2026-09-02): the rail's Title entry and the
/// sidebar row must answer with the same string. The rail used to print
/// `session.title` raw — which for rows born before the birth-title law is the
/// forbidden `Remote {CLI} {shorthash}` shape — while the row showed its own
/// derived name. Now: a real title passes through untouched; a low-signal one
/// falls through the SAME humanized fallback (`humanized_terminal_title`) the
/// row's label ends at, so a row and its rail cannot disagree again.
fn session_metadata_title(session: &ManagedSessionView, cwd: &str) -> Option<String> {
    let raw = session.title.trim();
    if !raw.is_empty() && !yggterm_core::looks_like_generated_fallback_title(raw) {
        return Some(raw.to_string());
    }
    humanized_terminal_title(session.kind, cwd, Some(session.host_label.trim()))
        .filter(|title| !title.trim().is_empty())
        .or_else(|| (!raw.is_empty()).then(|| raw.to_string()))
}
/// The session-id line the metadata pane shows for one row: WHICH id and
/// under WHAT name, resolved once so the pane and any reader agree.
///
/// ⛔ THE CLI'S OWN SESSION ID OUTRANKS THE ROW UUID, AND THE LABEL IS THE
/// REGISTRY'S, NOT A HAND MATCH. The pane used to read the "UUID" metadata and
/// hand-name two kinds — so an OpenCode row displayed its row uuid under a
/// generic "Session id" label, and for a uuid-keyed anchor row that uuid is
/// NOT a session id at all (measured live 2026-09-02: four uuid rows whose
/// "UUID" was the row's birth seat). Order: the CLI store id the plane stamped
/// (`session_metadata_label`), then the tab mirror's id, then the row uuid,
/// then the row id — each labelled as what it is.
pub(crate) fn metadata_session_identity(
    session: &ManagedSessionView,
) -> Option<(&'static str, String)> {
    let descriptor = yggterm_core::agent_cli::agent_cli_descriptor(session.kind);
    let label = descriptor
        .map(|d| d.session_metadata_label)
        .unwrap_or("Session id");
    let store_id = descriptor
        .map(|d| metadata_value(session, d.session_metadata_label))
        .filter(|value| !value.trim().is_empty());
    if let Some(value) = store_id {
        return Some((label, value.trim().to_string()));
    }
    let tab_id = metadata_value(session, "Tab Session Id");
    if !tab_id.trim().is_empty() {
        return Some((label, tab_id.trim().to_string()));
    }
    let uuid = metadata_value(session, "UUID");
    (!uuid.trim().is_empty()).then(|| (label, uuid.trim().to_string()))
}

/// The dynamicity lines the pane shows for one row: an OpenCode tab row names
/// its session; the TUI anchor names whichever session the human is LOOKING at
/// right now (the mirror refreshes it every tick from the service's focus
/// stream). Absent entries stay absent — a row that is not part of a tab
/// group has nothing dynamic to say.
pub(crate) fn metadata_dynamicity_entries(
    session: &ManagedSessionView,
) -> Vec<(&'static str, String)> {
    [
        ("Viewing Tab Session Id", "Viewing session"),
        ("Tab Session Id", "Mirrored session"),
    ]
    .iter()
    .filter_map(|(metadata_label, display_label)| {
        let value = metadata_value(session, metadata_label);
        (!value.trim().is_empty()).then(|| (*display_label, value.trim().to_string()))
    })
    .collect()
}

/// The Live Diagnostic group (Issue 31): the row's identity contract against
/// the CLI's own dynamicity signal, composed from snapshot fields the GUI
/// already holds — no new wire fields.
///
/// ⛔ A WITNESS, NEVER A DRIVER: nothing here feeds back into daemon
/// decisions, and the group renders only for agent rows carrying a Viewing
/// stamp (the mirror saw the TUI render something). A quiet row shows nothing
/// new — absence of the group means absence of signal, not health.
///
/// The verdict vocabulary matches `cli/mirror_tick`'s `decision` field so the
/// pane and the probe can be read against each other: `in sync` |
/// `DIVERGED — row aims at <bound>, TUI shows <viewing>`.
pub(crate) fn metadata_live_diagnostic(
    session: &ManagedSessionView,
) -> Vec<SessionMetadataEntry> {
    if yggterm_core::agent_cli::agent_cli_descriptor(session.kind).is_none() {
        return Vec::new();
    }
    let viewing = metadata_value(session, "Viewing Tab Session Id");
    let viewing = viewing.trim();
    if viewing.is_empty() {
        return Vec::new();
    }
    let bound = metadata_session_identity(session)
        .map(|(_, value)| value)
        .unwrap_or_else(|| session.id.clone());
    let verdict = if bound.trim() == viewing {
        format!("in sync — row and TUI agree on {viewing}")
    } else {
        format!(
            "DIVERGED — row aims at {}, TUI shows {viewing}",
            bound.trim()
        )
    };
    vec![SessionMetadataEntry {
        label: "Identity",
        value: verdict,
    }]
}

/// Build the view-aware "useful" metadata panel from the rich snapshot fields
/// (kind, host/source, pty grid, pid, working state) plus the genuinely useful
/// daemon metadata entries (cwd, restore command, resume id, transcript stats),
/// instead of dumping every raw entry (Bytes, Preview Blocks, Launch Error:none,
/// the multi-line launch shell script). One source of truth per fact.
fn render_session_metadata(session: &ManagedSessionView, palette: Palette) -> Element {
    let kind = session.kind;
    let locality = match session.source {
        SessionSource::LiveLocal => "local",
        SessionSource::LiveSsh => "remote",
        SessionSource::Stored => "stored",
    };
    let host = session.host_label.trim();
    let machine = if host.is_empty() {
        locality.to_string()
    } else {
        format!("{host} · {locality}")
    };
    let cwd = {
        let primary = metadata_value(session, "Cwd");
        if primary.trim().is_empty() {
            metadata_value(session, "Target")
        } else {
            primary
        }
    };

    let mut identity = vec![
        SessionMetadataEntry {
            label: "Type",
            value: friendly_session_kind(kind).to_string(),
        },
        SessionMetadataEntry {
            label: "Machine",
            value: machine,
        },
    ];
    if !cwd.trim().is_empty() {
        identity.push(SessionMetadataEntry {
            label: "Working dir",
            value: cwd.trim().to_string(),
        });
    }
    if let Some(title) = session_metadata_title(session, &cwd) {
        identity.push(SessionMetadataEntry {
            label: "Title",
            value: title,
        });
    }

    let connect = session_connect_command(session, &cwd);

    // The limit-wait check comes FIRST: a session waiting out a usage limit
    // has no working phrase on screen, so `working` reads `Some(false)` — and
    // rendering that as "idle" was the owner-reported lie (the row's own
    // footer said "Usage limit reached · continuing shortly" while this field
    // said idle).
    // ⛔ The picker check comes before BOTH: a row holding an owner question is
    // mid-turn, so `working` reads `Some(true)` and this field said "working"
    // on a session that had been stopped for 27 minutes waiting for its owner
    // — while eating every sentence typed at it.
    let working = if session.awaiting_user_choice {
        "asking you a question"
    } else if session.limit_wait {
        "waiting on limit"
    } else {
        match session.working {
            Some(true) => "working",
            Some(false) => "idle",
            None => "—",
        }
    };
    let mut runtime = vec![SessionMetadataEntry {
        label: "Status",
        value: format!(
            "{} · {working}",
            friendly_launch_phase(session.launch_phase)
        ),
    }];
    // PTY grid is surfaced by `managed_session_from_snapshot` as a "PTY size"
    // metadata entry (the daemon owns the live grid; the GUI session model does
    // not carry pty_cols/rows directly).
    let pty_size = metadata_value(session, "PTY size");
    if !pty_size.trim().is_empty() {
        runtime.push(SessionMetadataEntry {
            label: "PTY size",
            value: pty_size.trim().to_string(),
        });
    }
    if let Some(pid) = session.terminal_process_id {
        runtime.push(SessionMetadataEntry {
            label: "PID",
            value: pid.to_string(),
        });
    }
    {
        let (id_label, id_value) =
            metadata_session_identity(session).unwrap_or(("Session id", session.id.clone()));
        runtime.push(SessionMetadataEntry {
            label: id_label,
            value: id_value,
        });
    }
    // The CLI's dynamicity, as far as this row carries it: an OpenCode tab row
    // names its session; the TUI anchor names whichever session the human is
    // LOOKING at right now (the mirror refreshes it every tick from the
    // service's focus stream). Absent entries stay absent — a row that is not
    // part of a tab group has nothing dynamic to say.
    for (display_label, value) in metadata_dynamicity_entries(session) {
        runtime.push(SessionMetadataEntry {
            label: display_label,
            value,
        });
    }
    let live_diagnostic = metadata_live_diagnostic(session);

    let mut history = Vec::new();
    for (label, key) in [
        ("Conversation", "Messages"),
        ("Started", "Started"),
        ("Last active", "Updated"),
        ("Persistence", "Runtime Persistence"),
        ("Rollout file", "Storage"),
    ] {
        let value = metadata_value(session, key);
        if !value.trim().is_empty() {
            history.push(SessionMetadataEntry {
                label,
                value: value.trim().to_string(),
            });
        }
    }

    rsx! {
        MetadataGroup { title: "Session".to_string(), entries: identity, palette }
        if !connect.trim().is_empty() {
            MetadataConnectBlock { palette, command: connect }
        }
        MetadataGroup { title: "Runtime".to_string(), entries: runtime, palette }
        if !live_diagnostic.is_empty() {
            MetadataGroup { title: "Live Diagnostic".to_string(), entries: live_diagnostic, palette }
        }
        if !history.is_empty() {
            MetadataGroup { title: "History".to_string(), entries: history, palette }
        }
    }
}
#[component]
fn MetadataConnectBlock(palette: Palette, command: String) -> Element {
    let background = if palette_is_dark(palette) {
        "rgba(255,255,255,0.05)"
    } else {
        "rgba(13,21,30,0.04)"
    };
    let border = if palette_is_dark(palette) {
        "rgba(141,160,178,0.20)"
    } else {
        "rgba(198,212,224,0.55)"
    };
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px; padding-bottom:4px;",
            RailSectionTitle { title: "Connect".to_string(), muted_color: palette.muted.to_string() }
            div {
                style: format!("font-size:11px; line-height:1.45; color:{};", palette.muted),
                "Reattach to this session's PTY from any shell:"
            }
            div {
                "data-metadata-connect-command": "1",
                style: format!(
                    "font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size:11.5px; \
                     line-height:1.55; color:{}; background:{}; border-radius:10px; padding:10px 11px; \
                     white-space:pre-wrap; word-break:break-all; user-select:text; -webkit-user-select:text; \
                     cursor:text; box-shadow: inset 0 0 0 1px {};",
                    palette.text, background, border
                ),
                "{command}"
            }
        }
    }
}
#[component]
fn SettingsRailBody(
    snapshot: SharedSnapshot,
    on_endpoint_change: EventHandler<String>,
    on_api_key_change: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_open_launch_flags: EventHandler<MouseEvent>,
    on_open_cli_install: EventHandler<MouseEvent>,
    on_focus_input: EventHandler<String>,
    on_blur_input: EventHandler<()>,
    on_set_ui_theme: EventHandler<UiTheme>,
    on_open_theme_editor: EventHandler<MouseEvent>,
    on_open_keymap_editor: EventHandler<MouseEvent>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_set_terminal_telemetry: EventHandler<bool>,
    on_set_perf_profiling: EventHandler<bool>,
    on_set_titlebar_auto_hide: EventHandler<bool>,
    on_set_chrome_mirrored: EventHandler<bool>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_set_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
    on_set_main_zoom: EventHandler<i32>,
    on_set_terminal_theme_name: EventHandler<(UiTheme, String)>,
    on_trigger_update: EventHandler<MouseEvent>,
) -> Element {
    let terminal_light_theme_options = terminal_theme_names_for_mode(UiTheme::ZedLight);
    let terminal_dark_theme_options = terminal_theme_names_for_mode(UiTheme::ZedDark);
    let selected_light_terminal_theme = terminal_theme_value_for_settings(
        &snapshot.settings.terminal_light_theme_name,
        UiTheme::ZedLight,
    );
    let selected_dark_terminal_theme = terminal_theme_value_for_settings(
        &snapshot.settings.terminal_dark_theme_name,
        UiTheme::ZedDark,
    );
    let main_zoom_label = active_viewport_zoom_label(&snapshot);
    rsx! {
        RailHeader { title: "Settings".to_string(), color: snapshot.palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
            div {
                // §12.2's inversion dissolved the old blanket
                // `data-keytip-exempt="settings-panel"` (§12.1: an exempt
                // SUBTREE is forbidden): the panel's controls are now DERIVED
                // by the overlay-open walk, so each visible widget gets its own
                // letter with no per-widget declaration.
                style: "display:flex; flex-direction:column; gap:12px; padding-bottom:8px;",
            ChromeBehaviorSettingsSection {
                palette: snapshot.palette,
                auto_hide_titlebar: snapshot.settings.auto_hide_titlebar,
                chrome_mirrored: snapshot.settings.chrome_orientation.is_mirrored(),
                on_change: on_set_titlebar_auto_hide,
                on_change_mirror: on_set_chrome_mirrored,
            }
            style { "{UPDATE_CTA_CSS}" }
            InstallUpdateRow {
                update_call_to_action: snapshot.update_call_to_action.clone(),
                // ⛔ The RUNNING build, not the install record's claim about it.
                // This read `install_context.current_version`, which is the
                // record's `active_version` — and the record is a claim about a
                // PATH, kept by whatever last wrote it. On the live host it said
                // **2.11.0** while the process rendering this panel was 3.0.44,
                // under a button offering to "restart now to update" to the
                // build already running. The user read the panel and asked why.
                //
                // Two encodings of "what version am I" existed side by side:
                // `daemon_update_state.current_gui_version` was correct all
                // along because it uses `current_version()`. This is now the
                // same one owner. See `handoff_target_is_usable` for the other
                // half of the same record's dishonesty.
                version: yggterm_core::current_version(),
                palette: snapshot.palette,
                on_trigger_update,
            }
            MetadataGroup {
                title: "Install".to_string(),
                entries: vec![
                    SessionMetadataEntry {
                        label: "Channel",
                        value: format!("{:?}", snapshot.install_context.channel).to_lowercase(),
                    },
                    SessionMetadataEntry {
                        label: "Updates",
                        value: match snapshot.install_context.update_policy {
                            yggterm_core::UpdatePolicy::Auto => "Automatic on launch".to_string(),
                            yggterm_core::UpdatePolicy::NotifyOnly => snapshot
                                .install_context
                                .manager_hint
                                .clone()
                                .unwrap_or_else(|| "Notify only".to_string()),
                        },
                    },
                ],
                palette: snapshot.palette,
            }
            SettingsField {
                field_key: "litellm-endpoint".to_string(),
                label: "LiteLLM Endpoint".to_string(),
                value: snapshot.settings.litellm_endpoint.clone(),
                placeholder: "https://litellm.example/v1".to_string(),
                secret: false,
                autofocus: false,
                palette: snapshot.palette,
                on_focus_input: on_focus_input.clone(),
                on_blur_input: on_blur_input.clone(),
                on_change: on_endpoint_change,
            }
            SettingsField {
                field_key: "litellm-api-key".to_string(),
                label: "API Key".to_string(),
                value: snapshot.settings.litellm_api_key.clone(),
                placeholder: "sk-...".to_string(),
                secret: true,
                autofocus: false,
                palette: snapshot.palette,
                on_focus_input: on_focus_input.clone(),
                on_blur_input: on_blur_input.clone(),
                on_change: on_api_key_change,
            }
            SettingsField {
                field_key: "interface-llm".to_string(),
                label: "Interface LLM".to_string(),
                value: snapshot.settings.interface_llm_model.clone(),
                placeholder: "openai/gpt-5.4-mini".to_string(),
                secret: false,
                autofocus: false,
                palette: snapshot.palette,
                on_focus_input: on_focus_input.clone(),
                on_blur_input: on_blur_input.clone(),
                on_change: on_model_change,
            }
            LaunchFlagsSettingsSection {
                palette: snapshot.palette,
                summary: launch_flags_rail_summary(&snapshot.settings.agent_cli_extra_args),
                on_open: on_open_launch_flags,
            }
            CliInstallSettingsSection {
                palette: snapshot.palette,
                summary: cli_install_rail_summary(&snapshot.settings.agent_cli_install_consent),
                on_open: on_open_cli_install,
            }
            ThemeSettingsSection {
                palette: snapshot.palette,
                selected_theme: snapshot.settings.theme,
                accent: snapshot.theme_accent.clone(),
                custom_stop_count: snapshot.settings.yggui_theme.colors.len(),
                light_tip: keytip_tip_attr(&snapshot, "theme.light"),
                dark_tip: keytip_tip_attr(&snapshot, "theme.dark"),
                on_select: on_set_ui_theme,
                on_open_editor: on_open_theme_editor,
            }
            KeytipsSettingsSection {
                palette: snapshot.palette,
                accent: snapshot.theme_accent.clone(),
                customized: !snapshot.keymap.overrides().is_empty(),
                on_open_editor: on_open_keymap_editor,
            }
            NotificationSettingsSection {
                palette: snapshot.palette,
                selected: notification_delivery_mode(&snapshot.settings),
                sound_enabled: snapshot.settings.notification_sound,
                on_select: on_set_notification_delivery,
                on_change: on_set_notification_sound,
            }
            TelemetrySettingsSection {
                palette: snapshot.palette,
                enabled: snapshot.settings.terminal_telemetry_enabled,
                db_path: "~/.yggterm/telemetry/terminal.sqlite3".to_string(),
                on_change: on_set_terminal_telemetry,
            }
            PerfProfilingSettingsSection {
                palette: snapshot.palette,
                enabled: snapshot.settings.perf_profiling_enabled,
                on_change: on_set_perf_profiling,
            }
            ZoomSettingRow {
                field_key: "interface-zoom".to_string(),
                label: "Interface Zoom".to_string(),
                percent: zoom_percent(snapshot.settings.ui_font_size, 14.0),
                palette: snapshot.palette,
                on_focus_input: on_focus_input.clone(),
                on_blur_input: on_blur_input.clone(),
                on_decrease: move |_| on_adjust_ui_zoom.call(-1),
                on_increase: move |_| on_adjust_ui_zoom.call(1),
                on_set_percent: move |value: i32| on_set_ui_zoom.call(value),
            }
            ZoomSettingRow {
                field_key: "main-zoom".to_string(),
                label: main_zoom_label,
                percent: zoom_percent(
                    active_viewport_zoom_value(&snapshot),
                    main_zoom_base(active_main_zoom_target(&snapshot)),
                ),
                palette: snapshot.palette,
                on_focus_input,
                on_blur_input,
                on_decrease: move |_| on_adjust_main_zoom.call(-1),
                on_increase: move |_| on_adjust_main_zoom.call(1),
                on_set_percent: move |value: i32| on_set_main_zoom.call(value),
            }
            if active_viewport_shows_terminal_theme(&snapshot) {
                TerminalThemeSettingRow {
                    palette: snapshot.palette,
                    light_value: selected_light_terminal_theme,
                    dark_value: selected_dark_terminal_theme,
                    light_options: terminal_light_theme_options,
                    dark_options: terminal_dark_theme_options,
                    on_change: on_set_terminal_theme_name,
                }
            }
            if cfg!(debug_assertions) {
                MetadataGroup {
                    title: "Terminal Debug".to_string(),
                    entries: vec![
                        SessionMetadataEntry {
                            label: "State",
                            value: snapshot.last_terminal_debug.clone(),
                        },
                        SessionMetadataEntry {
                            label: "Active",
                            value: snapshot.active_session.as_ref().map(|session| session.session_path.clone()).unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Host",
                            value: snapshot.active_session.as_ref().map(|session| terminal_host_id(&session.session_path)).unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Font",
                            value: format!("{:.1}", snapshot.settings.terminal_font_size),
                        },
                    ],
                    palette: snapshot.palette,
                }
                MetadataGroup {
                    title: "Tree Debug".to_string(),
                    entries: vec![
                        SessionMetadataEntry {
                            label: "State",
                            value: snapshot.last_tree_debug.clone(),
                        },
                        SessionMetadataEntry {
                            label: "Selected",
                            value: if snapshot.selected_tree_paths.is_empty() {
                                "none".to_string()
                            } else {
                                snapshot.selected_tree_paths.join(", ")
                            },
                        },
                        SessionMetadataEntry {
                            label: "Drag Target",
                            value: snapshot
                                .drag_hover_target
                                .as_ref()
                                .map(|target| format!("{}:{:?}", target.path, target.placement))
                                .unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Pending Delete",
                            value: snapshot
                                .pending_delete
                                .as_ref()
                                .map(|pending| format!(
                                    "{} item(s), hard={}",
                                    pending.document_paths.len()
                                        + pending.group_paths.len()
                                        + pending.session_paths.len()
                                        + pending.ssh_machine_keys.len(),
                                    pending.hard_delete
                                ))
                                .unwrap_or_else(|| "none".to_string()),
                        },
                    ],
                    palette: snapshot.palette,
                }
            }
            }
            }
        }
    }
}
#[component]
fn NotificationsRailBody(
    snapshot: SharedSnapshot,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
    on_activate_notification: EventHandler<String>,
) -> Element {
    rsx! {
        RailHeader { title: "Notifications".to_string(), color: snapshot.palette.text.to_string() }
        div {
            // The old `notifications-panel` subtree exemption is dissolved
            // (§12.1/§12.2): Clear All is derived by the overlay-open walk.
            style: "padding:0 16px 8px 16px; display:flex; justify-content:flex-end;",
            button {
                style: chip_style(snapshot.palette, false),
                onclick: move |evt| on_clear_notifications.call(evt),
                "Clear All"
            }
        }
        RailScrollBody {
            content: rsx!{
            if snapshot.notifications.is_empty() {
                div {
                    style: format!("font-size:12px; line-height:1.5; color:{};", snapshot.palette.muted),
                    "No notifications yet."
                }
            } else {
                for notification in snapshot.notifications.iter().cloned().rev() {
                    ToastCard {
                        item: notification.clone(),
                        palette: ToastPalette {
                            text: snapshot.palette.text,
                            muted: snapshot.palette.muted,
                            accent: snapshot.palette.accent,
                            is_dark: palette_is_dark(snapshot.palette),
                        },
                        on_clear: move |_| on_clear_notification.call(notification.id),
                        on_activate: move |session_path: String| {
                            on_activate_notification.call(session_path)
                        },
                        // The panel is where notifications are KEPT and read
                        // later, so this is where "when" and "which session"
                        // earn their space. The floating toast gets neither: it
                        // is its own timestamp.
                        meta: true,
                        now_ms: current_millis(),
                        source_label: notification
                            .source
                            .as_deref()
                            .and_then(|path| session_display_label(&snapshot, path)),
                    }
                }
            }
            }
        }
    }
}
#[component]
fn ConnectRailBody(
    snapshot: SharedSnapshot,
    on_connect_ssh_custom: EventHandler<MouseEvent>,
    on_ssh_target_change: EventHandler<String>,
    on_ssh_prefix_change: EventHandler<String>,
) -> Element {
    rsx! {
        RailHeader { title: "Connect SSH".to_string(), color: snapshot.palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
            div {
            // The old `connect-form` subtree exemption is dissolved (§12.1/
            // §12.2): the form's fields and buttons are derived by the
            // overlay-open walk.
            style: "display:flex; flex-direction:column; gap:10px;",
            div {
                style: "display:flex; flex-direction:column; gap:10px; padding-bottom:10px;",
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Guide"
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "Use `user@ip`, `user@host`, or an SSH config alias such as `dev` in the target field. Yggterm will SSH there and open or focus a live terminal session."
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "The prefix field is optional. Think of it as the command that should run immediately after SSH lands on the remote machine."
                }
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Example"
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "Example: if `dev` is your SSH host and the real work happens inside an LXC guest there, enter `dev` as the target and use `lxc exec yggdrasil -- bash` as the prefix. Yggterm will SSH into `dev`, run that prefix, and continue from inside the container."
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "The same pattern works for tmux (`tmux new-session -A -s yggterm`), Docker (`docker exec -it web sh`), or systemd/machinectl shells (`sudo machinectl shell prod /bin/bash`)."
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;"
                ),
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Target"
                }
                input {
                    r#type: "text",
                    // ⛔ `initial_value`: this text comes from the lagging
                    // snapshot, and volatile `value` re-asserts the stale
                    // copy mid-edit, throwing the caret to the end.
                    initial_value: "{snapshot.ssh_connect_target}",
                    placeholder: "dev or pi@raspberry or user@192.0.2.15",
                    style: format!(
                        "height:36px; padding:0 12px; border:1px solid {}; border-radius:10px; background:{}; color:{}; \
                         font-size:12px; outline:none; box-shadow: inset 0 1px 0 rgba(255,255,255,0.55);",
                        snapshot.palette.border, snapshot.palette.panel, snapshot.palette.text
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    onclick: |evt| evt.stop_propagation(),
                    onkeydown: |evt| evt.stop_propagation(),
                    oninput: move |evt| on_ssh_target_change.call(evt.value()),
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Optional Prefix"
                }
                input {
                    r#type: "text",
                    // ⛔ `initial_value`: this text comes from the lagging
                    // snapshot, and volatile `value` re-asserts the stale
                    // copy mid-edit, throwing the caret to the end.
                    initial_value: "{snapshot.ssh_connect_prefix}",
                    placeholder: "Optional prefix, e.g. sudo machinectl shell prod",
                    style: format!(
                        "height:36px; padding:0 12px; border:1px solid {}; border-radius:10px; background:{}; color:{}; \
                         font-size:12px; outline:none; box-shadow: inset 0 1px 0 rgba(255,255,255,0.55);",
                        snapshot.palette.border, snapshot.palette.panel, snapshot.palette.text
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    onclick: |evt| evt.stop_propagation(),
                    onkeydown: |evt| evt.stop_propagation(),
                    oninput: move |evt| on_ssh_prefix_change.call(evt.value()),
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
                button {
                    style: primary_action_style(snapshot.palette),
                    onclick: move |evt| on_connect_ssh_custom.call(evt),
                    div {
                        style: "display:flex; flex-direction:column; align-items:flex-start; gap:3px; min-width:0;",
                        span {
                            style: "font-size:12px; font-weight:800; color:white;",
                            "Proceed ->"
                        }
                        span {
                            style: "font-size:11px; line-height:1.35; color:rgba(255,255,255,0.88); white-space:pre-wrap;",
                            "Yggterm will open or focus a terminal session for this target."
                        }
                    }
                }
            }
            if !snapshot.ssh_targets.is_empty() || !snapshot.remote_machines.is_empty() {
                div {
                    style: "display:flex; flex-direction:column; gap:10px; padding-top:14px; border-top:1px solid rgba(127,127,127,0.20);",
                    div {
                        style: format!(
                            "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            snapshot.palette.muted
                        ),
                        "Connected SSH Systems"
                    }
                    for target in snapshot.ssh_targets.iter() {
                        {
                            let target_str = target.ssh_target.clone();
                            let prefix_str = target.prefix.clone().unwrap_or_default();
                            let on_ssh_target_change = on_ssh_target_change.clone();
                            let on_ssh_prefix_change = on_ssh_prefix_change.clone();
                            let on_connect_ssh_custom = on_connect_ssh_custom.clone();
                            let machine_match = snapshot.remote_machines.iter().find(|m| m.ssh_target == target.ssh_target);
                            let session_count = machine_match.map(|m| m.sessions.len()).unwrap_or(0);
                            rsx! {
                                div {
                                    key: "ssh-target-{target_str}",
                                    style: format!(
                                        "display:flex; align-items:center; justify-content:space-between; gap:10px; padding:10px 12px; \
                                         border-radius:10px; background:{}; border:1px solid {}; box-sizing:border-box;",
                                        snapshot.palette.panel, snapshot.palette.border
                                    ),
                                    div {
                                        style: "display:flex; flex-direction:column; gap:3px; min-width:0; flex:1;",
                                        div {
                                            style: "display:flex; align-items:center; gap:6px;",
                                            span {
                                                style: format!("font-size:12px; font-weight:800; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", snapshot.palette.text),
                                                "{target_str}"
                                            }
                                            if session_count > 0 {
                                                span {
                                                    style: format!(
                                                        "display:inline-flex; align-items:center; padding:1px 6px; border-radius:999px; \
                                                         background:{}; color:{}; font-size:10px; font-weight:700;",
                                                        snapshot.palette.accent_soft, snapshot.palette.accent
                                                    ),
                                                    "{session_count} live"
                                                }
                                            }
                                        }
                                        if !prefix_str.is_empty() {
                                            span {
                                                style: format!("font-size:10px; color:{}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", snapshot.palette.muted),
                                                "prefix: {prefix_str}"
                                            }
                                        }
                                    }
                                    button {
                                        style: format!(
                                            "display:inline-flex; align-items:center; padding:5px 12px; border:none; border-radius:7px; \
                                             background:{}; color:white; font-size:11px; font-weight:700; cursor:pointer;",
                                            snapshot.palette.accent
                                        ),
                                        onclick: {
                                            let t = target_str.clone();
                                            let p = prefix_str.clone();
                                            move |evt| {
                                                on_ssh_target_change.call(t.clone());
                                                on_ssh_prefix_change.call(p.clone());
                                                on_connect_ssh_custom.call(evt);
                                            }
                                        },
                                        "Open"
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
}
/// One entry of the ROW MENU — the sidebar's right-click menu, and the ALT layer's
/// `rowmenu` scope.
///
/// The ordered `Vec<RowMenuItem>` this builds is the SINGLE SOURCE OF TRUTH for
/// what that menu contains: [`ContextMenuOverlay`] draws it and
/// [`build_keytip_scopes`] declares it. So a chord can never name an item the menu
/// does not show, an item can never appear without an accelerator (the §12 audit),
/// and adding an item wires the mouse and the keyboard in one edit.
/// THE SHELL'S ONE ICON SET — a menu entry's leading mark, a contributed row's
/// verb, a pane footer's archive and delete.
///
/// DESIGN.md ▸ Context menus asks for "modern Microsoft app menus", and those
/// have an icon column; DESIGN.md ▸ Tree behavior and ▸ Brand and mascot ask for
/// the marks themselves to stay "restrained and mostly grayscale" and to be
/// "crisp simple line icons". So these are stroked SVG paths in `currentColor`
/// on a 14-unit box — they inherit the row's tone, including the destructive red
/// and the dimmed grey, rather than carrying colour of their own. An emoji glyph
/// would fail both rules at once (full colour, and a different metric per
/// platform).
///
/// A NAMED SET, not free-form path data at the call site: an icon vocabulary
/// that anyone can extend inline is how a menu ends up with three different
/// close marks.
///
/// ⛔ IT SERVES CONTRIBUTED PANES TOO, and that is why it stopped being called
/// `MenuIcon` on 2026-08-04. A contributed app names a mark with the
/// `icon:<name>` token ([`ShellIcon::from_token`]) wherever the schema takes a
/// glyph — a `list-row`'s `icon`, a row action's `label`, a footer button's
/// `label`. Before that, an app could only send a CHARACTER, so ychrome's vault
/// rail wore `⧉ ⏱ ✎ 👁 🗑` — emoji at 11px, in whatever face the platform
/// happened to have, which is exactly what this set exists to prevent. The user
/// saw the result and said the icons "look illegible".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShellIcon {
    Plus,
    Reopen,
    Reload,
    Copy,
    Duplicate,
    Split,
    Folder,
    Rename,
    Collapse,
    Expand,
    Back,
    Close,
    Trash,
    /// Reveal a stored value — the vault's eye.
    Eye,
    /// Hide it again.
    EyeOff,
    /// A time-based code (TOTP).
    Clock,
    /// Put this into the page — the vault's fill verb.
    Fill,
    /// A payment card.
    Card,
    /// A passkey / authenticator secret.
    Key,
    /// Archive: put it away without destroying it.
    Archive,
    /// Open this elsewhere (a URI).
    External,
    /// A site / web address.
    Globe,
    /// Roll a new value — the password generator.
    Dice,
    /// Past values, in order.
    History,
    /// An affirmative mark.
    Check,
}

impl ShellIcon {
    /// The `icon:<name>` token a CONTRIBUTED pane may send instead of a glyph.
    ///
    /// ONE spelling, resolved in ONE place, for every slot a schema can put a
    /// mark in. Anything unknown returns `None` and the caller falls back to
    /// drawing the text it was given — an app that names an icon this shell
    /// does not have gets its literal string, never a blank.
    fn from_token(token: &str) -> Option<Self> {
        Some(match token.strip_prefix("icon:")? {
            "plus" => ShellIcon::Plus,
            "reopen" => ShellIcon::Reopen,
            "reload" => ShellIcon::Reload,
            "copy" => ShellIcon::Copy,
            "duplicate" => ShellIcon::Duplicate,
            "split" => ShellIcon::Split,
            "folder" => ShellIcon::Folder,
            "rename" | "edit" | "pencil" => ShellIcon::Rename,
            "collapse" => ShellIcon::Collapse,
            "expand" => ShellIcon::Expand,
            "back" => ShellIcon::Back,
            "close" => ShellIcon::Close,
            "trash" | "delete" => ShellIcon::Trash,
            "eye" | "reveal" => ShellIcon::Eye,
            "eye-off" | "hide" => ShellIcon::EyeOff,
            "clock" | "totp" => ShellIcon::Clock,
            "fill" => ShellIcon::Fill,
            "card" => ShellIcon::Card,
            "key" | "passkey" => ShellIcon::Key,
            "archive" => ShellIcon::Archive,
            "external" | "open" => ShellIcon::External,
            "globe" | "site" => ShellIcon::Globe,
            "dice" | "generate" => ShellIcon::Dice,
            "history" => ShellIcon::History,
            "check" => ShellIcon::Check,
            _ => return None,
        })
    }

    /// The stroked paths that draw this mark, on a `0 0 14 14` box.
    fn paths(self) -> &'static [&'static str] {
        match self {
            ShellIcon::Plus => &["M7 3.2v7.6", "M3.2 7h7.6"],
            ShellIcon::Reopen => &["M3.6 7.4a3.9 3.9 0 1 1 1.4 3.3", "M2.6 4.6v3h3"],
            ShellIcon::Reload => &["M10.4 6.6a3.9 3.9 0 1 0 .3 2.2", "M11.4 3.6v3h-3"],
            ShellIcon::Copy => &["M5.4 5.4h5.2v5.2H5.4z", "M3.4 8.6V3.4h5.2"],
            ShellIcon::Duplicate => &["M3.2 3.2h5v5h-5z", "M5.8 10.8h5v-5"],
            ShellIcon::Split => &["M2.6 3.4h8.8v7.2H2.6z", "M7 3.4v7.2"],
            ShellIcon::Folder => &[
                "M2.4 4.3a1 1 0 0 1 1-1h2l1.2 1.4h4.1a1 1 0 0 1 1 1v4.1a1 1 0 0 1-1 1H3.4a1 1 0 0 1-1-1V4.3Z",
            ],
            ShellIcon::Rename => &["M3 11l1.4-.35 6-6-1.05-1.05-6 6L3 11z"],
            ShellIcon::Collapse => &["M3.8 5.4L7 8.6l3.2-3.2"],
            ShellIcon::Expand => &["M5.4 3.8L8.6 7l-3.2 3.2"],
            ShellIcon::Back => &["M8.6 3.8L5.4 7l3.2 3.2"],
            ShellIcon::Close => &["M4.2 4.2l5.6 5.6", "M9.8 4.2l-5.6 5.6"],
            ShellIcon::Trash => &[
                "M3.2 4.4h7.6",
                "M5.6 4.4V2.9h2.8v1.5",
                "M4.4 4.4l.45 6.3h4.3l.45-6.3",
            ],
            ShellIcon::Eye => &[
                "M1.6 7s2.1-3.4 5.4-3.4S12.4 7 12.4 7s-2.1 3.4-5.4 3.4S1.6 7 1.6 7Z",
                "M7 8.6a1.6 1.6 0 1 0 0-3.2 1.6 1.6 0 0 0 0 3.2Z",
            ],
            ShellIcon::EyeOff => &[
                "M2.6 4.6C1.9 5.4 1.6 7 1.6 7s2.1 3.4 5.4 3.4c1 0 1.9-.3 2.6-.7",
                "M11.5 8.7c.6-.7.9-1.7.9-1.7S10.3 3.6 7 3.6c-.5 0-1 .1-1.4.2",
                "M2.6 2.6l8.8 8.8",
            ],
            ShellIcon::Clock => &[
                "M7 2.6a4.4 4.4 0 1 0 0 8.8 4.4 4.4 0 0 0 0-8.8Z",
                "M7 4.6V7l1.7 1",
            ],
            // An arrow travelling INTO a box: "put this value in the page".
            ShellIcon::Fill => &[
                "M11.4 4.2V2.9H2.6v8.2h8.8V9.8",
                "M6 7h5.8",
                "M9.9 5.2 11.9 7l-2 1.8",
            ],
            ShellIcon::Card => &["M1.9 3.9h10.2v6.2H1.9z", "M1.9 6.1h10.2", "M4 8.4h2.4"],
            ShellIcon::Key => &[
                "M9.2 3.2a2.6 2.6 0 1 1-2.1 4.1L3 11.4H1.9V9.9l.9-.9h1.3V7.7h1.3l1-1a2.6 2.6 0 0 1 2.8-3.5Z",
                "M9.7 5.1h.01",
            ],
            // A lidded box: the archive tray, Bitwarden's own mark.
            ShellIcon::Archive => &[
                "M1.9 3.1h10.2v2.2H1.9z",
                "M2.9 5.3v5.6h8.2V5.3",
                "M5.6 7.5h2.8",
            ],
            ShellIcon::External => &[
                "M7.6 2.9h3.5v3.5",
                "M11.1 2.9 6.4 7.6",
                "M9.6 8.4v2.7H2.9V4.4h2.7",
            ],
            ShellIcon::Globe => &[
                "M7 2.1a4.9 4.9 0 1 0 0 9.8 4.9 4.9 0 0 0 0-9.8Z",
                "M2.1 7h9.8",
                "M7 2.1c1.3 1.4 2 3.1 2 4.9s-.7 3.5-2 4.9c-1.3-1.4-2-3.1-2-4.9s.7-3.5 2-4.9Z",
            ],
            ShellIcon::Dice => &["M2.4 2.4h9.2v9.2H2.4z", "M5 5h.01", "M9 9h.01", "M7 7h.01"],
            ShellIcon::History => &[
                "M2.5 7a4.5 4.5 0 1 0 1.4-3.3",
                "M1.7 2.6v2.9h2.9",
                "M7 4.6V7l1.9 1.1",
            ],
            ShellIcon::Check => &["M3 7.3 5.8 10 11 4.2"],
        }
    }
}

/// Draw a named mark at `size` px. ONE emitter — the menu's icon column, a
/// contributed row's verb and a footer's icon button all draw the same stroke
/// weight, so a mark cannot read as two different products in two places.
#[component]
fn ShellIconMark(icon: ShellIcon, size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}",
            height: "{size}",
            view_box: "0 0 14 14",
            fill: "none",
            style: "flex:0 0 auto; display:block;",
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

/// A schema-declared glyph, drawn as SHARP VECTOR when it names one of the
/// shell's marks and as the literal text otherwise.
///
/// The fallback is deliberate: an app that sends `"3"` or `"€"` still gets what
/// it asked for. Only `icon:<name>` reaches the vector set.
fn shell_glyph(label: &str, size: u32) -> Element {
    match ShellIcon::from_token(label) {
        Some(icon) => rsx! { ShellIconMark { icon, size } },
        None => rsx! { span { style: "font-size:{size}px; line-height:1;", "{label}" } },
    }
}

#[derive(Clone, PartialEq, Debug)]
struct RowMenuItem {
    /// Stable within the menu. Doubles as the `data-context-menu-action` value and
    /// the KeyTip node key; payload-carrying items encode the payload in it
    /// (`close-split-pane:<path>`, `app:<app>:<verb>`).
    id: String,
    label: String,
    /// The letter this item *wants*. The §5 ladder may deny it (earlier
    /// declarations win), so a hint is a preference, never a guarantee.
    hint: Option<char>,
    destructive: bool,
    emphasized: bool,
    /// A divider: drawn, never badged, never dispatched.
    separator: bool,
    /// Shown, but inert — and its label already SAYS why (see
    /// [`RowMenuItem::disabled`]). A menu that silently omits an item teaches
    /// the user the verb does not exist, and a menu whose shape depends on
    /// state teaches a verb exists only by accident; greying it out with the
    /// reason teaches what would make it available.
    disabled: bool,
    /// WHY the item is inert, or a note a live item carries — for the TOOLTIP,
    /// never for the label.
    ///
    /// It used to be appended to the label (`"Close tab — this is the app's own
    /// tab; quitting the app closes it"`), which made every dimmed row four
    /// times too long for the menu box and clipped it mid-word — the user's
    /// screenshot, and a menu in which not one dimmed verb could be READ. The
    /// reason is a hint about a command; it is not the command's name, and the
    /// user must never read our justification as if it were the verb.
    reason: Option<String>,
    /// The leading mark, when this menu draws an icon column.
    ///
    /// `None` is the default and stays the default for four of the five menus:
    /// [`ContextMenuOverlay`] draws the column only when at least ONE item in
    /// the list it was handed carries an icon, so a menu that opts out is drawn
    /// exactly as it was before the column existed. Within a menu that opts in,
    /// every row reserves the slot — a half-indented list reads worse than none.
    icon: Option<ShellIcon>,
    /// Non-empty ⇒ this item OPENS a child list instead of dispatching a verb.
    ///
    /// The child is drawn as a PAGE TURN in the same overlay, not a flyout:
    /// [`ContextMenuOverlay`] is the one menu in this app and it draws a flat
    /// list, so the submenu is the SAME box at the SAME anchor showing a
    /// different list — the pattern "Move to folder ▸" already uses. A true
    /// hover flyout would need per-item DOM geometry the shell deliberately does
    /// not keep, plus hover-intent timing, for no gain the keyboard layer does
    /// not already give.
    ///
    /// ⚖ The KeyTip layer does NOT page-turn with it: `build_keytip_scopes`
    /// declares the parent AND every child scope from this tree on every frame,
    /// so `ALT,E,S,L` resolves in one go without the submenu ever having been
    /// drawn. That split — flatten for the mouse, whole tree for the chord — is
    /// the entire trick.
    submenu: Vec<RowMenuItem>,
}
impl RowMenuItem {
    fn new(id: impl Into<String>, label: impl Into<String>, hint: char) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: Some(hint),
            destructive: false,
            emphasized: false,
            separator: false,
            disabled: false,
            icon: None,
            reason: None,
            submenu: Vec::new(),
        }
    }
    fn hinted(id: impl Into<String>, label: impl Into<String>, hint: Option<char>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint,
            destructive: false,
            emphasized: false,
            separator: false,
            disabled: false,
            icon: None,
            reason: None,
            submenu: Vec::new(),
        }
    }
    fn icon(mut self, icon: ShellIcon) -> Self {
        self.icon = Some(icon);
        self
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    fn emphasized(mut self) -> Self {
        self.emphasized = true;
        self
    }
    /// Grey the item out and SAY WHY — in the TOOLTIP. The reason is not
    /// decoration: an item that just goes dim is indistinguishable from a bug.
    /// But it is not the item's NAME either; appending it to the label is what
    /// made a 216px menu render `Close tab — this is the app's own ta…` and
    /// left the user with no readable verb at all.
    ///
    /// A disabled item also loses its accelerator — a chord must never reach a
    /// verb the mouse cannot.
    fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled = true;
        self.hint = None;
        self.reason = Some(reason.into());
        self
    }
    /// A note a LIVE item carries, for the same tooltip slot. Used where the
    /// consequence matters more than the verb reads — "Delete folder" needs to
    /// say that the tabs survive, and it needs to say it somewhere other than
    /// in its own name.
    fn note(mut self, note: impl Into<String>) -> Self {
        self.reason = Some(note.into());
        self
    }
    /// The hover text for this entry.
    ///
    /// The reason when there is one; otherwise the LABEL, so an entry the box
    /// had to ellipsize is still readable somewhere. A menu must never show a
    /// name it cannot show in full and offer no way to read the rest.
    fn tooltip(&self) -> String {
        match (&self.reason, self.disabled) {
            (Some(reason), true) => format!("{} — {reason}", self.label),
            (Some(note), false) => format!("{} — {note}", self.label),
            (None, _) => self.label.clone(),
        }
    }
    fn divider() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            hint: None,
            destructive: false,
            emphasized: false,
            separator: true,
            disabled: false,
            icon: None,
            reason: None,
            submenu: Vec::new(),
        }
    }

    /// Make this item a submenu opener. Child ids are NAMESPACED under the
    /// opener (`open-session-here/new-agent:pi`) so a node key stays a unique
    /// DOM identity and a unique dispatch target across both levels.
    fn submenu(mut self, items: Vec<RowMenuItem>) -> Self {
        self.submenu = items
            .into_iter()
            .map(|mut child| {
                child.id = format!("{}/{}", self.id, child.id);
                child
            })
            .collect();
        self
    }

    /// The label as drawn, with the affordance that says this opens a list.
    ///
    /// `▸` is the mark "Move to folder ▸" already carries, so the two submenus
    /// in the app read as one idea rather than two conventions.
    fn display_label(&self) -> String {
        if self.submenu.is_empty() {
            self.label.clone()
        } else {
            format!("{} \u{25b8}", self.label)
        }
    }
}

/// Does this menu draw an icon column?
///
/// A property of the LIST, asked once per menu, so the answer is the same for
/// every row in it: a column that appears on some rows and not others is a
/// ragged left edge, and reserving the slot unconditionally would re-indent the
/// four menus that carry no icons at all.
fn context_menu_has_icons(items: &[RowMenuItem]) -> bool {
    items.iter().any(|item| item.icon.is_some())
}
/// What a click on a menu item DISPATCHES — `None` when the item is inert.
///
/// THE guard, as a value rather than as a shape inside an event closure: a
/// disabled item cannot reach [`ContextMenuOverlay`]'s `on_action` because
/// there is no id to call it with. A guard spelled inside the closure could be
/// gutted while still LOOKING like a guard (`if is_disabled { (); }`), and a
/// source needle that only matches the `if` would never notice; this one is
/// exercised by a test that calls it.
///
/// A separator is not clickable either — it is drawn in the other branch, and
/// dispatching its empty id would be dispatching nothing.
fn context_menu_click_action(item: &RowMenuItem) -> Option<String> {
    if item.disabled || item.separator {
        return None;
    }
    Some(item.id.clone())
}

/// The KeyTip node key for a row-menu item (`rowmenu:<id>`), so the resolver and
/// [`dispatch_keytip_node`] agree on one identity per item.
fn row_menu_node_key(id: &str) -> String {
    format!("rowmenu:{id}")
}

/// Which viewport surface a right-click landed on — selects the context menu.
///
/// Only the TERMINAL is here. A document (yedit) or web (ychrome) surface is a
/// WebKit-rendered DOM/page whose OWN context menu already offers
/// Copy/Cut/Paste/Select-All, so the shell opens no menu over those at all (it
/// lets the native one through). The terminal is a canvas with no native menu,
/// so it needs this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ViewportMenuKind {
    Terminal,
    /// A document surface (yedit). Gets a real yggterm-drawn Copy/Cut/Paste/
    /// Select-All menu rather than relying on WebKitGTK's native one: the native
    /// menu could not be verified to fire here (a GTK popup is a separate window,
    /// invisible to the webview screenshot, and a synthetic DOM event cannot
    /// raise it), and "the user can right-click copy" is not something to ship on
    /// an unverified assumption.
    Document,
}

/// The menu for a viewport surface. Ids share the `viewport-` prefix so
/// [`dispatch_row_menu_action`] routes them to the surface, never to the row.
fn viewport_menu_items(kind: ViewportMenuKind) -> Vec<RowMenuItem> {
    match kind {
        ViewportMenuKind::Terminal => vec![
            RowMenuItem::new("viewport-copy", "Copy", 'C'),
            RowMenuItem::new("viewport-paste", "Paste", 'P'),
            RowMenuItem::divider(),
            RowMenuItem::new("viewport-select-all", "Select All", 'A'),
        ],
        // An editor edits, so Cut belongs here and not in the read-only terminal.
        ViewportMenuKind::Document => vec![
            RowMenuItem::new("viewport-copy", "Copy", 'C'),
            RowMenuItem::new("viewport-cut", "Cut", 'X'),
            RowMenuItem::new("viewport-paste", "Paste", 'P'),
            RowMenuItem::divider(),
            RowMenuItem::new("viewport-select-all", "Select All", 'A'),
        ],
    }
}

fn viewport_menu_title(kind: ViewportMenuKind) -> String {
    match kind {
        ViewportMenuKind::Terminal => "Terminal".to_string(),
        ViewportMenuKind::Document => "Editor".to_string(),
    }
}

// ===== the WebTabs rail's row menu =====
//
// The rail was the one row surface with no right-click menu at all, so every
// verb it owns (close, duplicate, file, split) was reachable only by hunting
// for a hover button. These build that menu in the SHARED vocabulary
// ([`RowMenuItem`]) so [`ContextMenuOverlay`] — the one menu component in the
// app — draws it exactly like the cwd tree's.

/// The `(id, folder)` pairs the scope arithmetic needs. Both tab homes reduce
/// to this: the render path from the overlay view, the dispatch path from
/// `ShellState`'s own tabs. One shape, so "which tabs does this act on" has one
/// answer no matter who asks.
type WebTabScopeRow = (u64, Option<u64>);

/// The app tab. `tabs[0]` belongs to the APP, not to the tree: it cannot be
/// closed by a tab verb (quitting the app is the strip's ⏻ / the row's ✕, which
/// sends a real Ctrl+C) and it can neither join a group nor head one.
const WEB_TAB_APP_TAB_ID: u64 = 0;

fn web_tab_scope_rows(tabs: &[WebSurfaceOverlayTabView]) -> Vec<WebTabScopeRow> {
    tabs.iter().map(|tab| (tab.id, tab.group_head)).collect()
}

/// The tabs a "Close other tabs" on `keep` closes.
///
/// GROUP-SCOPED, and the label says the number this returns: "other tabs" means
/// the ones beside it WHERE IT LIVES — a tab inside a group does not speak for
/// the root, and a root tab does not reach into anyone's group.
/// Never `keep` itself, and never the app tab.
///
/// One owner: the menu label counts this, and the action closes exactly this.
/// A label that promised a different number than the verb delivers is the
/// bulk-close dishonesty the user has forbidden outright.
fn web_tab_close_others_targets(tabs: &[WebTabScopeRow], keep: u64) -> Vec<u64> {
    let Some(scope) = tabs
        .iter()
        .find(|(id, _)| *id == keep)
        .map(|(_, head)| *head)
    else {
        return Vec::new();
    };
    tabs.iter()
        .filter(|(id, head)| *id != keep && *id != WEB_TAB_APP_TAB_ID && *head == scope)
        .map(|(id, _)| *id)
        .collect()
}

/// The tabs a group head's "Close N tabs" closes: its MEMBERS, and nothing
/// else. ⛔ Never the head itself — the head is a page the user is reading, not
/// a container, and closing the row you right-clicked when you asked to close
/// what is under it is exactly the bulk-close dishonesty the label must not
/// commit.
fn web_tab_group_close_targets(tabs: &[WebTabScopeRow], head: u64) -> Vec<u64> {
    tabs.iter()
        .filter(|(id, member_of)| {
            *id != WEB_TAB_APP_TAB_ID && *id != head && *member_of == Some(head)
        })
        .map(|(id, _)| *id)
        .collect()
}

/// The tabs a "Close tabs below" on `from` closes: the ones AFTER it in its own
/// scope. Group-scoped for the same reason
/// [`web_tab_close_others_targets`] is — a tab inside a group does not speak
/// for the root, and "below" in a tree means below within your own group.
fn web_tab_close_below_targets(tabs: &[WebTabScopeRow], from: u64) -> Vec<u64> {
    let Some(at) = tabs.iter().position(|(id, _)| *id == from) else {
        return Vec::new();
    };
    let scope = tabs[at].1;
    tabs.iter()
        .skip(at + 1)
        .filter(|(id, head)| *id != WEB_TAB_APP_TAB_ID && *head == scope)
        .map(|(id, _)| *id)
        .collect()
}

/// English for a count of tabs, so every destructive item in this menu NAMES
/// what it will take. One owner, because "Close 12 other tabs" and "Close 1
/// other tab" must not be two independently-maintained sentences.
fn web_tab_count_phrase(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The tab menu. Pure: same surface, same target, same page, same menu, in a
/// stable order.
///
/// GROUPED BY INTENT, dividers between the groups, destructive LAST. The order
/// used to open with "Close tab", which put the one irreversible verb under the
/// pointer the instant the menu appeared — and buried the create verbs, of which
/// there were none at all. DESIGN.md ▸ Context menus asks for "modern Microsoft
/// app menus", and those read create → act on this thing → arrange → destroy:
///
///   create      New tab · New tab above this one · Reopen closed tabs
///   page        Reload · Reload (drop cache) · Copy URL · Duplicate tab · Split with active tab
///   arrange     Move to group ▸ · (on a head) Expand/Collapse · Ungroup
///   destroy     Close tab · Close N other tabs · Close N tabs below
///
/// "Move to group ▸" turns the menu to its own PAGE rather than flattening one
/// sibling row per group into this list. The flat form pushed the tab's own
/// verbs off the bottom as soon as a user had more than a few groups, and it
/// scaled linearly with something the user controls. The page is the same
/// [`ContextMenuOverlay`] at the same anchor — one menu component, per the reuse
/// doctrine — showing a different list.
///
/// ⭐ ONE menu, because there is one kind of row. A row that HEADS a group
/// grows three more items in the arrange band; it does not get a menu of its
/// own, because it is not a different kind of thing — it is a tab that other
/// tabs point at.
fn web_tab_menu_items(
    tabs: &[WebSurfaceOverlayTabView],
    active_tab_id: u64,
    target: &WebTabMenuTarget,
    page: WebTabMenuPage,
    reopen_count: usize,
) -> Vec<RowMenuItem> {
    let scope = web_tab_scope_rows(tabs);
    let mut items: Vec<RowMenuItem> = Vec::new();
    match target {
        WebTabMenuTarget::Tab(tab_id) => {
            let tab_id = *tab_id;
            let Some(tab) = tabs.iter().find(|tab| tab.id == tab_id) else {
                return items;
            };
            if page == WebTabMenuPage::MoveToGroup {
                return web_tab_move_page_items(tab, tabs);
            }
            // ---- create -----------------------------------------------------
            items.push(RowMenuItem::new("webtab-new", "New tab", 't').icon(ShellIcon::Plus));
            // The head row's "+", which is what replaced the folder header's:
            // "+" fills the group, not the window.
            if tab.group_size > 0 {
                items.push(
                    RowMenuItem::new("webgroup-new-tab", "New tab in this group", 'g')
                        .icon(ShellIcon::Plus),
                );
            }
            // Meaningful only because placement has an owner: "above this one"
            // is a real destination now, and the tab it opens joins this tab's
            // opener group so the next one cascades above it.
            items.push(
                RowMenuItem::new("webtab-new-above", "New tab above this one", 'a')
                    .icon(ShellIcon::Plus),
            );
            let reopen = RowMenuItem::new(
                "webtab-reopen",
                format!(
                    "Reopen {}",
                    web_tab_count_phrase(reopen_count.max(1), "closed tab")
                ),
                'e',
            )
            .icon(ShellIcon::Reopen);
            items.push(if reopen_count == 0 {
                reopen.disabled("nothing has been closed here yet")
            } else {
                reopen
            });
            items.push(RowMenuItem::divider());
            // ---- this page --------------------------------------------------
            let reload = RowMenuItem::new("webtab-reload", "Reload", 'r').icon(ShellIcon::Reload);
            items.push(if tab.effective_url.trim().is_empty() {
                reload.disabled("this tab has not gone anywhere yet")
            } else {
                reload
            });
            let hard_reload =
                RowMenuItem::new("webtab-reload-hard", "Reload (drop cache)", 'h')
                    .icon(ShellIcon::Reload);
            items.push(if tab.effective_url.trim().is_empty() {
                hard_reload.disabled("this tab has not gone anywhere yet")
            } else {
                hard_reload
            });
            let copy_url =
                RowMenuItem::new("webtab-copy-url", "Copy URL", 'u').icon(ShellIcon::Copy);
            items.push(if tab.effective_url.trim().is_empty() {
                copy_url.disabled("this tab has no address to copy")
            } else {
                copy_url
            });
            // ⭐ The refusal keys on the PAGE, not on the tab's identity.
            //
            // It used to refuse for the whole app tab, and the reason it gave
            // itself was about persistence: a duplicate would be an ordinary
            // user tab, and duplicating the app's start page would mint the
            // stale row `persist_web_tabs` deliberately refuses to save. That
            // reasoning is right and it is NARROWER than the rule it was
            // spelled as. Once the user navigates the app tab to a real page —
            // a YouTube video, say — a duplicate of it is an ordinary tab
            // pointing at an ordinary URL, which is exactly what the tree
            // saves. The user hit this and asked why (2026-08-01).
            //
            // So it asks `web_tab_is_saved`, the ONE owner of "would the tree
            // keep this", which is the very rule the old comment was appealing
            // to. Nothing new is encoded here.
            let duplicate = RowMenuItem::new("webtab-duplicate", "Duplicate tab", 'd')
                .icon(ShellIcon::Duplicate);
            items.push(if tab.holds_saved_page {
                duplicate
            } else {
                duplicate.disabled("this tab is showing the app itself, not a page to copy")
            });
            let split = RowMenuItem::new("webtab-split", "Split with active tab", 's')
                .icon(ShellIcon::Split);
            items.push(if tab_id == active_tab_id {
                split.disabled("this IS the active tab")
            } else {
                split
            });
            items.push(RowMenuItem::divider());
            // ---- arrange ----------------------------------------------------
            // Naming a row is ARRANGE, not page: it changes the tree, not the
            // page. Discoverable here as well as by double-click — the folder
            // rows have had exactly this verb all along, and a tab row that
            // could only be renamed by a gesture nobody announces is a
            // half-shipped affordance.
            let rename =
                RowMenuItem::new("webtab-rename", "Rename tab", 'n').icon(ShellIcon::Rename);
            items.push(if tab.is_app_tab {
                rename.disabled("the app's tab is named by the app")
            } else {
                rename
            });
            let move_to =
                RowMenuItem::new("webtab-move", "Move to group ▸", 'm').icon(ShellIcon::Folder);
            items.push(if tab.is_app_tab {
                move_to.disabled("the app's tab belongs to the app, not to the tree")
            } else {
                move_to
            });
            // A head's own group verbs, in the ARRANGE band where they belong —
            // they arrange the tree, they do not destroy anything.
            if tab.group_size > 0 {
                items.push(
                    RowMenuItem::new(
                        "webgroup-toggle",
                        if tab.group_collapsed {
                            "Expand group"
                        } else {
                            "Collapse group"
                        },
                        'e',
                    )
                    .icon(if tab.group_collapsed {
                        ShellIcon::Expand
                    } else {
                        ShellIcon::Collapse
                    }),
                );
                // ⛔ The note is load-bearing: "Ungroup" beside a list of close
                // verbs reads as a destructive one, and it is the opposite —
                // nothing closes, the members move up one level.
                items.push(
                    RowMenuItem::new("webgroup-disband", "Ungroup", 'u')
                        .icon(ShellIcon::Collapse)
                        .note("its tabs move up one level; nothing closes"),
                );
            }
            items.push(RowMenuItem::divider());
            // ---- destroy, last ----------------------------------------------
            let close = RowMenuItem::new("webtab-close", "Close tab", 'c')
                .icon(ShellIcon::Close)
                .destructive();
            items.push(if tab.is_app_tab {
                close.disabled("this is the app's own tab; quitting the app closes it")
            } else {
                close
            });
            let others = web_tab_close_others_targets(&scope, tab_id);
            let close_others = RowMenuItem::new(
                "webtab-close-others",
                format!("Close {}", web_tab_count_phrase(others.len(), "other tab")),
                'o',
            )
            .icon(ShellIcon::Close)
            .destructive();
            items.push(if others.is_empty() {
                close_others.disabled(if tab.group_head.is_some() {
                    "nothing else is in this group"
                } else {
                    "nothing else is open at the root"
                })
            } else {
                close_others
            });
            let below = web_tab_close_below_targets(&scope, tab_id);
            let close_below = RowMenuItem::new(
                "webtab-close-below",
                format!("Close {} below", web_tab_count_phrase(below.len(), "tab")),
                'w',
            )
            .icon(ShellIcon::Close)
            .destructive();
            items.push(if below.is_empty() {
                close_below.disabled("nothing is below this one here")
            } else {
                close_below
            });
            // A head can close what it heads — its MEMBERS, never itself.
            if tab.group_size > 0 {
                let inside = web_tab_group_close_targets(&scope, tab_id);
                // ⛔ "Close N tabs in this group" is 25 chars and the rail's
                // menu draws about 23 — the label would be ellipsized into
                // "Close 1 tab in this…", which is a destructive verb the user
                // cannot read. The scope moves into the note, where the whole
                // sentence lives.
                let close_group = RowMenuItem::new(
                    "webgroup-close-tabs",
                    format!("Close {} inside", web_tab_count_phrase(inside.len(), "tab")),
                    'i',
                )
                .icon(ShellIcon::Close)
                .destructive()
                .note("the rows in this group; this row stays");
                items.push(if inside.is_empty() {
                    close_group.disabled("this group is empty")
                } else {
                    close_group
                });
            }
        }
    }
    items
}

/// The "Move to group ▸" PAGE: back out, then one row per destination.
///
/// One row per existing GROUP — that is, per row something already points at —
/// on its own page, so the tab's verbs are never pushed off the bottom by how
/// many groups the user happens to have. A tab already in a group still sees
/// that group, inert and saying so, because the current home being mysteriously
/// absent is worse than it being unclickable.
///
/// ⛔ There is no "New group…" item and there must not be one. A group is
/// created by dropping one row onto another — the head has to be a real tab, so
/// a verb here could only invent an empty container, which is the folder model
/// coming back through the menu.
///
/// The destinations exclude the tab itself, the app tab, and anything INSIDE
/// the tab's own group: each of those would make a cycle, and a cycle takes
/// every row under it out of the draw walk.
fn web_tab_move_page_items(
    tab: &WebSurfaceOverlayTabView,
    tabs: &[WebSurfaceOverlayTabView],
) -> Vec<RowMenuItem> {
    let mut items = vec![
        RowMenuItem::new("webtab-move-back", "Back", 'b').icon(ShellIcon::Back),
        RowMenuItem::divider(),
    ];
    let root = RowMenuItem::new("webtab-move-root", "Root", 'r').icon(ShellIcon::Folder);
    items.push(if tab.group_head.is_none() {
        root.disabled("already here")
    } else {
        root
    });
    let descends = |mut candidate: Option<u64>| {
        for _ in 0..tabs.len() {
            match candidate {
                Some(id) if id == tab.id => return true,
                Some(id) => {
                    candidate = tabs
                        .iter()
                        .find(|row| row.id == id)
                        .and_then(|row| row.group_head)
                }
                None => return false,
            }
        }
        false
    };
    for head in tabs.iter().filter(|row| {
        row.group_size > 0 && row.id != tab.id && !row.is_app_tab && !descends(Some(row.id))
    }) {
        let item = RowMenuItem::hinted(
            format!("webtab-move:{}", head.id),
            head.label.clone(),
            head.label.chars().next(),
        )
        .icon(ShellIcon::Folder);
        items.push(if tab.group_head == Some(head.id) {
            item.disabled("already here")
        } else {
            item
        });
    }
    items
}

/// The menu heading: the row the user right-clicked, named — and on a submenu
/// page, what that page is for.
fn web_tab_menu_title(
    _tabs: &[WebSurfaceOverlayTabView],
    _target: &WebTabMenuTarget,
    page: WebTabMenuPage,
) -> String {
    match page {
        // Which PAGE you are on — something no row on screen says.
        WebTabMenuPage::MoveToGroup => "Move to group".to_string(),
        // The row's own name, which the row is saying directly above the menu,
        // highlighted. Repeating it stacked the same words twice and spent a
        // line of a 216px box on nothing (the user's screenshot).
        WebTabMenuPage::Root => String::new(),
    }
}

/// Menu ids that TURN THE PAGE rather than run a verb.
///
/// Resolved BEFORE the action router, because these two are the only ids that
/// must leave the menu open — everything else in this menu closes it. A pure
/// function so "which ids are navigation" is one list rather than a shape
/// inside the dispatch closure.
fn web_tab_menu_page_turn(id: &str) -> Option<WebTabMenuPage> {
    match id {
        "webtab-move" => Some(WebTabMenuPage::MoveToGroup),
        "webtab-move-back" => Some(WebTabMenuPage::Root),
        _ => None,
    }
}

/// What a rail-menu item id MEANS, resolved against the row it was raised on.
///
/// Pure, and the ONLY router: the mouse path dispatches this and nothing else,
/// so a test can prove "the split item drives the split verb" without a live
/// webview. An id the row does not own resolves to `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WebTabMenuAction {
    NewTab,
    NewTabAbove(u64),
    ReopenClosedTabs,
    ReloadTab(u64),
    HardReloadTab(u64),
    CopyTabUrl(u64),
    CloseTab(u64),
    CloseOtherTabs(u64),
    CloseTabsBelow(u64),
    DuplicateTab(u64),
    /// File this tab into the group headed by that tab, or back to the root
    /// with `None`.
    MoveToGroup(u64, Option<u64>),
    SplitWithActiveTab(u64),
    /// A blank tab INSIDE the group this row heads — the head row's "+", which
    /// is what replaced the folder header's.
    NewTabInGroup(u64),
    /// Rename THIS ROW. One verb, because the rail has one rename
    /// ([`ShellState::web_tab_begin_rename`]).
    RenameRow(WebTabMenuTarget),
    ToggleGroup(u64),
    /// Close every row in the group this one heads — not the head itself, which
    /// is a page the user is reading.
    CloseGroupTabs(u64),
    /// Take the group apart without closing anything: the members move up one
    /// level and the head stays a tab.
    DisbandGroup(u64),
}

fn web_tab_menu_action(target: &WebTabMenuTarget, id: &str) -> Option<WebTabMenuAction> {
    match target {
        WebTabMenuTarget::Tab(tab) => {
            let tab = *tab;
            match id {
                "webgroup-new-tab" => Some(WebTabMenuAction::NewTabInGroup(tab)),
                "webgroup-toggle" => Some(WebTabMenuAction::ToggleGroup(tab)),
                "webgroup-close-tabs" => Some(WebTabMenuAction::CloseGroupTabs(tab)),
                "webgroup-disband" => Some(WebTabMenuAction::DisbandGroup(tab)),
                "webtab-new" => Some(WebTabMenuAction::NewTab),
                "webtab-new-above" => Some(WebTabMenuAction::NewTabAbove(tab)),
                "webtab-reopen" => Some(WebTabMenuAction::ReopenClosedTabs),
                "webtab-reload" => Some(WebTabMenuAction::ReloadTab(tab)),
                "webtab-reload-hard" => Some(WebTabMenuAction::HardReloadTab(tab)),
                "webtab-copy-url" => Some(WebTabMenuAction::CopyTabUrl(tab)),
                "webtab-close" => Some(WebTabMenuAction::CloseTab(tab)),
                "webtab-close-others" => Some(WebTabMenuAction::CloseOtherTabs(tab)),
                "webtab-close-below" => Some(WebTabMenuAction::CloseTabsBelow(tab)),
                "webtab-duplicate" => Some(WebTabMenuAction::DuplicateTab(tab)),
                "webtab-rename" => Some(WebTabMenuAction::RenameRow(WebTabMenuTarget::Tab(tab))),
                "webtab-split" => Some(WebTabMenuAction::SplitWithActiveTab(tab)),
                "webtab-move-root" => Some(WebTabMenuAction::MoveToGroup(tab, None)),
                _ => id
                    .strip_prefix("webtab-move:")
                    .and_then(|head| head.parse::<u64>().ok())
                    .map(|head| WebTabMenuAction::MoveToGroup(tab, Some(head))),
            }
        }
    }
}

/// The tabs a menu action CLOSES — the whole answer, for every close verb the
/// menu offers, and the only derivation of it.
///
/// [`web_tab_menu_items`] counts this to write the label; the dispatch
/// ([`ShellState::apply_web_tab_menu_action`]) takes exactly this and nothing
/// else. A re-derivation at the call site is how "Close 2 other tabs" comes to
/// close five, which is the bulk-close dishonesty the user forbade outright —
/// so the call site owns no list at all, it owns a call to this.
///
/// The app tab is never in a close plan, on any verb: it is the app's own page
/// and closing it is the ⏻/Ctrl+C path, not a tab verb's business.
fn web_tab_menu_close_plan(tabs: &[WebTabScopeRow], action: &WebTabMenuAction) -> Vec<u64> {
    match action {
        WebTabMenuAction::CloseTab(tab_id) => tabs
            .iter()
            .filter(|(id, _)| id == tab_id && *id != WEB_TAB_APP_TAB_ID)
            .map(|(id, _)| *id)
            .collect(),
        WebTabMenuAction::CloseOtherTabs(tab_id) => web_tab_close_others_targets(tabs, *tab_id),
        WebTabMenuAction::CloseTabsBelow(tab_id) => web_tab_close_below_targets(tabs, *tab_id),
        WebTabMenuAction::CloseGroupTabs(head) => web_tab_group_close_targets(tabs, *head),
        // Not a close verb. An empty plan is the honest answer, and the arm
        // that runs it never asks. `DisbandGroup` is listed HERE deliberately:
        // taking a group apart moves its members up a level, it does not close
        // them, and a close plan is the one place that could quietly turn
        // "remove the organization" into "delete the content".
        WebTabMenuAction::NewTab
        | WebTabMenuAction::NewTabAbove(_)
        | WebTabMenuAction::ReopenClosedTabs
        | WebTabMenuAction::ReloadTab(_)
        | WebTabMenuAction::HardReloadTab(_)
        | WebTabMenuAction::CopyTabUrl(_)
        | WebTabMenuAction::DuplicateTab(_)
        | WebTabMenuAction::MoveToGroup(_, _)
        | WebTabMenuAction::SplitWithActiveTab(_)
        | WebTabMenuAction::NewTabInGroup(_)
        | WebTabMenuAction::RenameRow(_)
        | WebTabMenuAction::ToggleGroup(_)
        | WebTabMenuAction::DisbandGroup(_) => Vec::new(),
    }
}

// ===== the ychrome PROFILE switcher (both surfaces) =====

/// A profile's display name. The ephemeral jar is "Temporary" everywhere the
/// user meets it (the picker card, this dropdown) — "temp" is the wire name.
fn web_profile_display_name(profile: &str) -> String {
    if yggterm_core::web_profile::web_profile_is_ephemeral(profile) {
        "Temporary".to_string()
    } else {
        profile.to_string()
    }
}

/// Does a session ROW in the cwd tree wear a profile chip?
///
/// THE ONE OWNER of the deliberate difference between the three surfaces that
/// draw a profile. The rail header badge and the classic strip badge are the
/// switcher's ENTRY POINTS and therefore draw for every identity — a switcher
/// you cannot reach because you never chose a profile is not a switcher. The
/// cwd tree's chip is a LABEL beside a session title in a long list, so it
/// speaks only when the identity says something the row does not already:
/// "default" is the absence of a choice, and the ephemeral jar wears its ⏲ on
/// the surface itself.
///
/// All three read the SAME identity ([`ShellState::web_surface_session_profile`]);
/// only this predicate is allowed to make one of them quieter, and it is
/// spelled here once.
fn web_profile_earns_row_badge(profile: &str) -> bool {
    !profile.is_empty()
        && profile != yggterm_core::web_profile::WEB_PROFILE_DEFAULT
        && profile != WEB_SURFACE_TEMP_PROFILE
}

/// The profile dropdown's rows: avatar + name, current one marked. ONE builder
/// for BOTH anchor sites — the rail header badge and the classic strip badge
/// raise the same list or they are not the same feature.
fn web_profile_switcher_menu_items(profiles: &[String], current: &str) -> Vec<RowMenuItem> {
    profiles
        .iter()
        .map(|profile| {
            let item = RowMenuItem::hinted(
                format!("webprofile:{profile}"),
                format!(
                    "{}  {}{}",
                    web_surface_profile_avatar(profile),
                    web_profile_display_name(profile),
                    if profile == current { "  ✓" } else { "" },
                ),
                web_profile_display_name(profile).chars().next(),
            );
            if profile == current {
                item.emphasized()
            } else {
                item
            }
        })
        .collect()
}

/// Every profile the switcher offers: the host's jars, plus the ephemeral one
/// (which has no directory by design), plus whatever the surface is on right
/// now — a surface running under a profile the dropdown did not list would be
/// a menu that cannot see its own ✓.
fn web_profile_switcher_choices(current: &str) -> Vec<String> {
    let mut names = enumerate_web_surface_profiles();
    names.push(WEB_SURFACE_TEMP_PROFILE.to_string());
    names.push(current.to_string());
    names.retain(|name| !name.is_empty());
    names.sort();
    names.dedup();
    names
}

/// May this client take `target_profile`'s jar?
///
/// The rule the whole switch hangs on: ONE `WebContext` per profile. The daemon
/// hands out the single-writer lock (`yggterm-server::profile_write_lock`), so
/// this reads its report rather than guessing — and REFUSES by name when
/// somebody else holds it. Silently opening a second writer corrupts cookies,
/// logins and IndexedDB for both clients.
///
/// Ephemeral profiles need no lock (their context is in memory, nothing is
/// shared), and a lock this very client already holds is not a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WebProfileSwitchGate {
    Allowed,
    Refused {
        holder_client_id: String,
        holder_pid: u32,
    },
}

fn web_profile_switch_gate(
    target_profile: &str,
    locks: &[(String, String, u32)],
    self_client_id: &str,
    self_pid: u32,
) -> WebProfileSwitchGate {
    let target = normalize_web_surface_profile(Some(target_profile));
    if yggterm_core::web_profile::web_profile_is_ephemeral(&target) {
        return WebProfileSwitchGate::Allowed;
    }
    let Some((_, holder_client_id, holder_pid)) = locks
        .iter()
        .find(|(profile, _, _)| normalize_web_surface_profile(Some(profile)) == target)
    else {
        return WebProfileSwitchGate::Allowed;
    };
    if holder_client_id == self_client_id && *holder_pid == self_pid {
        return WebProfileSwitchGate::Allowed;
    }
    WebProfileSwitchGate::Refused {
        holder_client_id: holder_client_id.clone(),
        holder_pid: *holder_pid,
    }
}

/// The refusal, in a sentence that names the holder. "Could not switch" teaches
/// the user nothing; naming the client and pid tells them exactly what to close.
fn web_profile_switch_refusal_message(
    profile: &str,
    holder_client_id: &str,
    holder_pid: u32,
) -> String {
    format!(
        "The \"{}\" profile's cookie jar is held by another live client ({holder_client_id}, pid {holder_pid}). \
         Switching now would open a second writer on one jar and corrupt it, so this surface stays on its profile.",
        web_profile_display_name(profile),
    )
}

/// The switch, decided — the ONE thing the async path acts on.
///
/// Pure, and shaped so "a refusal leaves the surface untouched" is structural
/// rather than a promise: `Refuse` carries no profile, so there is nothing for
/// the caller to apply. Only `Switch` names a target, and the only way to reach
/// it is a gate that said [`WebProfileSwitchGate::Allowed`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum WebProfileSwitchPlan {
    Switch(String),
    Refuse(String),
}

fn web_profile_switch_plan(
    target_profile: &str,
    gate: WebProfileSwitchGate,
) -> WebProfileSwitchPlan {
    match gate {
        WebProfileSwitchGate::Allowed => WebProfileSwitchPlan::Switch(target_profile.to_string()),
        WebProfileSwitchGate::Refused {
            holder_client_id,
            holder_pid,
        } => WebProfileSwitchPlan::Refuse(web_profile_switch_refusal_message(
            target_profile,
            &holder_client_id,
            holder_pid,
        )),
    }
}

/// Why the native-surface reconciler must DESTROY a live webview and build a
/// fresh one, or `None` when it can keep the one it has.
///
/// Three facts are fixed per `WebContext` and a live view cannot adopt them:
/// the SOCKS proxy, the storage PROFILE, and (WebKitGTK's stalled in-place
/// reload) a reload request. This is the one owner of both the decision and the
/// reason string it journals — they used to be two encodings of one rule, which
/// is how a condition and its `reason` drift apart.
///
/// It is also what makes the PROFILE SWITCH work: retargeting a surface's tabs
/// onto a new profile makes every one of them answer `profile_changed` here, so
/// the old context is destroyed and the new one opens on the new jar.
///
/// ## The fourth reason: `policy_arrived`
///
/// A FOURTH fact binds at creation and no live view can adopt it either — the
/// app's userscripts and adblock ruleset, which ride `init_script`. A surface
/// built while the policy endpoint was unreachable (a ychrome daemon handover
/// strands the GUI on the retired daemon's control port; the fetch exhausts
/// `MAX_POLICY_FETCH_ATTEMPTS` and the gate opens unblocked rather than never
/// opening) therefore runs with NO adblock and NO userscripts for its whole
/// life — reloading the page does not fix it, only rebuilding the webview does.
/// So when the policy does land, the surface is rebuilt under it.
///
/// ⚠ It is asked LAST, and only of a surface nobody is looking at. The rebuild
/// is visible — the page reloads and its scroll and form state go with it — so
/// a background tab, a stashed session and an agent's headless surface are
/// repaired the moment the policy lands, while the page in front of the user is
/// left alone until they next leave it. Deferring is not "never": the same rule
/// fires on the tick after it stops being visible.
#[allow(clippy::too_many_arguments)]
fn web_surface_recreate_reason(
    applied_socks_port: Option<u16>,
    applied_profile: &str,
    applied_reload_nonce: u64,
    applied_policy_settled: bool,
    applied_visible: bool,
    socks_port: Option<u16>,
    profile: &str,
    reload_nonce: u64,
    policy_ready: bool,
) -> Option<&'static str> {
    if applied_socks_port != socks_port {
        Some("socks_port_changed")
    } else if applied_profile != profile {
        Some("profile_changed")
    } else if applied_reload_nonce != reload_nonce {
        Some("reload")
    } else if !applied_policy_settled && policy_ready && !applied_visible {
        Some("policy_arrived")
    } else {
        None
    }
}

/// Who gets the keyboard back when a transient overlay closes.
///
/// The borrow-and-give-back doctrine: an overlay may hold the keyboard for
/// exactly its own lifetime and not one tick longer. When the viewport is
/// showing a TERMINAL, that terminal is who the keys belonged to, so closing
/// any menu hands them straight back; a document/web viewport owns its own
/// focus and must not be yanked. One owner for every menu in the app — the cwd
/// tree's row menu, the rail's, and the profile dropdown's.
fn overlay_focus_giveback_session(
    view_mode: WorkspaceViewMode,
    active_session_path: Option<&str>,
) -> Option<String> {
    (view_mode == WorkspaceViewMode::Terminal)
        .then(|| active_session_path.map(str::to_string))
        .flatten()
}

/// Dismiss one shell-owned floating menu — the ONE terminus for every dismissal
/// that is not an item pick.
///
/// Outside-click (each mount's `on_close`) and Escape both land here, so the two
/// gestures cannot drift into two different ideas of what dismissing costs: the
/// menu closes and the keyboard goes straight back to whoever lent it. An item
/// PICK is deliberately not routed here — an action may open a field or a dialog
/// that wants the keys, so it closes the menu on its own terms.
fn dismiss_menu(mut state: Signal<ShellState>, menu: ShellMenu) {
    let active_terminal_session = state.with_mut_counted(|shell| {
        match menu {
            ShellMenu::WebProfile => {
                shell.close_web_profile_switcher();
            }
            ShellMenu::WebTab => {
                shell.close_web_tab_context_menu();
            }
            ShellMenu::Row => {
                shell.close_context_menu();
            }
            ShellMenu::AppPane => {
                shell.close_app_pane_context_menu();
            }
        }
        overlay_focus_giveback_session(
            shell.server.active_view_mode(),
            shell.server.active_session_path(),
        )
    });
    // Only the cwd tree's menu is raised FROM the sidebar, so only its dismissal
    // can leave the sidebar holding a keyboard claim. The rail, the profile
    // dropdown and a contributed pane's rows never took one.
    if menu == ShellMenu::Row {
        clear_sidebar_keyboard_owner();
    }
    if let Some(session_path) = active_terminal_session {
        schedule_terminal_focus_after_activation(state, session_path);
    }
}

/// Dismiss whichever menu is on top. Returns false when there was none, which is
/// what keeps a menu-less Escape flowing to the surface that wanted it.
fn dismiss_top_menu(state: Signal<ShellState>) -> bool {
    let Some(menu) = state.with(|shell| shell.top_menu()) else {
        return false;
    };
    dismiss_menu(state, menu);
    true
}

/// Run a document-surface edit action on the focused editor.
///
/// `execCommand` is the only route that edits a textarea *through the undo
/// stack* — a manual `value` splice would make Ctrl+Z jump over the edit. Copy
/// and cut also make yggterm's own WebKit the clipboard owner, which is exactly
/// the case the paste deadlock fix made safe.
fn document_surface_edit_script(action: &str) -> String {
    let command = match action {
        "copy" => "copy",
        "cut" => "cut",
        "select-all" => "selectAll",
        _ => "paste",
    };
    format!(
        r#"
        (() => {{
          try {{
            const editors = Array.from(document.querySelectorAll("textarea[data-document-editor]"));
            const active = document.activeElement;
            const ta = (active && active.matches && active.matches("textarea[data-document-editor]"))
              ? active
              : editors[0];
            if (!ta) return "no_editor";
            ta.focus();
            if ({select_all}) {{ ta.select(); return "selected"; }}
            return document.execCommand({command:?}) ? "ok" : "refused";
          }} catch (_e) {{ return "error"; }}
        }})();
        "#,
        select_all = if command == "selectAll" {
            "true"
        } else {
            "false"
        },
        command = command,
    )
}
/// The opener id of the agent-CLI submenu. `ALT, E, S` reaches it.
const OPEN_SESSION_MENU_ID: &str = "open-session-here";
/// The opener id of the libyggterm-app submenu.
const OPEN_APP_MENU_ID: &str = "open-app-here";

/// One entry per REGISTERED agent CLI, in registry order.
///
/// ⚖ Derived from `AGENT_CLIS`, never hand-listed. Before this there were two
/// hardcoded pushes (`New Codex Session Here`, `New Claude Code Session Here`)
/// in two branches of this function, plus a third copy in the start page and a
/// fourth in the KeyTips scope — four places a new CLI had to be remembered, in
/// a codebase whose whole harness spec exists because that shape keeps costing
/// bugs. A CLI that registers a descriptor now appears here by construction.
///
/// `here` distinguishes the two callers: a SESSION row opens the new session in
/// that session's cwd ("… Here"), a folder row opens it in the folder.
fn agent_session_menu_items(here: bool) -> Vec<RowMenuItem> {
    yggterm_core::agent_cli::AGENT_CLIS
        .iter()
        .map(|descriptor| {
            let label = if here {
                format!("{} Here", descriptor.new_session_label())
            } else {
                descriptor.new_session_label()
            };
            RowMenuItem::new(
                format!("{NEW_AGENT_MENU_PREFIX}{}", descriptor.slug),
                label,
                descriptor.menu_hint,
            )
        })
        .collect()
}

/// The id prefix carrying which CLI a "New … Session" entry means.
///
/// One shape for all nine, so the dispatcher is one descriptor lookup rather
/// than an arm per CLI. It replaced `new-codex-here` / `new-claude-here` /
/// `new-session` / `new-claude-code` — four ids for one verb.
const NEW_AGENT_MENU_PREFIX: &str = "new-agent:";

/// Verbs CONTRIBUTED by the libyggterm apps on this row's host.
///
/// Same registry as the titlebar `+` menu and the start page — a purged app
/// leaves all three at once. The manifest's `keytip` is the requested letter
/// (libyggterm-surfaces spec §10). yggterm contributes NO app-specific chrome
/// of its own here; the list is whatever the host's `~/.yggterm/apps/*.json`
/// manifests declare.
fn libyggterm_app_menu_items(apps: &[AppManifest], here: bool) -> Vec<RowMenuItem> {
    app_row_spawn_entries(apps)
        .into_iter()
        .map(|(app, verb)| {
            let label = if here {
                format!("{} Here", verb.label)
            } else {
                verb.label.clone()
            };
            RowMenuItem::hinted(
                format!("app:{}:{}", app.name, verb.id),
                label,
                verb.keytip
                    .chars()
                    .next()
                    .or_else(|| app.keytip.chars().next()),
            )
        })
        .collect()
}

/// The page a row menu is showing: the root list, or one opener's children.
///
/// Mirrors [`WebTabMenuPage`] deliberately — one submenu idiom in the app, not
/// two. `Some(opener_id)` is a page turn in the SAME overlay at the SAME anchor.
type RowMenuPage = Option<String>;

/// Flatten the row-menu TREE to the page the mouse is looking at.
///
/// The KeyTip layer never calls this: it is handed the whole tree, which is why
/// `ALT,E,S,L` resolves without the submenu having been drawn.
fn row_menu_page_items(items: &[RowMenuItem], page: &RowMenuPage) -> Vec<RowMenuItem> {
    let Some(opener) = page.as_deref() else {
        return items.to_vec();
    };
    let Some(parent) = items.iter().find(|item| item.id == opener) else {
        // The opener vanished between frames (its app was purged, the row
        // changed kind). Falling back to the root beats drawing an empty box.
        return items.to_vec();
    };
    let mut page_items = vec![
        RowMenuItem::new(row_menu_back_id(opener), "Back", 'b').icon(ShellIcon::Back),
        RowMenuItem::divider(),
    ];
    page_items.extend(parent.submenu.iter().cloned());
    page_items
}

/// The id that turns a submenu page back to the root.
fn row_menu_back_id(opener: &str) -> String {
    format!("{opener}/__back")
}

/// The page this id turns to, if it is a navigation id rather than a verb.
///
/// Resolved BEFORE the action router, exactly as `web_tab_menu_page_turn` is:
/// these are the only ids that leave the menu OPEN.
fn row_menu_page_turn(items: &[RowMenuItem], id: &str) -> Option<RowMenuPage> {
    if id.ends_with("/__back") {
        return Some(None);
    }
    items
        .iter()
        .find(|item| item.id == id && !item.submenu.is_empty())
        .map(|item| Some(item.id.clone()))
}

/// Build the row menu for `row`. Pure: same inputs, same menu, in a stable order
/// — which is what makes the KeyTip letters stable too (invariant 1).
/// What this row is to a row set, for the menu that offers to take it apart.
///
/// ⚖ Computed by the caller, which can see the ARRANGEMENT; a row alone cannot
/// answer it. Depth would be a tempting proxy and a wrong one: a cwd-tree
/// session row is nested too, and offering to un-group it there names a
/// structure that surface does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowSetMenuRole {
    /// Heads a set — un-grouping DISSOLVES it and promotes its members.
    Head,
    /// Sits in someone's set — un-grouping removes just this row.
    Member,
    /// In no set at all.
    None,
}

fn row_menu_items(
    row: &BrowserRow,
    apps: &[AppManifest],
    keep_alive_plan: Option<&KeepAlivePlan>,
    split_group_members: &[(String, String)],
    split_candidate_count: usize,
    can_move_selected_document: bool,
    can_remove_saved_ssh_target: bool,
    row_set_role: RowSetMenuRole,
) -> Vec<RowMenuItem> {
    let mut items: Vec<RowMenuItem> = Vec::new();
    let is_live_sessions_group = row.full_path == "__live_sessions__";
    let can_create_in_context = !row.full_path.starts_with("__live_")
        && matches!(
            row.kind,
            BrowserRowKind::Group | BrowserRowKind::Document | BrowserRowKind::Separator
        );
    let can_rename = context_menu_allows_rename(row);
    let can_regenerate_copy = !is_live_sessions_group
        && matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Group);
    let is_live_runtime_row = row_supports_keep_alive(row);
    let keep_alive_active = keep_alive_plan.is_some_and(|plan| plan.all_keep_alive);
    let keep_alive_count = keep_alive_plan.map(|plan| plan.paths.len()).unwrap_or(0);
    let keep_alive_suffix = if keep_alive_count > 1 {
        format!(" ({keep_alive_count} sessions)")
    } else {
        String::new()
    };
    // ROW SET actions. ⚖ A split's `Ungroup` sits below and means something
    // else entirely — that one takes apart a VIEW, this one an ARRANGEMENT —
    // which is exactly why `DESIGN.md` forbids the two ever sharing a noun in
    // the model. They can share a verb in a menu because only one of them is
    // ever offered on a given row.
    match row_set_role {
        RowSetMenuRole::Head => {
            items.push(RowMenuItem::new("ungroup-row-set", "Ungroup", 'u'));
        }
        RowSetMenuRole::Member => {
            items.push(RowMenuItem::new("leave-row-set", "Remove from group", 'u'));
        }
        RowSetMenuRole::None => {}
    }
    // Split-view group actions ([[campaign-split-view-groups]]).
    if !split_group_members.is_empty() {
        // Compound split row: structural ops only — there is no × on the row
        // itself, so a built workspace is hard to lose.
        items.push(RowMenuItem::new("ungroup-split", "Ungroup", 'u'));
        for (pane_path, pane_label) in split_group_members {
            items.push(RowMenuItem::hinted(
                format!("close-split-pane:{pane_path}"),
                format!("Close pane: {pane_label}"),
                None,
            ));
        }
        items.push(RowMenuItem::new("close-split-group", "Close all panes", 'a').destructive());
    } else if split_candidate_count >= 2 {
        items.push(RowMenuItem::new(
            "split-side-by-side",
            format!("Split side by side ({split_candidate_count} panes)"),
            'b',
        ));
        items.push(RowMenuItem::new(
            "split-stacked",
            format!("Split stacked ({split_candidate_count} panes)"),
            'k',
        ));
    }
    if can_rename {
        items.push(RowMenuItem::new(
            "rename-session",
            if row.kind == BrowserRowKind::Session {
                "Rename Session"
            } else {
                "Rename"
            },
            'r',
        ));
    }
    if is_live_sessions_group {
        items.push(RowMenuItem::new("close-all-live-sessions", "Close All…", 'o').destructive());
    } else if is_remote_machine_group_row(row) {
        items.push(RowMenuItem::new(
            "refresh-remote-sessions",
            "Refresh Remote Sessions",
            'f',
        ));
        if can_remove_saved_ssh_target {
            items.push(RowMenuItem::divider());
            items.push(RowMenuItem::new("delete", "Delete…", 'x').destructive());
        }
    } else if can_create_in_context {
        if can_move_selected_document {
            items.push(
                RowMenuItem::new("move-selected-here", "Move Selected Here", 'm').emphasized(),
            );
        }
        items.push(RowMenuItem::new("add-folder", "Add Folder", 'f'));
        items.push(RowMenuItem::new("new-terminal", "New Terminal", 't'));
        items.push(
            RowMenuItem::new(OPEN_SESSION_MENU_ID, "Open Session", 's')
                .submenu(agent_session_menu_items(false)),
        );
        if !app_row_spawn_entries(apps).is_empty() {
            items.push(
                RowMenuItem::new(OPEN_APP_MENU_ID, "Open libyggterm App", 'b')
                    .submenu(libyggterm_app_menu_items(apps, false)),
            );
        }
        items.push(RowMenuItem::divider());
        items.push(RowMenuItem::new("add-separator", "Add Separator", 'p'));
    } else if row.kind == BrowserRowKind::Session {
        if is_live_runtime_row {
            items.push(RowMenuItem::new("redraw-terminal", "Redraw Terminal", 'd'));
            items.push(RowMenuItem::new("restart-session", "Restart Session", 'e'));
            items.push(RowMenuItem::new(
                if keep_alive_active {
                    "stop-keep-alive"
                } else {
                    "keep-alive"
                },
                if keep_alive_active {
                    format!("Stop Keeping Alive{keep_alive_suffix}")
                } else {
                    format!("Keep Alive{keep_alive_suffix}")
                },
                'k',
            ));
            items.push(RowMenuItem::divider());
            // Open a sibling session in THIS session's cwd, without hunting
            // through the cwd tree. Every one of these lands the new row directly
            // below this one ([`spawn_start_session_for_row`]).
            items.push(RowMenuItem::new(
                "open-terminal-here",
                "Open Terminal Here",
                't',
            ));
            // ⚖ TWO LAYERS, recorded 2026-08-08. Nine agent CLIs and four
            // app verbs as siblings is a list nobody can read, and it grows
            // every time a CLI ships. `Open Terminal Here` stays flat — it is
            // the one entry that is not a choice between vendors.
            items.push(
                RowMenuItem::new(OPEN_SESSION_MENU_ID, "Open Session Here", 's')
                    .submenu(agent_session_menu_items(true)),
            );
            if !app_row_spawn_entries(apps).is_empty() {
                items.push(
                    RowMenuItem::new(OPEN_APP_MENU_ID, "Open libyggterm App Here", 'b')
                        .submenu(libyggterm_app_menu_items(apps, true)),
                );
            }
        }
        items.push(RowMenuItem::new(
            "regenerate-copy",
            "Regenerate Title/Summary",
            'g',
        ));
        // ⚠ Re-hinted off `s`, which the submenu above now takes. Left alone,
        // the ladder would silently hand this item `i` (its hint denied, then
        // the first free letter of its title) and `ALT,E,S` would quietly stop
        // meaning "Edit Summary" — the exact "a chord never silently changes its
        // target" invariant the KeyTips spec names.
        items.push(RowMenuItem::new("edit-summary", "Edit Summary", 'y'));
        items.push(RowMenuItem::divider());
        items.push(
            RowMenuItem::new(
                "delete-session",
                if is_live_runtime_row {
                    "Close Terminal…"
                } else {
                    "Delete…"
                },
                'x',
            )
            .destructive(),
        );
    }
    if can_regenerate_copy && row.kind != BrowserRowKind::Session {
        items.push(RowMenuItem::divider());
        items.push(RowMenuItem::new(
            "regenerate-copy",
            "Regenerate Title/Summary",
            'g',
        ));
        items.push(RowMenuItem::new("regenerate-title", "Regenerate Titles", 'i').emphasized());
        items
            .push(RowMenuItem::new("regenerate-summary", "Regenerate Summaries", 'z').emphasized());
    }
    if is_workspace_row(row) {
        items.push(RowMenuItem::divider());
        items.push(RowMenuItem::new("delete", "Delete…", 'x').destructive());
    }
    items
}
/// The `(path, label)` pair per pane when `row` IS a compound split row; empty
/// otherwise. Same lookup [`split_group_cells_for_row`] does, over the raw state
/// rather than a `RenderSnapshot`, so `ShellState::snapshot` can build the row
/// menu while it is still assembling itself.
fn split_group_member_labels(
    split_groups: &[SplitGroup],
    live_sessions: &[ManagedSessionView],
    row: &BrowserRow,
) -> Vec<(String, String)> {
    if !row.full_path.starts_with("split://") {
        return Vec::new();
    }
    let Some(group_id) = row.session_id.as_deref() else {
        return Vec::new();
    };
    let Some(group) = split_groups.iter().find(|group| group.group_id == group_id) else {
        return Vec::new();
    };
    group
        .members
        .iter()
        .map(|member| {
            let label = live_sessions
                .iter()
                .find(|session| session.session_path == member.session)
                .map(|session| session.title.clone())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| member.session.clone());
            (member.session.clone(), label)
        })
        .collect()
}
/// The live rows a "Split …" item would compound: the selection (or the clicked
/// row alone), minus anything already in a group. One rule, read by the menu
/// builder (for the item's count) and by the dispatcher (for the actual paths).
fn split_candidate_paths_for(
    row: &BrowserRow,
    selected_tree_paths: &[String],
    split_groups: &[SplitGroup],
) -> Vec<String> {
    let selected: Vec<String> = if selected_tree_paths.is_empty() {
        vec![row.full_path.clone()]
    } else {
        selected_tree_paths.to_vec()
    };
    selected
        .into_iter()
        .filter(|path| is_hot_terminal_sidebar_path(path))
        .filter(|path| !split_groups.iter().any(|group| group.contains(path)))
        .collect()
}
/// Run one row-menu item against `row`. THE terminus for the row menu: a click on
/// the item and its ALT chord (`ALT,E,<letter>`) both arrive here with the same
/// id, so the keyboard can never reach an action the mouse cannot, or vice versa.
/// Select the visible terminal host for `session_path` and highlight all of it.
fn terminal_viewport_select_all_script(session_path: &str) -> String {
    format!(
        r#"
        (() => {{
          try {{
            const sessionPath = {session_path:?};
            const registry = window.__yggtermXtermHosts || {{}};
            const entry = Object.values(registry)
              .filter((e) => e && e.term && e.sessionPath === sessionPath)
              .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0))[0];
            if (entry && entry.term && typeof entry.term.selectAll === "function") {{
              try {{ entry.term.focus && entry.term.focus(); }} catch (_f) {{}}
              entry.term.selectAll();
            }}
          }} catch (_e) {{}}
        }})();
        "#
    )
}

/// Send back the visible terminal host's current selection text (empty if none).
fn terminal_viewport_get_selection_script(session_path: &str) -> String {
    format!(
        r#"
        (() => {{
          try {{
            const sessionPath = {session_path:?};
            const registry = window.__yggtermXtermHosts || {{}};
            const entry = Object.values(registry)
              .filter((e) => e && e.term && e.sessionPath === sessionPath)
              .sort((a, b) => (b.mountedAt || 0) - (a.mountedAt || 0))[0];
            const text = entry && entry.term && entry.term.getSelection
              ? String(entry.term.getSelection() || "")
              : "";
            dioxus.send(text);
          }} catch (_e) {{ dioxus.send(""); }}
        }})();
        "#
    )
}

/// Run a viewport (terminal) context-menu action against the active session.
///
/// Copy reads the xterm selection and routes it through the SAME clipboard
/// owner path as Ctrl+Shift+C (off-main owner + a "Copied N" notification —
/// which is what a document/web surface's native menu gives for free, and what
/// the terminal menu owes to match). Paste reuses the terminal paste; Select
/// All highlights the buffer.
fn dispatch_viewport_menu_action(mut state: Signal<ShellState>, action: String) {
    let (surface, session_path, trace_home) = state.with_mut_counted(|shell| {
        let surface = shell.context_menu_surface;
        let session = shell.server.active_session_path().map(str::to_string);
        let trace = perf_home_dir(&shell.bootstrap.settings_path);
        shell.close_context_menu();
        (surface, session, trace)
    });
    // A document surface edits its own DOM: route to execCommand so the edit
    // joins the editor's undo stack. `paste` goes through the off-main clipboard
    // read + an insert, because execCommand('paste') is refused by WebKit.
    if surface == Some(ViewportMenuKind::Document) {
        if action == "paste" {
            spawn(async move {
                let Ok(text) = read_native_clipboard_text_off_main().await else {
                    return;
                };
                let literal = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string());
                let _ = document::eval(&format!(
                    r#"
                    (() => {{
                      try {{
                        const editors = Array.from(document.querySelectorAll("textarea[data-document-editor]"));
                        const active = document.activeElement;
                        const ta = (active && active.matches && active.matches("textarea[data-document-editor]"))
                          ? active : editors[0];
                        if (!ta) return;
                        ta.focus();
                        // insertText keeps the undo stack and fires `input`, so the
                        // document channel sees the change like any keystroke.
                        if (!document.execCommand("insertText", false, {literal})) {{
                          const s = ta.selectionStart, e = ta.selectionEnd;
                          ta.value = ta.value.slice(0, s) + {literal} + ta.value.slice(e);
                          ta.selectionStart = ta.selectionEnd = s + {literal}.length;
                          ta.dispatchEvent(new Event("input", {{ bubbles: true }}));
                        }}
                      }} catch (_e) {{}}
                    }})();
                    "#
                ));
            });
            return;
        }
        let _ = document::eval(&document_surface_edit_script(&action));
        return;
    }
    if surface != Some(ViewportMenuKind::Terminal) {
        return;
    }
    let Some(session_path) = session_path else {
        return;
    };
    match action.as_str() {
        "paste" => {
            spawn(async move {
                let _ = paste_terminal_native_clipboard(state, &session_path).await;
            });
        }
        "select-all" => {
            let _ = document::eval(&terminal_viewport_select_all_script(&session_path));
        }
        "copy" => {
            spawn(async move {
                let mut eval =
                    document::eval(&terminal_viewport_get_selection_script(&session_path));
                let text = eval.recv::<String>().await.unwrap_or_default();
                if text.is_empty() {
                    safe_push_notification(
                        state,
                        NotificationTone::Info,
                        "Nothing to Copy",
                        "Select terminal text first.".to_string(),
                    );
                    return;
                }
                let chars = text.chars().count();
                if copy_terminal_selection_to_clipboard(&session_path, "copy", text, trace_home)
                    .is_ok_and(|outcome| outcome.served_method().is_some())
                {
                    safe_push_notification(
                        state,
                        NotificationTone::Success,
                        "Copied to Clipboard",
                        format!("Copied {chars} character(s) from the terminal selection."),
                    );
                }
            });
        }
        _ => {}
    }
}

/// A rail row's RIGHT-CLICK, from the raw event to the open menu. THE handler
/// for both row kinds — a tab row and a folder row differ only in the target
/// they name.
///
/// The rows do not spell this themselves on purpose. A handler that spells its
/// own opener can be wrapped (`if false { … }`) or emptied while the call-site
/// COUNT stays at two, which is exactly the bypass a "two rows call the opener"
/// source needle cannot see. With one handler the rows' bodies are a single
/// call each, and a lock can pin that body whole.
fn open_web_tab_menu_from_event(
    mut state: Signal<ShellState>,
    session_path: &str,
    target: WebTabMenuTarget,
    anchor: WebSurfaceChromeAnchor,
    evt: MouseEvent,
) {
    // The native WebKit menu must not also open, and the row's own click must
    // not select the row out from under the menu.
    evt.prevent_default();
    evt.stop_propagation();
    let coords = evt.client_coordinates();
    let session_path = session_path.to_string();
    state.with_mut_counted(|shell| {
        shell.open_web_tab_context_menu(&session_path, target, (coords.x, coords.y), anchor);
    });
}

/// A profile badge's CLICK, from the raw event to the open dropdown. THE
/// handler for BOTH anchor sites (the rail header badge and the classic strip
/// badge), for the same reason the rail rows share one: a badge that spells its
/// own opener can be neutered while the call-site count stays at two, and then
/// "two anchors, one menu" is true of the state slot and false of the buttons.
fn open_web_profile_switcher_from_event(
    mut state: Signal<ShellState>,
    session_path: &str,
    anchor: WebSurfaceChromeAnchor,
    evt: MouseEvent,
) {
    evt.stop_propagation();
    let coords = evt.client_coordinates();
    let session_path = session_path.to_string();
    state.with_mut_counted(|shell| {
        shell.open_web_profile_switcher(&session_path, anchor, (coords.x, coords.y));
    });
}

/// Run a WebTabs-rail menu item. The mouse's ONE terminus, and it dispatches
/// nothing the pure router ([`web_tab_menu_action`]) did not resolve — so an id
/// that reaches here is an id the menu actually offered on that row.
///
/// It owns NO verb of its own. Every action runs in
/// [`ShellState::apply_web_tab_menu_action`], where a headless test can compare
/// what a label counted against what actually closed; the one thing that cannot
/// live there is the split, which needs this `Signal` to open a pane.
fn dispatch_web_tab_menu_action(
    mut state: Signal<ShellState>,
    menu: WebTabContextMenu,
    id: String,
) {
    // PAGE TURNS FIRST, and they are the only ids that leave the menu standing.
    // "Move to folder ▸" is a submenu: the overlay stays where it is and shows
    // the other list. Resolved through the pure owner
    // ([`web_tab_menu_page_turn`]) so which ids navigate is a list, not a shape
    // buried in this closure.
    if let Some(page) = web_tab_menu_page_turn(&id) {
        state.with_mut_counted(|shell| shell.turn_web_tab_menu_page(page));
        return;
    }
    let Some(action) = web_tab_menu_action(&menu.target, &id) else {
        state.with_mut_counted(|shell| shell.close_web_tab_context_menu());
        return;
    };
    let session_path = menu.session_path.clone();
    state.with_mut_counted(|shell| shell.close_web_tab_context_menu());
    if state.with_mut_counted(|shell| shell.apply_web_tab_menu_action(&session_path, &action)) {
        return;
    }
    // The verbs that need the Signal.
    match &action {
        WebTabMenuAction::HardReloadTab(tab_id) => {
            let result = web_surface_native_id_for(&session_path, *tab_id)
                .ok_or_else(|| {
                    "web surface not live (session backgrounded or not yet revealed)".to_string()
                })
                .and_then(|native_id| {
                    dioxus_desktop::window().reload_web_surface_bypass_cache(native_id)
                });
            if let Err(error) = result {
                state.with_mut_counted(|shell| {
                    shell.push_notification(
                        NotificationTone::Warning,
                        "Could Not Reload Without Cache",
                        error,
                    )
                });
            }
            return;
        }
        // Both open through the ONE UI opener, so a tab opened from the menu is
        // typing-ready exactly like one opened from a "+" — which is the whole
        // "on new tab ANYWHERE in the tree the focus should be on the URL input
        // box" half of the report.
        WebTabMenuAction::NewTab => {
            open_web_surface_tab(state, &session_path, WebTabOpenRequest::blank());
            return;
        }
        WebTabMenuAction::NewTabAbove(tab_id) => {
            open_web_surface_tab(
                state,
                &session_path,
                WebTabOpenRequest::blank_above(*tab_id),
            );
            return;
        }
        WebTabMenuAction::ReopenClosedTabs => {
            let reopened =
                state.with_mut_counted(|shell| shell.web_surface_reopen_closed_tabs(&session_path));
            // ONE selection for the batch: reopening twelve tabs must not walk
            // the user through twelve fronts, and the selection is what
            // resolves that tab's egress (the same reason the duplicate
            // selects). `Opened`, because the user picked a verb, not a row.
            if let Some(first) = reopened.first() {
                select_web_surface_tab(state, session_path.clone(), *first, WebTabSelect::Opened);
            }
            return;
        }
        WebTabMenuAction::CopyTabUrl(tab_id) => {
            let url = state.peek().web_surface_tab_url(&session_path, *tab_id);
            let Some(url) = url else {
                return;
            };
            // The app's ONE clipboard owner. A second writer here would fight
            // the terminal's copy path for the selection.
            let copied = set_native_clipboard_contents(
                state,
                &YgguiClipboardContents::Text { text: url.clone() },
            );
            state.with_mut_counted(|shell| match copied {
                Ok(_) => {
                    shell.push_notification(NotificationTone::Success, "URL Copied", url.clone())
                }
                // Named, not swallowed: a copy that silently did nothing is
                // discovered at the paste, somewhere else entirely.
                Err(error) => shell.push_notification(
                    NotificationTone::Warning,
                    "Could Not Copy URL",
                    error.to_string(),
                ),
            });
            return;
        }
        _ => {}
    }
    if let WebTabMenuAction::SplitWithActiveTab(tab_id) = action {
        // The EXISTING intra-tab split, which had no UI entry until now:
        // pane 0 stays the session's surface, pane 1 is pinned to this tab.
        split_web_tab_into_pane(state, &session_path, tab_id, SplitAxis::SideBySide);
    }
    if let WebTabMenuAction::DuplicateTab(tab_id) = action {
        // Duplicate, then SELECT — the selection is what resolves the new tab's
        // egress and hands the reconciler a real URL to open. Without it the
        // copy opened on about:blank wearing the source's title, and the page
        // observer then wrote that blank back into the saved tree.
        if let Some(new_id) =
            state.with_mut_counted(|shell| shell.web_surface_duplicate_tab(&session_path, tab_id))
        {
            select_web_surface_tab(state, session_path.clone(), new_id, WebTabSelect::Opened);
        }
    }
}

/// Take a profile dropdown selection: gate it on the daemon's single-writer
/// lock, then either switch or REFUSE by name.
///
/// The lock report is a daemon round-trip, so it runs off the UI event loop —
/// but the refusal has to be decided BEFORE anything mutates, because "the
/// surface survives untouched" is the whole promise of a refusal.
fn spawn_web_profile_switch(mut state: Signal<ShellState>, session_path: String, profile: String) {
    let endpoint = state.peek().bootstrap.server_endpoint.clone();
    spawn(async move {
        let target = normalize_web_surface_profile(Some(&profile));
        let identity = yggterm_server::current_client_identity();
        let self_pid = std::process::id();
        let self_client_id = identity
            .client_id
            .clone()
            .unwrap_or_else(|| format!("anonymous:{self_pid}"));
        let gate = {
            let target = target.clone();
            let self_client_id = self_client_id.clone();
            task::spawn_blocking(move || {
                if yggterm_core::web_profile::web_profile_is_ephemeral(&target) {
                    // No jar, no shared state, nothing to lock.
                    return WebProfileSwitchGate::Allowed;
                }
                match yggterm_server::profile_write_lock_report(&endpoint) {
                    Ok(report) => {
                        web_profile_switch_gate(&target, &report.locks, &self_client_id, self_pid)
                    }
                    // Cannot ask who holds it ⇒ cannot promise we are alone on
                    // the jar. The reconciler's own acquire will fall back by
                    // role, so a switch here would hand the user an outcome
                    // nobody decided; refuse and name why.
                    Err(error) => WebProfileSwitchGate::Refused {
                        holder_client_id: format!("unknown ({error})"),
                        holder_pid: 0,
                    },
                }
            })
            .await
            .unwrap_or(WebProfileSwitchGate::Refused {
                holder_client_id: "unknown (lock probe panicked)".to_string(),
                holder_pid: 0,
            })
        };
        match web_profile_switch_plan(&target, gate) {
            WebProfileSwitchPlan::Refuse(message) => {
                // Nothing to apply, by construction: the refusal carries a
                // sentence, never a profile. The surface stays exactly as it was.
                state.with_mut_counted(|shell| {
                    shell.push_notification(NotificationTone::Error, "Profile In Use", message);
                });
            }
            WebProfileSwitchPlan::Switch(profile) => {
                let switched = state
                    .with_mut(|shell| shell.switch_web_surface_profile(&session_path, &profile));
                if let Some(tabs) = switched.filter(|tabs| *tabs > 0) {
                    state.with_mut_counted(|shell| {
                        shell.push_notification(
                            NotificationTone::Success,
                            "Profile Switched",
                            format!(
                                "Now browsing as \"{}\"; {} reloading under the new profile.",
                                web_profile_display_name(&profile),
                                web_tab_count_phrase(tabs, "tab"),
                            ),
                        );
                    });
                }
            }
        }
    });
}

fn dispatch_row_menu_action(mut state: Signal<ShellState>, row: BrowserRow, id: String) {
    // A viewport-surface menu item (terminal Copy/Paste/Select-All) — routed by
    // its `viewport-` prefix to the surface, never to the row's session actions.
    if let Some(action) = id.strip_prefix("viewport-") {
        dispatch_viewport_menu_action(state, action.to_string());
        return;
    }
    // ⚠ NAVIGATION FIRST, before the action router — these are the only ids
    // that leave the menu OPEN. Without this the catch-all at the bottom would
    // treat "Open Session Here ▸" as an unknown verb and close the box, which
    // is a submenu that dismisses instead of opening.
    {
        let turn = state.with(|shell| {
            let items = shell.snapshot().row_menu_tree;
            row_menu_page_turn(&items, &id)
        });
        if let Some(page) = turn {
            state.with_mut_counted(|shell| shell.turn_row_menu_page(page));
            return;
        }
    }
    // A submenu leaf carries its opener's id as a prefix so node keys stay
    // unique across both levels; the verb below it is what dispatches.
    let id = match id.rsplit_once('/') {
        Some((_, leaf)) => leaf.to_string(),
        None => id,
    };
    // The creation-context row (a folder for a paper, a session's own cwd) — the
    // same row `open_context_menu` resolved when the menu opened.
    let context_row = state.with(|shell| {
        shell
            .context_menu_context_row
            .clone()
            .unwrap_or_else(|| resolve_creation_context_row(&shell.snapshot().rows, &row))
    });
    let preferred_agent_kind = preferred_agent_session_kind(&state.read().settings);
    // Payload-carrying ids (§ `RowMenuItem::id`).
    if let Some(pane_path) = id.strip_prefix("close-split-pane:") {
        let pane_path = pane_path.to_string();
        if let Some(group_id) = row.session_id.clone() {
            remove_split_pane(state, &group_id, &pane_path);
        }
        spawn_close_session_runtime(state, pane_path);
        return;
    }
    // ONE arm for every registered agent CLI. The nine "New … Session" entries
    // differ only in which descriptor they name, so they route through one
    // lookup — the four hardcoded ids this replaced (`new-session`,
    // `new-claude-code`, `new-codex-here`, `new-claude-here`) were four places
    // a tenth CLI would have had to be remembered.
    //
    // A folder row creates in the folder (`context_row`); a session row creates
    // in that session's own cwd (`row`), landing directly below it.
    if let Some(slug) = id.strip_prefix(NEW_AGENT_MENU_PREFIX) {
        let anchor = if row.kind == BrowserRowKind::Session {
            row.clone()
        } else {
            context_row.clone()
        };
        let kind = yggterm_core::agent_cli::AGENT_CLIS
            .iter()
            .find(|descriptor| descriptor.slug == slug)
            .map(|descriptor| descriptor.kind);
        match kind {
            // Codex means "whatever the user set as their default agent" — the
            // `AgentSessionProfile` setting picks the fork, exactly as the old
            // `new-session` arm did.
            Some(SessionKind::Codex) => {
                spawn_start_group_session(state, anchor, preferred_agent_kind)
            }
            Some(kind) => spawn_start_group_session(state, anchor, kind),
            // An id naming a CLI that is not registered cannot have been drawn
            // by this menu; closing is the honest response to a verb that does
            // not exist.
            None => state.with_mut_counted(|shell| shell.close_context_menu()),
        }
        return;
    }
    if id.starts_with("app:") {
        let mut parts = id.splitn(3, ':');
        let (Some(_), Some(app_name), Some(verb_id)) = (parts.next(), parts.next(), parts.next())
        else {
            return;
        };
        // Same registry the menu was drawn from: this row's machine.
        let entry =
            state.with(|shell| resolve_app_verb_for_row(shell, Some(&row), app_name, verb_id));
        if let Some((app, verb)) = entry {
            // cwd AND sidebar anchor both from the clicked row, like the "New …
            // Here" items above it.
            spawn_launch_app_verb_here(state, app, verb, Some(row.clone()));
        }
        return;
    }
    match id.as_str() {
        // ⛔ DISSOLVE, never cascade and never refuse. `DESIGN.md`: removing a
        // set's head promotes its members to where the head sat, in order. The
        // user asked to take the arrangement apart, not to be told about
        // bookkeeping and not to lose the rows inside it.
        "ungroup-row-set" => {
            let path = normalize_live_session_path(&row.full_path);
            state.with_mut_counted(|shell| {
                let members = shell.dissolve_row_set(&path);
                shell.last_action = format!("ungrouped {} row(s)", members.len());
                shell.sync_browser_settings();
            });
        }
        "leave-row-set" => {
            let path = normalize_live_session_path(&row.full_path);
            state.with_mut_counted(|shell| {
                shell.row_arrangement.detach(&path);
                shell.last_action = format!("removed {} from its group", row.label);
                shell.sync_browser_settings();
            });
        }
        "ungroup-split" => {
            if let Some(group_id) = row.session_id.clone() {
                ungroup_split_group(state, &group_id);
            }
        }
        "close-split-group" => {
            let members: Vec<String> = state.with_mut_counted(|shell| {
                let members = split_group_member_labels(
                    &shell.split_groups,
                    &shell.server.live_sessions(),
                    &row,
                )
                .into_iter()
                .map(|(path, _)| path)
                .collect();
                shell.close_context_menu();
                members
            });
            if let Some(group_id) = row.session_id.clone() {
                ungroup_split_group(state, &group_id);
            }
            for path in members {
                spawn_close_session_runtime(state, path);
            }
        }
        "split-side-by-side" | "split-stacked" => {
            let candidates = state.with_mut_counted(|shell| {
                let candidates = split_candidate_paths_for(
                    &row,
                    &shell
                        .selected_tree_paths
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                    &shell.split_groups,
                );
                shell.close_context_menu();
                candidates
            });
            let axis = if id == "split-side-by-side" {
                SplitAxis::SideBySide
            } else {
                SplitAxis::Stacked
            };
            create_split_group(state, candidates, axis);
        }
        "rename-session" => {
            state.with_mut_counted(|shell| shell.begin_tree_rename(&row));
            sync_active_terminal_input_policy(state);
        }
        "close-all-live-sessions" => {
            state.with_mut_counted(|shell| shell.open_live_sessions_close_all_dialog());
        }
        "refresh-remote-sessions" => {
            if let Some(machine_key) = row
                .full_path
                .strip_prefix("__remote_machine__/")
                .map(ToOwned::to_owned)
            {
                let label = row.label.clone();
                spawn_server_snapshot_action(
                    state,
                    format!("refreshing {label}"),
                    move |endpoint| refresh_remote_machine(&endpoint, &machine_key),
                );
            }
        }
        // One id, two rows: on a saved SSH machine row "Delete…" forgets the
        // target; on a workspace row it opens the delete dialog. The row decides,
        // exactly as the two mutually-exclusive render branches did.
        "delete" => {
            if is_remote_machine_group_row(&row) {
                queue_remove_saved_ssh_target(state, row.clone());
            } else {
                state.with_mut_counted(|shell| shell.open_context_menu_delete_for_row(&row, false));
            }
        }
        "move-selected-here" => {
            if let Some(placement) = context_menu_drop_placement(&context_row) {
                queue_move_selected_items_to_group(
                    state,
                    placement,
                    format!("near {}", context_row.label),
                );
            }
        }
        "add-folder" => queue_new_group_for_row(state, context_row),
        "add-separator" => queue_new_separator_for_row(state, row),
        "new-terminal" => spawn_start_group_session(state, context_row, SessionKind::Shell),
        // The "… Here" items on a live-session row use the RAW row, so the new
        // session inherits THAT session's cwd and lands directly below it.
        "open-terminal-here" => spawn_start_group_session(state, row, SessionKind::Shell),
        "keep-alive" | "stop-keep-alive" => {
            let keep_alive = id == "keep-alive";
            // Re-derive from the same function that labelled the item, so the
            // click can never write to a different set than the menu promised.
            let Some(plan) = state.with_mut_counted(|shell| {
                let plan = shell.context_menu_keep_alive_plan();
                if let Some(plan) = plan.as_ref() {
                    for path in &plan.paths {
                        mark_live_session_keep_alive_locally(shell, path, keep_alive);
                    }
                }
                shell.close_context_menu();
                shell.refresh_tree_debug("keep_alive_optimistic_toggle");
                plan
            }) else {
                return;
            };
            let target = if plan.paths.len() > 1 {
                format!("{} sessions", plan.paths.len())
            } else {
                row.label.clone()
            };
            let paths = plan.paths.clone();
            spawn_server_snapshot_action(
                state,
                if keep_alive {
                    format!("keeping {target} alive")
                } else {
                    format!("stopping keep-alive for {target}")
                },
                move |endpoint| {
                    // Exact paths, one request each, tree order. The LAST snapshot
                    // is the one the UI adopts. A failure must NOT abort the batch
                    // — the remaining sessions still get their request (a bulk
                    // keep-alive that stops at the first bad row silently strands
                    // the rest); errors aggregate and surface once, and any
                    // success still adopts a snapshot so the tree shows the
                    // daemon's truth for the rows that took.
                    let mut result = None;
                    let mut errors = Vec::new();
                    for path in &paths {
                        match set_session_keep_alive(&endpoint, path, keep_alive) {
                            Ok(snapshot) => result = Some(snapshot),
                            Err(error) => errors.push(format!("{path}: {error}")),
                        }
                    }
                    match result {
                        Some(snapshot) => Ok(snapshot),
                        None if !errors.is_empty() => Err(anyhow!(errors.join("; "))),
                        None => Err(anyhow!("no live session to keep alive")),
                    }
                },
            );
        }
        "redraw-terminal" => {
            let path = row.full_path.clone();
            let label = row.label.clone();
            state.with_mut_counted(|shell| shell.close_context_menu());
            spawn(async move {
                // FIX C: re-fetch the daemon's authoritative vt100 screen FIRST,
                // then re-fit the renderer. The plain renderer redraw only re-fits
                // + `term.refresh()`s the EXISTING client buffer, so when that
                // buffer is a stale "shadow" or a scooped buffer, repainting it
                // "does nothing". The content reconcile replays the daemon screen,
                // which promotes the source back to daemon_pty and closes the
                // broken-bottom/shadow. This is the user-initiated escape hatch,
                // so it runs unconditionally (the quiet/working gates only protect
                // the AUTOMATIC reconcile from recovery-churn).
                let endpoint = state.read().bootstrap.server_endpoint.clone();
                if let Ok(home) = resolve_yggterm_home() {
                    let _ = reconcile_terminal_from_daemon_for(endpoint, &path, &home).await;
                }
                let result = redraw_terminal_viewport_for(&path).await;
                let accepted = result
                    .get("accepted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if accepted {
                    // A successful redraw used to silently succeed; without a toast
                    // "nothing happens" is the user-perceived response when the
                    // viewport did not actually need changing.
                    state.with_mut_counted(|shell| {
                        shell.push_notification(
                            NotificationTone::Info,
                            "Redraw Terminal",
                            format!("Re-synced {label} from daemon."),
                        );
                    });
                    return;
                }
                let reason = result
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("terminal host did not accept redraw");
                // The most common failure is `terminal_host_missing` — the row's
                // xterm host isn't mounted (it is not the active viewport). Swap in
                // a clearer instruction rather than the raw debug string.
                let (title, message) = if reason == "terminal_host_missing" {
                    (
                        "Open Session Before Redrawing",
                        format!(
                            "{label} isn't the active viewport. Click the row to focus the session, then right-click → Redraw Terminal."
                        ),
                    )
                } else {
                    ("Redraw Failed", format!("{label}: {reason}"))
                };
                state.with_mut_counted(|shell| {
                    shell.push_notification(NotificationTone::Error, title, message);
                });
            });
        }
        "restart-session" => {
            let path = row.full_path.clone();
            let label = row.label.clone();
            state.with_mut_counted(|shell| shell.close_context_menu());
            // Manual restart override for ANY live session.
            // terminal_force_remote_restart_async issues the daemon TerminalRestart
            // (force_remote=true): for a remote agent it terminates the remote
            // runtime and re-resumes; for a local session the remote-terminate is a
            // no-op and the daemon re-launches the PTY.
            let (endpoint, appearance) = state.with(|shell| {
                (
                    shell.bootstrap.server_endpoint.clone(),
                    shell.effective_terminal_identity_appearance().to_string(),
                )
            });
            let trace_home = resolve_yggterm_home().unwrap_or_else(|_| PathBuf::from("."));
            state.with_mut_counted(|shell| {
                shell.push_notification(
                    NotificationTone::Warning,
                    "Restart Session",
                    format!("Restarting {label}…"),
                );
            });
            spawn(async move {
                if let Err(error) = terminal_force_remote_restart_async(
                    endpoint,
                    path.clone(),
                    Some(appearance),
                    None,
                    &trace_home,
                    "manual_restart_session",
                    0,
                )
                .await
                {
                    state.with_mut_counted(|shell| {
                        shell.push_notification(
                            NotificationTone::Error,
                            "Restart Failed",
                            format!("{label}: {error}"),
                        );
                    });
                }
            });
        }
        "regenerate-copy" => {
            spawn_selected_copy_regeneration(state, row, CopyRegenerationMode::Copy)
        }
        "regenerate-title" => {
            spawn_selected_copy_regeneration(state, row, CopyRegenerationMode::Title)
        }
        "regenerate-summary" => {
            spawn_selected_copy_regeneration(state, row, CopyRegenerationMode::Summary)
        }
        "edit-summary" => queue_copy_edit_for_row(state, row, CopyEditField::Summary),
        "delete-session" => {
            state.with_mut_counted(|shell| shell.open_context_menu_delete_for_row(&row, false));
        }
        _ => {
            state.with_mut_counted(|shell| shell.close_context_menu());
        }
    }
}
