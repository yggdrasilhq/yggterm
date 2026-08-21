// ============================================================================
// SECTION: `pub fn launch_shell` — binary entry point
// ----------------------------------------------------------------------------
// Receives the bootstrap from apps/yggterm/src/main.rs (settings loaded,
// daemon endpoint resolved, browser tree built) and starts the Dioxus
// desktop event loop. Anything after this fn through ~line 54604 is
// either part of launch_shell's setup chain or top-level helpers for the
// render path (Dioxus components, palette/theme rendering, side-rail
// state, sidebar projection, start-page rendering).
// ============================================================================
pub fn launch_shell(mut bootstrap: ShellBootstrap) -> Result<()> {
    let trace_home = perf_home_dir(&bootstrap.settings_path);
    let linux_window_transparent = bootstrap.linux_window_transparent;
    let linux_window_profile_reason = bootstrap.linux_window_profile_reason.clone();
    let linux_native_decorations = linux_force_native_decorations();
    #[cfg(target_os = "linux")]
    {
        // XTERM-BUG: ibus-cumulative-input. On desktops with an active input
        // method (ibus/fcitx — the Debian/GNOME default) and no explicit
        // GTK_IM_MODULE, WebKitGTK routes keystrokes through the IME's
        // Wayland/XIM text-input path. Those commits bypass xterm.js's `keydown`
        // handler, so xterm never clears its hidden input textarea and re-emits
        // the WHOLE accumulated buffer via onData on every keystroke (type "s"
        // -> "s", type "t" -> "st" -> the CLI renders "sst"). Forcing the simple
        // IM module (compose/dead keys still work) makes keys arrive as ordinary
        // key events. This mirrors the launcher-script guard so spawn paths that
        // bypass the launcher are also covered — notably the update-restart
        // relaunch (`Command::new(next_exe)`) and daemon app-launch. Must run
        // before the GTK event loop is built below. Opt back into the native IME
        // (full engines, e.g. CJK) with YGGTERM_ENABLE_NATIVE_IME=1.
        let enable_native_ime = std::env::var(yggterm_core::ENV_YGGTERM_ENABLE_NATIVE_IME)
            .map(|value| value == "1")
            .unwrap_or(false);
        let im_already_set = std::env::var_os("GTK_IM_MODULE").is_some();
        if !enable_native_ime && !im_already_set {
            // SAFETY: startup, before the GTK event loop or any environment-reading
            // threads are created.
            unsafe {
                std::env::set_var("GTK_IM_MODULE", "gtk-im-context-simple");
            }
        }
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "linux_im_module_policy",
            json!({
                "pid": std::process::id(),
                "enable_native_ime": enable_native_ime,
                "im_already_set": im_already_set,
                "gtk_im_module": std::env::var("GTK_IM_MODULE").unwrap_or_default(),
            }),
        );
    }
    #[cfg(target_os = "linux")]
    let linux_desktop_app_id_value = Some(linux_desktop_app_id());
    #[cfg(not(target_os = "linux"))]
    let linux_desktop_app_id_value: Option<String> = None;
    #[cfg(target_os = "macos")]
    {
        let process_name = NSString::from_str("Yggterm");
        NSProcessInfo::processInfo().setProcessName(&process_name);
    }
    append_trace_event(
        &trace_home,
        "ui",
        "startup",
        "launch_shell_enter",
        json!({
            "pid": std::process::id(),
            "transparent": linux_window_transparent,
            "profile_reason": linux_window_profile_reason,
            "native_decorations": linux_native_decorations,
        }),
    );
    if CLIENT_INSTANCE.get().is_none() {
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "launch_shell_register_begin",
            json!({
                "pid": std::process::id(),
            }),
        );
        match register_client_instance(
            &bootstrap.settings_path,
            &bootstrap.server_endpoint,
            linux_desktop_app_id_value.as_deref(),
        ) {
            Ok(registration) => {
                let _ = CLIENT_INSTANCE.set(registration);
                match terminate_superseded_client_instances(
                    &bootstrap.settings_path,
                    &bootstrap.server_endpoint,
                ) {
                    Ok(handoff) if !handoff.terminated_pids.is_empty() => {
                        apply_superseded_client_handoff_to_bootstrap(&mut bootstrap, &handoff);
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "startup",
                            "launch_shell_terminated_superseded_clients",
                            json!({
                                "pid": std::process::id(),
                                "terminated": handoff.terminated_pids.clone(),
                                "handoff_active_session_path": handoff
                                    .active_state
                                    .as_ref()
                                    .and_then(|state| state.active_session_path.as_deref()),
                            }),
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        warn!(error=%error, "failed to terminate superseded yggterm clients");
                    }
                }
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "launch_shell_register_end",
                    json!({
                        "pid": std::process::id(),
                        "ok": true,
                    }),
                );
            }
            Err(error) => {
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "launch_shell_register_end",
                    json!({
                        "pid": std::process::id(),
                        "ok": false,
                        "error": error.to_string(),
                    }),
                );
                warn!(error=%error, "failed to register yggterm client instance");
            }
        }
    }
    let shutdown_bootstrap = bootstrap.clone();
    append_trace_event(
        &trace_home,
        "ui",
        "startup",
        "launch_shell_before_bootstrap_set",
        json!({
            "pid": std::process::id(),
        }),
    );
    let initial_window_maximized = shutdown_bootstrap.settings.window_maximized;
    let _ = BOOTSTRAP.set(bootstrap);
    append_trace_event(
        &trace_home,
        "ui",
        "startup",
        "launch_shell_after_bootstrap_set",
        json!({
            "pid": std::process::id(),
        }),
    );
    #[cfg(target_os = "macos")]
    let linux_transparent_window = linux_window_transparent;
    #[cfg(target_os = "macos")]
    let window = WindowBuilder::new()
        .with_title("Yggterm")
        .with_window_icon(Some(window_icon::load_yggterm_window_icon()))
        .with_transparent(linux_transparent_window)
        .with_decorations(true)
        .with_title_hidden(true)
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_traffic_light_inset(tao::dpi::LogicalPosition::new(16.0, 14.0))
        .with_resizable(true)
        .with_maximized(initial_window_maximized)
        .with_inner_size(LogicalSize::new(1460.0, 920.0))
        .with_min_inner_size(LogicalSize::new(480.0, 360.0));
    // Phase F under-glass needs NO window-visual change (F.0.1 finding): the
    // glass webview's transparent page holes composite onto the page webview
    // below through WebKit's DMABUF renderer path, on an ordinary opaque
    // window (sandbox-verified both ways). The earlier "under-glass forces an
    // RGBA window" belief was an artifact of the SHM presentation path, which
    // cannot alpha-composite a webview over sibling widgets at all — that
    // path now demotes under-glass to legacy instead (see
    // configure_linux_webkit_compositing + the vendored arming gate).
    #[cfg(not(target_os = "macos"))]
    let linux_transparent_window = linux_window_transparent;
    #[cfg(not(target_os = "macos"))]
    let window = {
        let window = WindowBuilder::new()
            .with_title("Yggterm")
            .with_window_icon(Some(window_icon::load_yggterm_window_icon()))
            .with_transparent(linux_transparent_window)
            .with_decorations(linux_native_decorations)
            .with_resizable(true)
            .with_maximized(initial_window_maximized)
            .with_inner_size(LogicalSize::new(1460.0, 920.0))
            .with_min_inner_size(LogicalSize::new(480.0, 360.0));
        #[cfg(target_os = "linux")]
        let window = window.with_visible(false);
        #[cfg(target_os = "windows")]
        let window = window.with_taskbar_icon(Some(window_icon::load_yggterm_window_icon()));
        window
    };
    let mut config = Config::new()
        .with_window(window)
        .with_close_behaviour(WindowCloseBehaviour::WindowCloses)
        .with_exits_when_last_window_closes(true);
    #[cfg(target_os = "linux")]
    {
        let mut event_loop = EventLoopBuilder::<DesktopUserWindowEvent>::with_user_event();
        let app_id = linux_desktop_app_id_value
            .clone()
            .unwrap_or_else(linux_desktop_app_id);
        apply_linux_desktop_identity(&app_id, &trace_home);
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "linux_desktop_app_id",
            json!({
                "pid": std::process::id(),
                "app_id": app_id,
            }),
        );
        event_loop.with_app_id(&app_id);
        config = config.with_event_loop(event_loop.build());
    }
    if linux_window_transparent {
        config = config.with_background_color((0, 0, 0, 0));
    }
    append_trace_event(
        &trace_home,
        "ui",
        "startup",
        "launch_shell_config_ready",
        json!({
            "pid": std::process::id(),
            "transparent": linux_window_transparent,
            "profile_reason": linux_window_profile_reason,
            "native_decorations": linux_native_decorations,
            "initial_window_maximized": initial_window_maximized,
        }),
    );
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(app);
    if CLIENT_INSTANCE.get().is_some() {
        match finalize_client_shutdown(
            &shutdown_bootstrap.settings_path,
            &shutdown_bootstrap.server_endpoint,
        ) {
            Ok(CloseAppMode::ShutdownDaemon) => {
                info!("yggterm shutdown closed final live client and daemon");
            }
            Ok(CloseAppMode::CloseClientOnly { remaining_clients }) => {
                info!(remaining_clients, "yggterm shutdown closed client");
            }
            Err(error) => {
                warn!(error=%error, "failed to finalize yggterm shutdown");
            }
        }
    }
    if let Some(registration) = CLIENT_INSTANCE.get() {
        let _ = fs::remove_file(&registration.path);
    }
    Ok(())
}
fn render_trace_enabled() -> bool {
    *RENDER_TRACE_ENABLED.get_or_init(|| {
        std::env::var("YGGTERM_TRACE_RENDER")
            .ok()
            .as_deref()
            .is_some_and(|value| matches!(value, "1" | "true" | "TRUE" | "yes" | "YES"))
    })
}
// Per-field fingerprint of the ShellState fields most likely to churn during
// terminal streaming, plus the epoch signals app() subscribes to. Diffed
// render-over-render to name which field forced the whole-root re-render. Only
// called when render_trace_enabled() — the hashing cost is debug-only.
fn render_cause_field_hashes(
    shell: &ShellState,
    async_render_epoch: u64,
    window_epoch: u64,
) -> Vec<(&'static str, u64)> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn map_str_u64(m: &HashMap<String, u64>) -> u64 {
        let mut entries: Vec<(&String, &u64)> = m.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = DefaultHasher::new();
        for (k, v) in entries {
            k.hash(&mut s);
            v.hash(&mut s);
        }
        s.finish()
    }
    fn sidebar_samples_hash(m: &HashMap<String, LiveTerminalSidebarSample>) -> u64 {
        let mut entries: Vec<(&String, &LiveTerminalSidebarSample)> = m.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = DefaultHasher::new();
        for (k, v) in entries {
            k.hash(&mut s);
            v.cursor_line_text.hash(&mut s);
            v.text_tail.hash(&mut s);
        }
        s.finish()
    }
    fn h<T: Hash>(v: &T) -> u64 {
        let mut s = DefaultHasher::new();
        v.hash(&mut s);
        s.finish()
    }
    fn telemetry_hash(m: &HashMap<String, (String, u64)>) -> u64 {
        let mut entries: Vec<(&String, &(String, u64))> = m.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = DefaultHasher::new();
        for (k, (v, t)) in entries {
            k.hash(&mut s);
            v.hash(&mut s);
            t.hash(&mut s);
        }
        s.finish()
    }
    vec![
        ("forced_wakes", FORCED_WAKE_TOTAL.load(Ordering::SeqCst)),
        ("shellstate_mut", SHELLSTATE_MUT_TOTAL.load(Ordering::SeqCst)),
        ("epoch_async_render", async_render_epoch),
        ("epoch_window", window_epoch),
        ("busy_hint_until", map_str_u64(&shell.terminal_busy_hint_until_ms)),
        (
            "sidebar_samples",
            sidebar_samples_hash(&shell.live_terminal_sidebar_samples),
        ),
        ("input_hot_until", terminal_input_hot_until_ms()),
        (
            "snapshot_apply_count",
            shell.background_live_session_snapshot_apply_count,
        ),
        (
            "snapshot_skipped_input_hot",
            shell.background_live_session_snapshot_skipped_input_hot_count,
        ),
        (
            "snapshot_skipped_noop",
            shell.background_live_session_snapshot_skipped_noop_count,
        ),
        ("notifications_len", shell.notifications.len() as u64),
        ("last_action", h(&shell.last_action)),
        ("last_terminal_debug", h(&shell.last_terminal_debug)),
        ("last_tree_debug", h(&shell.last_tree_debug)),
        ("recent_ui_telemetry", telemetry_hash(&shell.recent_ui_telemetry)),
        (
            "cached_hot_views_len",
            shell.cached_hot_session_views.len() as u64,
        ),
        ("busy_request_id", h(&shell.busy_request_id)),
        (
            "active_surface_requests_len",
            shell.active_surface_requests.len() as u64,
        ),
        ("server_busy", shell.server_busy as u64),
        ("active_terminal_host_id", h(&shell.active_terminal_host_id)),
        ("terminal_mount_epochs", map_str_u64(&shell.terminal_mount_epochs)),
        (
            "terminal_resume_ready_len",
            shell.terminal_resume_ready_paths.len() as u64,
        ),
        (
            "latest_runtime_status",
            h(&shell.latest_runtime_status.is_some()),
        ),
        (
            "active_copy_hydration_len",
            shell.active_copy_hydration_in_flight.len() as u64,
        ),
        ("server_daemon_detail", h(&shell.server_daemon_detail)),
    ]
}
fn app() -> Element {
    let _render_span = crate::render_attribution::ComponentRenderSpan::start("app");
    // Close the causal gap: every ShellState write since the previous root
    // render is a cause of THIS one, and this is the moment that set is still
    // separable from the writes the render itself will provoke.
    crate::render_attribution::begin_root_render();
    let bootstrap = BOOTSTRAP
        .get()
        .expect("shell bootstrap not initialized")
        .clone();
    let linux_transparent_window = bootstrap.linux_window_transparent;
    let trace_home = perf_home_dir(&bootstrap.settings_path);
    // Report any elapsed attribution window from the render loop itself. It is
    // the one clock guaranteed to tick whenever there is something to report,
    // and a separate timer would have to wake on an idle app to say nothing.
    crate::render_attribution::flush_component_render_windows(&trace_home);
    // RENDER-STORM PROBE (the unpinned ~37 renders/s wake storm — implicated
    // in two CPU-spin incidents and the main-thread ensure starvation): count
    // app() executions and trace the rate once a minute, so the storm's
    // magnitude and its correlation with activity windows is measurable from
    // the event trace without a debugger.
    {
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        static RENDER_COUNT: AtomicU64 = AtomicU64::new(0);
        static LAST_REPORT_MS: AtomicU64 = AtomicU64::new(0);
        let count = RENDER_COUNT.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        let now = current_millis() as u64;
        let last = LAST_REPORT_MS.load(AtomicOrdering::Relaxed);
        if now.saturating_sub(last) >= 60_000
            && LAST_REPORT_MS
                .compare_exchange(last, now, AtomicOrdering::Relaxed, AtomicOrdering::Relaxed)
                .is_ok()
        {
            let window_ms = if last == 0 { 0 } else { now.saturating_sub(last) };
            let per_sec = if window_ms > 0 {
                count as f64 / (window_ms as f64 / 1000.0)
            } else {
                0.0
            };
            let app_render_payload = json!({
                    "renders_in_window": count,
                    "window_ms": window_ms,
                    "renders_per_sec": (per_sec * 10.0).round() / 10.0,
                });
            append_trace_event(
                &trace_home,
                "ui",
                "perf",
                "app_render_rate",
                app_render_payload.clone(),
            );
            // Mirror to ytrace for Dash notebook `ytrace query --category ui --name app_render_rate`
            yggterm_core::perf::ytrace_emit_event(
                "ui",
                "ui",
                "app_render_rate",
                app_render_payload,
            );
            // A sustained app() render rate this high is never output-driven
            // (steady agent streaming sits ~1/s, bursts ~16/s): it is the wake
            // storm implicated in the CPU-spin incidents. Surface it through
            // the same fail-pattern channel the client anomalies use so
            // scripts/render_fail_patterns.py groups it with the rest.
            if window_ms > 0 && per_sec >= 20.0 {
                let storm_payload = json!({
                        "session_path": "",
                        "anomaly": {
                            "pattern": "app_render_storm",
                            "renders_in_window": count,
                            "window_ms": window_ms,
                            "renders_per_sec": (per_sec * 10.0).round() / 10.0,
                        },
                    });
                append_trace_event(
                    &trace_home,
                    "ui",
                    "render_fail_pattern",
                    "detected",
                    storm_payload.clone(),
                );
                // Mirror storm incident to ytrace — Dash Dash `render/storm` + incident channel
                // so ytop Dash notebook can correlate fan/CPU with render rate without tailing trace.
                yggterm_core::perf::ytrace_emit_event(
                    "ui",
                    "render",
                    "storm",
                    storm_payload.clone(),
                );
                yggterm_core::perf::ytrace_provider().incident(
                    "ui",
                    "render",
                    "storm",
                    storm_payload,
                );
            }
            RENDER_COUNT.store(0, AtomicOrdering::Relaxed);
        }
        // Storm ONSET detector. The 60s report above can only describe a storm
        // that already ended; this samples every STORM_ONSET_SAMPLE_RENDERS
        // renders so a live storm arms the attribution pass while it is still
        // running. Arming is skipped when the env-gated probe is already on
        // (that path emits per render and owns the accumulators).
        if !render_trace_enabled() {
            let window_start = STORM_ONSET_WINDOW_START_MS.load(AtomicOrdering::Relaxed);
            if window_start == 0 {
                STORM_ONSET_WINDOW_START_MS.store(now, AtomicOrdering::Relaxed);
                STORM_ONSET_WINDOW_COUNT.store(0, AtomicOrdering::Relaxed);
            } else {
                let sampled = STORM_ONSET_WINDOW_COUNT.fetch_add(1, AtomicOrdering::Relaxed) + 1;
                if sampled >= STORM_ONSET_SAMPLE_RENDERS {
                    let elapsed = now.saturating_sub(window_start);
                    let storming = elapsed > 0 && elapsed <= storm_onset_max_window_ms();
                    let last_emit = STORM_AUTOPSY_LAST_EMIT_MS.load(AtomicOrdering::Relaxed);
                    let cooled = last_emit == 0
                        || now.saturating_sub(last_emit) >= STORM_AUTOPSY_COOLDOWN_MS;
                    if storming && cooled && !storm_autopsy_armed() {
                        if let Some(hits) = STORM_AUTOPSY_FIELD_HITS.get() {
                            if let Ok(mut hits) = hits.lock() {
                                hits.clear();
                            }
                        }
                        if let Some(prev) = STORM_AUTOPSY_PREV.get() {
                            if let Ok(mut prev) = prev.lock() {
                                prev.clear();
                            }
                        }
                        crate::render_attribution::clear_state_write_totals();
                        STORM_AUTOPSY_UNATTRIBUTED.store(0, AtomicOrdering::Relaxed);
                        STORM_AUTOPSY_RENDERS_LEFT
                            .store(STORM_AUTOPSY_RENDER_BUDGET, AtomicOrdering::Relaxed);
                        STORM_AUTOPSY_STARTED_MS.store(now, AtomicOrdering::Relaxed);
                        // Baseline the forced-wake counter so the emit can report
                        // how many of the window's renders were OUR explicit
                        // schedule_update() calls. THE discriminator the previous
                        // autopsies lacked: they reported 511/512 renders
                        // "unattributed" with every field histogram EMPTY, which
                        // says only "no signal changed" — not who woke the root.
                        // forced_wakes ~= renders  => a caller is over-scheduling
                        // (find it via the guarded call sites).
                        // forced_wakes ~= 0        => nothing of ours asked; the
                        //   wakes are Dioxus-internal (a future/eval/task
                        //   resolving every frame), which is a completely
                        //   different fix and cannot be found by auditing our
                        //   schedule_update callers at all.
                        STORM_AUTOPSY_FORCED_WAKE_BASE
                            .store(FORCED_WAKE_TOTAL.load(Ordering::SeqCst), AtomicOrdering::Relaxed);
                        STORM_AUTOPSY_ARMED.store(true, AtomicOrdering::Relaxed);
                    }
                    STORM_ONSET_WINDOW_START_MS.store(now, AtomicOrdering::Relaxed);
                    STORM_ONSET_WINDOW_COUNT.store(0, AtomicOrdering::Relaxed);
                }
            }
        }
    }
    let app_instance_id = use_hook(|| APP_INSTANCE_ID_SEQ.fetch_add(1, Ordering::SeqCst));
    let is_primary_instance = use_hook({
        let trace_home = trace_home.clone();
        move || {
            let is_primary = PRIMARY_APP_INSTANCE_ID
                .compare_exchange(0, app_instance_id, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
                || PRIMARY_APP_INSTANCE_ID.load(Ordering::SeqCst) == app_instance_id;
            if !is_primary {
                let primary_id = PRIMARY_APP_INSTANCE_ID.load(Ordering::SeqCst);
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "duplicate_app_instance_suppressed",
                    json!({
                        "pid": std::process::id(),
                        "app_instance_id": app_instance_id,
                        "primary_app_instance_id": primary_id,
                    }),
                );
            }
            is_primary
        }
    });
    let render_count = APP_ROOT_RENDER_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    if render_trace_enabled() {
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "app_root_render_count",
            json!({
                "pid": std::process::id(),
                "count": render_count,
            }),
        );
    }
    if !APP_ROOT_RENDER_TRACED.swap(true, Ordering::SeqCst) {
        append_trace_event(
            &trace_home,
            "ui",
            "startup",
            "app_root_render",
            json!({
                "pid": std::process::id(),
            }),
        );
    }
    if !is_primary_instance {
        return rsx! {
            div {
                id: "yggterm-shell-shadow-root",
                style: "display:none;",
            }
        };
    }
    let mut state =
        use_hook(|| Signal::new_in_scope(ShellState::new(bootstrap.clone()), ScopeId::ROOT));
    let desktop = use_window();
    let mut hovered = use_signal(|| None::<HoveredControl>);
    // One reveal state machine per auto-hidden edge: top (titlebar), left
    // (session tree), right (metadata rail). Same struct, same handlers — see
    // `AutoHideSignals`.
    let titlebar_autohide = use_autohide_signals();
    let left_sidebar_autohide = use_autohide_signals();
    let right_rail_autohide = use_autohide_signals();
    let mut titlebar_autohide_hovered = titlebar_autohide.hovered;
    let titlebar_autohide_lingering = titlebar_autohide.lingering;
    let titlebar_autohide_linger_generation = titlebar_autohide.linger_generation;
    let mut last_active_terminal_input_policy =
        use_signal(|| None::<ActiveTerminalInputPolicySignature>);
    // Last value pushed to the vendored chord claimer (`None` == never pushed),
    // so an unchanged answer costs nothing every render.
    let mut last_web_page_chords_armed = use_signal(|| None::<bool>);
    let mut startup_sync_started = use_signal(|| false);
    let mut browser_tree_load_started = use_signal(|| false);
    let browser_tree_refresh_loop_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let mut background_refresh_defer_started = use_signal(|| false);
    let mut update_check_started = use_signal(|| false);
    let mut dock_pulse_started = use_signal(|| false);
    let app_control_loop_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let app_control_watchdog_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let app_control_drain_in_flight = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let app_control_drain_started_ms = use_hook(|| Arc::new(AtomicU64::new(0))).clone();
    let startup_sync_render_launch = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let window_spawn_probe_started = use_hook(current_millis);
    let window_spawn_probe_launched = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let window_spawn_traced = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let transparent_window_reconfigure_started =
        use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    let mut window_epoch = use_signal(|| 0_u64);
    let async_render_epoch = use_signal(|| 0_u64);
    if render_trace_enabled() {
        let async_epoch_val = *async_render_epoch.peek();
        let window_epoch_val = *window_epoch.peek();
        if let Some(current) = safe_shell_read(state, "render_cause_fingerprint", |shell| {
            render_cause_field_hashes(shell, async_epoch_val, window_epoch_val)
        }) {
            let prev_lock = RENDER_CAUSE_PREV.get_or_init(|| Mutex::new(Vec::new()));
            if let Ok(mut prev) = prev_lock.lock() {
                let first = prev.is_empty();
                let changed: Vec<&'static str> = current
                    .iter()
                    .filter(|(name, hash)| {
                        prev.iter()
                            .find(|(pname, _)| pname == name)
                            .map(|(_, phash)| phash != hash)
                            .unwrap_or(true)
                    })
                    .map(|(name, _)| *name)
                    .collect();
                *prev = current;
                drop(prev);
                append_trace_event(
                    &trace_home,
                    "ui",
                    "startup",
                    "app_root_render_cause",
                    json!({
                        "pid": std::process::id(),
                        "count": render_count,
                        "changed": changed,
                        "unattributed": !first && changed.is_empty(),
                        "first": first,
                    }),
                );
                {
                    {
                        let snapshot: serde_json::Map<String, Value> =
                            crate::render_attribution::state_write_totals(None)
                                .into_iter()
                                .map(|(site, count)| (site, json!(count)))
                                .collect();
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "startup",
                            "shell_mut_hist",
                            json!({
                                "pid": std::process::id(),
                                "count": render_count,
                                "hist": Value::Object(snapshot),
                            }),
                        );
                    }
                }
            }
        }
    } else if storm_autopsy_armed() {
        // Armed by the onset detector. Accumulate which ShellState fields
        // changed render-over-render, then emit a single aggregated autopsy
        // when the budget or the time window closes.
        let async_epoch_val = *async_render_epoch.peek();
        let window_epoch_val = *window_epoch.peek();
        if let Some(current) = safe_shell_read(state, "storm_autopsy_fingerprint", |shell| {
            render_cause_field_hashes(shell, async_epoch_val, window_epoch_val)
        }) {
            let prev_lock = STORM_AUTOPSY_PREV.get_or_init(|| Mutex::new(Vec::new()));
            if let Ok(mut prev) = prev_lock.lock() {
                let first = prev.is_empty();
                let changed: Vec<&'static str> = current
                    .iter()
                    .filter(|(name, hash)| {
                        prev.iter()
                            .find(|(pname, _)| pname == name)
                            .map(|(_, phash)| phash != hash)
                            .unwrap_or(true)
                    })
                    .map(|(name, _)| *name)
                    .collect();
                *prev = current;
                drop(prev);
                if !first {
                    if changed.is_empty() {
                        STORM_AUTOPSY_UNATTRIBUTED.fetch_add(1, Ordering::Relaxed);
                    } else if let Ok(mut hits) = STORM_AUTOPSY_FIELD_HITS
                        .get_or_init(|| Mutex::new(HashMap::new()))
                        .lock()
                    {
                        for name in changed {
                            *hits.entry(name).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        let started = STORM_AUTOPSY_STARTED_MS.load(Ordering::Relaxed);
        let now_ms = current_millis();
        let left = STORM_AUTOPSY_RENDERS_LEFT
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        let expired = now_ms.saturating_sub(started) >= STORM_AUTOPSY_MAX_WINDOW_MS;
        if left == 0 || expired {
            STORM_AUTOPSY_ARMED.store(false, Ordering::Relaxed);
            STORM_AUTOPSY_LAST_EMIT_MS.store(now_ms, Ordering::Relaxed);
            let observed = STORM_AUTOPSY_RENDER_BUDGET.saturating_sub(left);
            let duration_ms = now_ms.saturating_sub(started);
            let per_sec = if duration_ms > 0 {
                (observed as f64 / (duration_ms as f64 / 1000.0) * 10.0).round() / 10.0
            } else {
                0.0
            };
            let field_hits: serde_json::Map<String, Value> = STORM_AUTOPSY_FIELD_HITS
                .get()
                .and_then(|hits| hits.lock().ok())
                .map(|hits| {
                    let mut entries: Vec<(&&'static str, &u64)> = hits.iter().collect();
                    entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                    entries
                        .into_iter()
                        .map(|(k, v)| ((*k).to_string(), json!(*v)))
                        .collect()
                })
                .unwrap_or_default();
            let mut_hist: serde_json::Map<String, Value> =
                crate::render_attribution::state_write_totals(Some(24))
                    .into_iter()
                    .map(|(site, count)| (site, json!(count)))
                    .collect();
            append_trace_event(
                &trace_home,
                "ui",
                "render_fail_pattern",
                "detected",
                json!({
                    "session_path": "",
                    "anomaly": {
                        "pattern": "app_render_storm_autopsy",
                        // Arm rate is stamped so a reader can tell a real storm
                        // from one forced by lowering the threshold to verify
                        // the probe path. Anything but the default is a drill.
                        "arm_rate_per_sec": storm_arm_rate_per_sec(),
                        "arm_rate_is_default": storm_arm_rate_per_sec() == STORM_ARM_RATE_DEFAULT,
                        "renders_observed": observed,
                        "window_ms": duration_ms,
                        "renders_per_sec": per_sec,
                        "unattributed": STORM_AUTOPSY_UNATTRIBUTED.load(Ordering::Relaxed),
                        // Renders this window that OUR schedule_update() asked
                        // for. Read it against `renders_observed`: ~equal means a
                        // caller over-schedules; ~0 with a high unattributed count
                        // means the wakes come from inside Dioxus (a future/eval
                        // resolving), not from us. Counted unconditionally, so
                        // this is always populated when a storm fires.
                        "forced_wakes": FORCED_WAKE_TOTAL
                            .load(Ordering::SeqCst)
                            .saturating_sub(STORM_AUTOPSY_FORCED_WAKE_BASE.load(Ordering::Relaxed)),
                        "changed_fields": Value::Object(field_hits),
                        "shellstate_mut": Value::Object(mut_hist),
                        "truncated_by_time": expired && left > 0,
                    },
                }),
            );
        }
    }
    let mut last_startup_terminal_restore_path = use_signal(|| None::<String>);
    let mut last_startup_terminal_recovery_path = use_signal(|| None::<String>);
    let mut last_terminal_mount_key = use_signal(|| None::<String>);
    let mut last_linux_window_chrome_apply = use_signal(|| None::<LinuxWindowChromeApplySignature>);
    let mut last_preview_refresh_marker = use_signal(|| None::<(String, u64, bool, bool)>);
    let mut last_sidebar_autoscroll_path = use_signal(|| None::<String>);
    let mut last_sidebar_bounds_repair_key = use_signal(|| None::<String>);
    let mut last_search_value_epoch_sync = use_signal(|| None::<u64>);
    let mut last_tree_rename_focus_path = use_signal(|| None::<String>);
    let schedule_ui_update = schedule_update();
    // Count forced wakes ALWAYS, not only under the render trace. A storm is
    // sporadic and unannounced, so a counter you must predict and enable ahead of
    // time is a counter that is off exactly when the storm you needed it for
    // fires (the previous autopsies all landed with no wake data at all). The
    // cost is one relaxed atomic add per explicit schedule_update — nothing next
    // to the render it triggers. The EXPENSIVE part (per-field hashing in
    // render_cause_field_hashes) stays gated behind render_trace_enabled().
    let schedule_ui_update: std::sync::Arc<dyn Fn() + Send + Sync> = {
        let inner = schedule_ui_update;
        std::sync::Arc::new(move || {
            FORCED_WAKE_TOTAL.fetch_add(1, Ordering::SeqCst);
            inner();
        })
    };
    let web_surface_reconcile_loop_started =
        use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !web_surface_reconcile_loop_started.swap(true, Ordering::SeqCst) {
        let desktop = desktop.clone();
        let trace_home = trace_home.clone();
        spawn_forever(async move {
            web_surface_native_reconcile_loop(state, desktop, trace_home).await;
        });
    }
    // F.1 under-glass input plumbing: the synchronous cover push and the
    // page-edge reveal forward. Both are inert in legacy stacking.
    let web_surface_cover_push_loop_started =
        use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !web_surface_cover_push_loop_started.swap(true, Ordering::SeqCst) {
        let desktop = desktop.clone();
        spawn_forever(async move {
            web_surface_cover_push_loop(desktop).await;
        });
    }
    let web_surface_edge_motion_loop_started =
        use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !web_surface_edge_motion_loop_started.swap(true, Ordering::SeqCst) {
        spawn_forever(web_surface_edge_motion_reveal_loop(
            state,
            titlebar_autohide,
            left_sidebar_autohide,
            right_rail_autohide,
        ));
    }
    // The one blink clock (see `STATUS_DOT_BLINK_CSS`). It lives here rather
    // than beside the stylesheet because the rule is emitted by three surfaces
    // and the clock must exist exactly once per webview; the installer is
    // idempotent anyway, so a second mount is harmless.
    let status_dot_blink_clock_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !status_dot_blink_clock_started.swap(true, Ordering::SeqCst) {
        spawn_forever(async move {
            let _ = document::eval(STATUS_DOT_BLINK_JS).await;
        });
    }
    // UI-THREAD BLOCK WATCHDOG.
    //
    // The stamp runs on the Dioxus executor, i.e. the UI thread, on a TIMER —
    // not on render. A render-driven heartbeat cannot work: the calm render
    // rate is around one per second, so an idle app would look permanently
    // blocked and a real block would be indistinguishable from quiet.
    //
    // The watcher is a plain OS thread started separately. That separation is
    // the whole point: every existing instrument for a stalled GUI runs on the
    // thread that stalls, which is why a freeze the user had to kill by hand
    // produced zero incidents. Only something outside the UI thread can still
    // speak while the UI thread cannot.
    let ui_heartbeat_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !ui_heartbeat_started.swap(true, Ordering::SeqCst) {
        yggterm_core::ui_block::spawn_watchdog(trace_home.clone());
        spawn_forever(async move {
            loop {
                yggterm_core::ui_block::stamp();
                sleep(Duration::from_millis(
                    yggterm_core::ui_block::STAMP_INTERVAL_MS,
                ))
                .await;
            }
        });
    }
    let keytip_alt_tap_loop_started = use_hook(|| Arc::new(AtomicBool::new(false))).clone();
    if !keytip_alt_tap_loop_started.swap(true, Ordering::SeqCst) {
        spawn_forever(async move {
            keytip_alt_tap_install_loop(state).await;
        });
    }
    if !browser_tree_refresh_loop_started.swap(true, Ordering::SeqCst) {
        spawn_forever(async move {
            loop {
                let (should_refresh, selected_hint, wait_ms) = {
                    let shell = state.peek();
                    let now_ms = current_millis();
                    let should_refresh = browser_tree_refresh_should_start(&shell, now_ms);
                    let selected_hint = should_refresh
                        .then(|| shell.browser.selected_path().map(ToOwned::to_owned))
                        .flatten();
                    let wait_ms = if should_refresh {
                        BACKGROUND_REFRESH_WAIT_MIN_MS
                    } else {
                        browser_tree_refresh_scheduler_wait_ms(&shell, now_ms)
                    };
                    (should_refresh, selected_hint, wait_ms)
                };
                if should_refresh {
                    spawn_browser_tree_refresh(
                        state,
                        "periodic_browser_tree_refresh",
                        selected_hint,
                    );
                }
                // Remote-machine truth refreshes have no other durable driver:
                // their only re-arm points are initial server sync and the tail
                // of a completed refresh, so one deferred tick (active terminal)
                // would otherwise kill the chain for the GUI's lifetime and
                // freeze the cwd tree + start page on stale scan data.
                maybe_spawn_missing_remote_machine_refreshes(state);
                maybe_spawn_missing_managed_cli_refreshes(state);
                sleep(Duration::from_millis(wait_ms)).await;
            }
        });
    }
    let desktop_for_root_effect = desktop.clone();
    let trace_home_for_root_effect = trace_home.clone();
    let linux_transparent_window_for_root_effect = linux_transparent_window;
    let restore_window_maximized_for_root_effect = state.read().settings.window_maximized;
    let transparent_window_reconfigure_started_for_root_effect =
        transparent_window_reconfigure_started.clone();
    let trace_home_for_mount_epoch = trace_home.clone();
    use_effect(move || {
        if !APP_ROOT_EFFECT_TRACED.swap(true, Ordering::SeqCst) {
            append_trace_event(
                &trace_home_for_root_effect,
                "ui",
                "startup",
                "app_root_effect",
                json!({
                    "pid": std::process::id(),
                }),
            );
        }
        if restore_window_maximized_for_root_effect
            && !APP_ROOT_MAXIMIZED_RESTORED.swap(true, Ordering::SeqCst)
        {
            window().set_maximized(true);
        }
        #[cfg(target_os = "macos")]
        {
            if !APP_ROOT_MAC_WINDOW_FORCED.swap(true, Ordering::SeqCst) {
                desktop_for_root_effect.set_visible(true);
                desktop_for_root_effect.set_minimized(false);
                desktop_for_root_effect.set_focus();
                append_trace_event(
                    &trace_home_for_root_effect,
                    "ui",
                    "startup",
                    "mac_window_forced_visible",
                    json!({
                        "pid": std::process::id(),
                    }),
                );
            }
        }
        #[cfg(target_os = "linux")]
        {
            if !APP_ROOT_LINUX_WINDOW_SHOWN.swap(true, Ordering::SeqCst) {
                let maximized = desktop_for_root_effect.is_maximized()
                    || restore_window_maximized_for_root_effect;
                let radius = if maximized {
                    0
                } else {
                    UNMAXIMIZED_SHELL_RADIUS_PX
                };
                let effective_radius = shell_effective_radius(
                    radius,
                    maximized,
                    linux_transparent_window_for_root_effect,
                );
                let reveal_after_shape = linux_startup_reveal_should_wait_for_shape(
                    linux_transparent_window_for_root_effect,
                    effective_radius,
                    maximized,
                );
                if reveal_after_shape {
                    prepare_linux_window_reveal_after_corner_shape(&desktop_for_root_effect);
                }
                let reveal_delay_ms =
                    if reveal_after_shape && linux_transparent_window_for_root_effect {
                        transparent_window_reconfigure_started_for_root_effect
                            .store(true, Ordering::SeqCst);
                        schedule_linux_transparent_window_pre_reveal_reconfigure(
                            &desktop_for_root_effect,
                            effective_radius,
                            maximized,
                            trace_home_for_root_effect.clone(),
                        )
                    } else {
                        0
                    };
                apply_linux_transparent_window_surface_style(
                    &desktop_for_root_effect,
                    linux_transparent_window_for_root_effect,
                    effective_radius,
                );
                apply_linux_window_corner_shape(
                    &desktop_for_root_effect,
                    effective_radius,
                    maximized,
                );
                apply_linux_compositor_blur(
                    &desktop_for_root_effect,
                    linux_transparent_window_for_root_effect,
                    &trace_home_for_root_effect,
                );
                apply_linux_window_shape_reapply_sequence(
                    &desktop_for_root_effect,
                    effective_radius,
                    maximized,
                );
                desktop_for_root_effect.request_redraw();
                let _ = desktop_for_root_effect.webview.set_visible(true);
                desktop_for_root_effect.set_visible(true);
                if reveal_after_shape {
                    reveal_linux_window_after_corner_shape(
                        &desktop_for_root_effect,
                        effective_radius,
                        maximized,
                        trace_home_for_root_effect.clone(),
                        reveal_delay_ms,
                    );
                }
                append_trace_event(
                    &trace_home_for_root_effect,
                    "ui",
                    "startup",
                    "linux_window_shown_after_shape_prepare",
                    json!({
                        "pid": std::process::id(),
                        "radius": effective_radius,
                        "transparent": linux_transparent_window_for_root_effect,
                    }),
                );
            }
        }
        if !APP_ROOT_WINDOW_FOCUS_REQUESTED.swap(true, Ordering::SeqCst) {
            desktop_for_root_effect.set_focus();
            append_trace_event(
                &trace_home_for_root_effect,
                "ui",
                "startup",
                "window_focus_requested",
                json!({
                    "pid": std::process::id(),
                }),
            );
        }
        if XTERM_ASSETS_BOOTSTRAPPED.get().is_none() {
            let _ = XTERM_ASSETS_BOOTSTRAPPED.set(());
            let _ = document::eval(&xterm_assets_bootstrap_script());
        }
    });
    {
        let desktop = desktop.clone();
        let trace_home = trace_home.clone();
        let state = state;
        let schedule_ui_update = schedule_ui_update.clone();
        let window_spawn_probe_launched = window_spawn_probe_launched.clone();
        let window_spawn_traced = window_spawn_traced.clone();
        let transparent_window_reconfigure_started = transparent_window_reconfigure_started.clone();
        let linux_transparent_window = linux_transparent_window;
        if !window_spawn_traced.load(Ordering::SeqCst)
            && !window_spawn_probe_launched.swap(true, Ordering::SeqCst)
        {
            let desktop = desktop.clone();
            let trace_home = trace_home.clone();
            let state = state;
            let schedule_ui_update = schedule_ui_update.clone();
            let started_at_ms = window_spawn_probe_started;
            let window_spawn_traced = window_spawn_traced.clone();
            let transparent_window_reconfigure_started =
                transparent_window_reconfigure_started.clone();
            spawn(async move {
                for _ in 0..40 {
                    let window = describe_window(&desktop);
                    let visible = window
                        .get("visible")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let width = window
                        .get("inner_size")
                        .and_then(|value| value.get("width"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let height = window
                        .get("inner_size")
                        .and_then(|value| value.get("height"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if width > 0 && height > 0 {
                        #[cfg(target_os = "linux")]
                        if linux_transparent_window
                            && !transparent_window_reconfigure_started.swap(true, Ordering::SeqCst)
                        {
                            let desktop_for_reconfigure = desktop.clone();
                            let trace_home_for_reconfigure = trace_home.clone();
                            spawn(async move {
                                append_trace_event(
                                    &trace_home_for_reconfigure,
                                    "ui",
                                    "startup",
                                    "transparent_window_reconfigure_begin",
                                    json!({
                                        "pid": std::process::id(),
                                        "width": width,
                                        "height": height,
                                    }),
                                );
                                desktop_for_reconfigure.request_redraw();
                                sleep(Duration::from_millis(120)).await;
                                let size = desktop_for_reconfigure.inner_size();
                                if size.width > 2 && size.height > 2 {
                                    desktop_for_reconfigure.set_inner_size(LogicalSize::new(
                                        f64::from(size.width + 1),
                                        f64::from(size.height),
                                    ));
                                    desktop_for_reconfigure.request_redraw();
                                    sleep(Duration::from_millis(40)).await;
                                    desktop_for_reconfigure.set_inner_size(LogicalSize::new(
                                        f64::from(size.width),
                                        f64::from(size.height),
                                    ));
                                    desktop_for_reconfigure.request_redraw();
                                }
                                append_trace_event(
                                    &trace_home_for_reconfigure,
                                    "ui",
                                    "startup",
                                    "transparent_window_reconfigure_end",
                                    json!({
                                        "pid": std::process::id(),
                                        "width": size.width,
                                        "height": size.height,
                                    }),
                                );
                            });
                        }
                        let focus_snapshot = focus_app_window(&desktop)
                            .unwrap_or_else(|error| json!({ "error": error.to_string() }));
                        sync_active_terminal_input_policy(state);
                        schedule_ui_update();
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "startup",
                            "window_spawned",
                            json!({
                                "pid": std::process::id(),
                                "elapsed_ms": current_millis().saturating_sub(started_at_ms),
                                "window_visible": visible,
                                "window": window,
                            }),
                        );
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "startup",
                            "window_spawn_focus_requested",
                            json!({
                                "pid": std::process::id(),
                                "window": focus_snapshot,
                            }),
                        );
                        window_spawn_traced.store(true, Ordering::SeqCst);
                        if kick_active_remote_preview_sync(
                            state,
                            "preview_refresh_after_window_spawn",
                        ) {
                            schedule_ui_update();
                        }
                        return;
                    }
                    sleep(Duration::from_millis(50)).await;
                }
            });
        }
    }
    {
        let settings_path = bootstrap.settings_path.clone();
        let wake_app_control = desktop.poll_waker();
        let schedule_ui_update = schedule_ui_update.clone();
        if !app_control_loop_started.swap(true, Ordering::SeqCst) {
            let settings_path = settings_path.clone();
            let trace_home = perf_home_dir(&settings_path);
            let wake_app_control = wake_app_control.clone();
            let schedule_ui_update = schedule_ui_update.clone();
            thread::spawn(move || {
                append_trace_event(
                    &trace_home,
                    "ui",
                    "app_control",
                    "loop_spawned",
                    json!({
                        "pid": std::process::id(),
                    }),
                );
                loop {
                    let pending =
                        app_control_requests_pending_for_worker(&trace_home, std::process::id());
                    if pending {
                        // Always WAKE so the request is processed (the Poll-event handler
                        // drains it independently of any render). Only FORCE a shell
                        // re-render for MUTATING commands — read-only probes (screenshot /
                        // state / buffer reads) must not re-render the whole giant shell
                        // tree (the ~4-renders-per-probe churn from the DOM-leak
                        // investigation). Mutating commands self-render via their state
                        // writes anyway. See AppControlCommand::is_read_only.
                        wake_app_control();
                        if app_control_pending_render_needed_for_worker(
                            &trace_home,
                            std::process::id(),
                        ) {
                            schedule_ui_update();
                        }
                    }
                    thread::sleep(Duration::from_millis(if pending {
                        APP_CONTROL_ACTIVE_POLL_MS
                    } else {
                        APP_CONTROL_IDLE_POLL_MS
                    }));
                }
            });
        }
    }
    {
        let settings_path = bootstrap.settings_path.clone();
        let desktop = desktop.clone();
        let state = state;
        let schedule_ui_update = schedule_ui_update.clone();
        let app_control_watchdog_started = app_control_watchdog_started.clone();
        let app_control_drain_in_flight = app_control_drain_in_flight.clone();
        let app_control_drain_started_ms = app_control_drain_started_ms.clone();
        let window_spawn_traced = window_spawn_traced.clone();
        if !app_control_watchdog_started.swap(true, Ordering::SeqCst) {
            let settings_path = settings_path.clone();
            let desktop = desktop.clone();
            let state = state;
            let schedule_ui_update = schedule_ui_update.clone();
            let app_control_drain_in_flight = app_control_drain_in_flight.clone();
            let app_control_drain_started_ms = app_control_drain_started_ms.clone();
            let window_spawn_traced = window_spawn_traced.clone();
            spawn_forever(async move {
                let trace_home = perf_home_dir(&settings_path);
                append_trace_event(
                    &trace_home,
                    "ui",
                    "app_control",
                    "watchdog_spawned",
                    json!({
                        "pid": std::process::id(),
                    }),
                );
                loop {
                    // The roster is only true if the primary keeps re-asserting
                    // itself: a record removed after startup would otherwise hide
                    // the user's GUI from every untargeted verb for the rest of
                    // its life (agent-control-plane finding #1).
                    reassert_client_instance_registration(&settings_path);
                    let pending_requests =
                        app_control_requests_pending_for_worker(&trace_home, std::process::id());
                    if app_control_drain_in_flight.load(Ordering::SeqCst) && pending_requests {
                        let started_ms = app_control_drain_started_ms.load(Ordering::SeqCst);
                        if started_ms > 0
                            && current_millis().saturating_sub(started_ms)
                                > APP_CONTROL_DRAIN_STUCK_MS
                        {
                            app_control_drain_in_flight.store(false, Ordering::SeqCst);
                            app_control_drain_started_ms.store(0, Ordering::SeqCst);
                            append_trace_event(
                                &trace_home,
                                "ui",
                                "app_control",
                                "watchdog_drain_stuck_reset",
                                json!({
                                    "pid": std::process::id(),
                                    "started_ms": started_ms,
                                    "age_ms": current_millis().saturating_sub(started_ms),
                                }),
                            );
                        }
                    }
                    if !window_spawn_traced.load(Ordering::SeqCst) {
                        sleep(Duration::from_millis(APP_CONTROL_WATCHDOG_IDLE_POLL_MS)).await;
                        continue;
                    }
                    if !app_control_drain_in_flight.load(Ordering::SeqCst) && pending_requests {
                        app_control_drain_in_flight.store(true, Ordering::SeqCst);
                        app_control_drain_started_ms.store(current_millis(), Ordering::SeqCst);
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "app_control",
                            "watchdog_drain_begin",
                            json!({
                                "pid": std::process::id(),
                            }),
                        );
                        loop {
                            match process_pending_app_control_requests(
                                &settings_path,
                                desktop.clone(),
                                state,
                                titlebar_autohide_hovered,
                                titlebar_autohide_lingering,
                                titlebar_autohide_linger_generation,
                            )
                            .await
                            {
                                Ok(true) => continue,
                                Ok(false) => break,
                                Err(error) => {
                                    append_trace_event(
                                        &trace_home,
                                        "ui",
                                        "app_control",
                                        "watchdog_loop_error",
                                        json!({
                                            "pid": std::process::id(),
                                            "error": error.to_string(),
                                        }),
                                    );
                                    warn!(error=%error, "failed to process app control request");
                                    break;
                                }
                            }
                        }
                        app_control_drain_in_flight.store(false, Ordering::SeqCst);
                        app_control_drain_started_ms.store(0, Ordering::SeqCst);
                        append_trace_event(
                            &trace_home,
                            "ui",
                            "app_control",
                            "watchdog_drain_end",
                            json!({
                                "pid": std::process::id(),
                            }),
                        );
                        if app_control_pending_render_needed_for_worker(
                            &trace_home,
                            std::process::id(),
                        ) {
                            schedule_ui_update();
                        }
                    }
                    sleep(Duration::from_millis(if pending_requests {
                        APP_CONTROL_ACTIVE_POLL_MS
                    } else {
                        APP_CONTROL_WATCHDOG_IDLE_POLL_MS
                    }))
                    .await;
                }
            });
        }
    }
    {
        let settings_path = bootstrap.settings_path.clone();
        let desktop = desktop.clone();
        let state = state;
        let schedule_ui_update = schedule_ui_update.clone();
        let window_spawn_traced = window_spawn_traced.clone();
        let app_control_drain_in_flight = app_control_drain_in_flight.clone();
        let app_control_drain_started_ms = app_control_drain_started_ms.clone();
        use_effect(move || {
            let trace_home = perf_home_dir(&settings_path);
            if !window_spawn_traced.load(Ordering::SeqCst) {
                return;
            }
            if app_control_drain_in_flight.load(Ordering::SeqCst)
                || !app_control_requests_pending_for_worker(&trace_home, std::process::id())
            {
                return;
            }
            app_control_drain_in_flight.store(true, Ordering::SeqCst);
            app_control_drain_started_ms.store(current_millis(), Ordering::SeqCst);
            let settings_path = settings_path.clone();
            let desktop = desktop.clone();
            let state = state;
            let schedule_ui_update = schedule_ui_update.clone();
            let app_control_drain_in_flight = app_control_drain_in_flight.clone();
            let app_control_drain_started_ms = app_control_drain_started_ms.clone();
            spawn(async move {
                let trace_home = perf_home_dir(&settings_path);
                loop {
                    match process_pending_app_control_requests(
                        &settings_path,
                        desktop.clone(),
                        state,
                        titlebar_autohide_hovered,
                        titlebar_autohide_lingering,
                        titlebar_autohide_linger_generation,
                    )
                    .await
                    {
                        Ok(true) => continue,
                        Ok(false) => break,
                        Err(error) => {
                            append_trace_event(
                                &trace_home,
                                "ui",
                                "app_control",
                                "loop_error",
                                json!({
                                    "pid": std::process::id(),
                                    "error": error.to_string(),
                                }),
                            );
                            warn!(error=%error, "failed to process app control request");
                            break;
                        }
                    }
                }
                app_control_drain_in_flight.store(false, Ordering::SeqCst);
                app_control_drain_started_ms.store(0, Ordering::SeqCst);
                if app_control_pending_render_needed_for_worker(&trace_home, std::process::id()) {
                    schedule_ui_update();
                }
            });
        });
    }
    let settings_path_for_app_control_handler = bootstrap.settings_path.clone();
    let desktop_for_app_control_handler = desktop.clone();
    let window_spawn_traced_for_app_control = window_spawn_traced.clone();
    let app_control_drain_in_flight_for_handler = app_control_drain_in_flight.clone();
    let app_control_drain_started_ms_for_handler = app_control_drain_started_ms.clone();
    use_wry_event_handler(move |event, _| {
        if matches!(event, TaoEvent::UserEvent(DesktopUserWindowEvent::Poll(_))) {
            let trace_home = perf_home_dir(&settings_path_for_app_control_handler);
            if !window_spawn_traced_for_app_control.load(Ordering::SeqCst) {
                return;
            }
            if !app_control_drain_in_flight_for_handler.load(Ordering::SeqCst)
                && app_control_requests_pending_for_worker(&trace_home, std::process::id())
            {
                app_control_drain_in_flight_for_handler.store(true, Ordering::SeqCst);
                app_control_drain_started_ms_for_handler.store(current_millis(), Ordering::SeqCst);
                let settings_path = settings_path_for_app_control_handler.clone();
                let desktop = desktop_for_app_control_handler.clone();
                let state = state;
                let app_control_drain_in_flight = app_control_drain_in_flight_for_handler.clone();
                let app_control_drain_started_ms = app_control_drain_started_ms_for_handler.clone();
                spawn(async move {
                    let trace_home = perf_home_dir(&settings_path);
                    loop {
                        match process_pending_app_control_requests(
                            &settings_path,
                            desktop.clone(),
                            state,
                            titlebar_autohide_hovered,
                            titlebar_autohide_lingering,
                            titlebar_autohide_linger_generation,
                        )
                        .await
                        {
                            Ok(true) => continue,
                            Ok(false) => break,
                            Err(error) => {
                                append_trace_event(
                                    &trace_home,
                                    "ui",
                                    "app_control",
                                    "loop_error",
                                    json!({
                                        "pid": std::process::id(),
                                        "error": error.to_string(),
                                    }),
                                );
                                warn!(error=%error, "failed to process app control request");
                                break;
                            }
                        }
                    }
                    app_control_drain_in_flight.store(false, Ordering::SeqCst);
                    app_control_drain_started_ms.store(0, Ordering::SeqCst);
                });
            }
        }
        if let TaoEvent::WindowEvent { event, .. } = event {
            match event {
                DesktopWindowEvent::KeyboardInput { event, .. } => {
                    // Only a key that drives an APP-LEVEL action (escape-cancel-
                    // delete, delete-from-tree) needs to re-render the root. Plain
                    // character input is handled entirely by xterm/the PTY, and the
                    // clean ALT tap + KeyTips chord walk live in the below-the-
                    // webview JS bridge (keytip_alt_tap_install_loop) — the window-
                    // level tao key events here do NOT fire while the xterm.js
                    // webview holds focus, which is the whole §13.1 defect. Gate the
                    // window_epoch bump on a real action.
                    let mut app_action_handled = false;
                    if event.state == ElementState::Pressed
                        && (event.logical_key == TaoKey::Escape
                            || event.physical_key == TaoKeyCode::Escape)
                        && state.read().pending_delete.is_some()
                    {
                        state.with_mut_counted(|shell| shell.cancel_delete_dialog());
                        app_action_handled = true;
                    }
                    if event.state == ElementState::Pressed
                        && (event.logical_key == TaoKey::Delete
                            || event.physical_key == TaoKeyCode::Delete)
                        && state.read().delete_shortcut_should_target_tree()
                    {
                        queue_delete_selected_items(state, false);
                        app_action_handled = true;
                    }
                    if app_action_handled {
                        window_epoch.with_mut(|epoch| *epoch += 1);
                    }
                }
                DesktopWindowEvent::Moved(_)
                | DesktopWindowEvent::Resized(_)
                | DesktopWindowEvent::ScaleFactorChanged { .. } => {
                    state.with_mut_counted(sync_window_frame_state);
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
                DesktopWindowEvent::Focused(focused) => {
                    state.with_mut_counted(|shell| {
                        sync_window_frame_state(shell);
                        shell.set_window_focused(*focused);
                        // KeyTips exit on focus change (spec decision c): a mode
                        // that survived an Alt+Tab away would paint hints over a
                        // window the user is no longer driving.
                        if !*focused && shell.alt_overlay_active {
                            shell.clear_alt_overlay();
                        }
                    });
                    sync_active_terminal_input_policy(state);
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
                DesktopWindowEvent::CloseRequested => {
                    INTENTIONAL_CLIENT_SHUTDOWN.store(true, Ordering::SeqCst);
                    state.with_mut_counted(sync_window_frame_state);
                    if linux_close_requires_terminal_detach()
                        && state.read().server.active_session_path().is_some()
                        && state.read().server.active_view_mode() == WorkspaceViewMode::Terminal
                    {
                        window().set_decorations(true);
                    }
                    state.with_mut_counted(|shell| {
                        shell.closing_app = true;
                        shell.last_action = "closing yggterm".to_string();
                    });
                }
                _ => {}
            }
        }
    });
    {
        let should_start = {
            let shell = state.read();
            shell.needs_initial_server_sync
                && !startup_sync_render_launch.load(Ordering::SeqCst)
                && !*startup_sync_started.read()
        };
        if should_start {
            startup_sync_render_launch.store(true, Ordering::SeqCst);
            startup_sync_started.set(true);
            let defer_sync = {
                let shell = state.read();
                shell.had_cached_startup_snapshot
                    && shell.server.active_session().is_none()
                    && shell.server.active_view_mode() != WorkspaceViewMode::Terminal
            };
            let schedule_ui = schedule_ui_update.clone();
            spawn(async move {
                if defer_sync {
                    sleep(Duration::from_millis(DEFERRED_STARTUP_SYNC_MS)).await;
                }
                spawn_initial_server_sync(state, schedule_ui, async_render_epoch);
            });
        }
    }
    let schedule_ui_update_for_sync = schedule_ui_update.clone();
    use_effect(move || {
        let should_start = {
            let shell = state.read();
            shell.needs_initial_server_sync && !*startup_sync_started.read()
        };
        if should_start {
            startup_sync_started.set(true);
            let defer_sync = {
                let shell = state.read();
                shell.had_cached_startup_snapshot
                    && shell.server.active_session().is_none()
                    && shell.server.active_view_mode() != WorkspaceViewMode::Terminal
            };
            let schedule_ui = schedule_ui_update_for_sync.clone();
            spawn(async move {
                if defer_sync {
                    sleep(Duration::from_millis(DEFERRED_STARTUP_SYNC_MS)).await;
                }
                spawn_initial_server_sync(state, schedule_ui, async_render_epoch);
            });
        }
    });
    use_effect(move || {
        let (active_view_mode, active_session_path, latest_open_request_id) = {
            let shell = state.read();
            (
                shell.server.active_view_mode(),
                shell.server.active_session_path().map(ToOwned::to_owned),
                shell.latest_open_request_id,
            )
        };
        let Some(active_session_path) = active_session_path else {
            set_signal_if_changed(last_terminal_mount_key, None);
            return;
        };
        if active_view_mode != WorkspaceViewMode::Terminal {
            set_signal_if_changed(last_terminal_mount_key, None);
            return;
        }
        let mount_key = format!("{active_session_path}:{latest_open_request_id}");
        if *last_terminal_mount_key.read() == Some(mount_key.clone()) {
            return;
        }
        let defer_to_startup_restore_open =
            state.with(|shell| shell.startup_terminal_restore_should_open(&active_session_path));
        if defer_to_startup_restore_open {
            last_terminal_mount_key.set(Some(mount_key.clone()));
            append_trace_event(
                &trace_home_for_mount_epoch,
                "ui",
                "terminal_mount",
                "mount_epoch_deferred_to_startup_restore_open",
                json!({
                    "session_path": active_session_path,
                    "latest_open_request_id": latest_open_request_id,
                    "mount_key": mount_key,
                }),
            );
            return;
        }
        last_terminal_mount_key.set(Some(mount_key.clone()));
        let (mount_epoch, reused_live_host, settled_futile) = state.with_mut_counted(|shell| {
            // Open the reveal grace window BEFORE deciding reuse. A switch-back
            // to a host that previously reached ready is a REVEAL, not a fault:
            // the grace makes both the reuse predicate ignore a stale spurious
            // "recovering" attempt from a prior switch AND the fault-recovery
            // watchdog defer on the transient empty-surface repaint gap, so the
            // existing host (scrollback intact) is reused instead of cold
            // remounted. Gated on ready_history so fresh/never-ready sessions
            // are unaffected; genuine faults still recover after the grace.
            if shell.terminal_session_was_ever_ready(&active_session_path) {
                shell.terminal_reveal_grace_until_ms.insert(
                    active_session_path.clone(),
                    current_millis().saturating_add(RETAINED_REVEAL_EMPTY_GRACE_MS),
                );
            }
            let (mount_epoch, reused_live_host, settled_futile) =
                shell.resolve_active_open_mount_epoch(&active_session_path, current_millis());
            if shell.server.active_view_mode() == WorkspaceViewMode::Terminal
                && shell.server.active_session_path() == Some(active_session_path.as_str())
            {
                shell.active_terminal_host_id =
                    shell.terminal_session_host_id(&active_session_path);
            }
            (mount_epoch, reused_live_host, settled_futile)
        });
        append_trace_event(
            &trace_home_for_mount_epoch,
            "ui",
            "terminal_mount",
            if settled_futile {
                "mount_epoch_settled_futile"
            } else if reused_live_host {
                "mount_epoch_reused"
            } else {
                "mount_epoch_bump"
            },
            json!({
                "session_path": active_session_path,
                "latest_open_request_id": latest_open_request_id,
                "mount_key": mount_key,
                "mount_epoch": mount_epoch,
                "reused_live_host": reused_live_host,
                "settled_futile": settled_futile,
            }),
        );
    });
    let trace_home_for_startup_terminal_restore = trace_home.clone();
    use_effect(move || {
        let Some((active_session_path, row)) = state.with(|shell| {
            let active_session_path = shell.server.active_session_path()?.to_string();
            if !shell.startup_terminal_restore_should_open(&active_session_path) {
                return None;
            }
            let row = resolve_app_control_row(shell, &active_session_path)?;
            Some((active_session_path, row))
        }) else {
            set_signal_if_changed(last_startup_terminal_restore_path, None);
            return;
        };
        if *last_startup_terminal_restore_path.read() == Some(active_session_path.clone()) {
            return;
        }
        last_startup_terminal_restore_path.set(Some(active_session_path.clone()));
        append_trace_event(
            &trace_home_for_startup_terminal_restore,
            "ui",
            "terminal_mount",
            "startup_terminal_restore_open",
            json!({
                "session_path": active_session_path,
                "row_path": row.full_path,
                "row_kind": format!("{:?}", row.kind),
            }),
        );
        spawn_open_session_row_with_mode(state, row.clone(), true);
    });
    let trace_home_for_startup_terminal_recovery = trace_home.clone();
    use_effect(move || {
        let maybe_recovery = {
            let shell = state.read();
            let active_session_path = shell.server.active_session_path().map(str::to_string);
            active_session_path.and_then(|active_session_path| {
                if !shell
                    .startup_terminal_restore_should_recover(&active_session_path, current_millis())
                {
                    return None;
                }
                let row = resolve_app_control_row(&shell, &active_session_path)?;
                Some((active_session_path, row))
            })
        };
        let Some((active_session_path, row)) = maybe_recovery else {
            set_signal_if_changed(last_startup_terminal_recovery_path, None);
            return;
        };
        if *last_startup_terminal_recovery_path.read() == Some(active_session_path.clone()) {
            return;
        }
        last_startup_terminal_recovery_path.set(Some(active_session_path.clone()));
        append_trace_event(
            &trace_home_for_startup_terminal_recovery,
            "ui",
            "terminal_mount",
            "startup_terminal_restore_recover",
            json!({
                "session_path": active_session_path,
                "row_path": row.full_path,
                "row_kind": format!("{:?}", row.kind),
            }),
        );
        spawn_focus_live_session_row(state, row, WorkspaceViewMode::Terminal);
    });
    let schedule_ui_update_for_tree = schedule_ui_update.clone();
    use_effect(move || {
        let should_start = {
            let shell = state.read();
            shell.browser_tree_loading_in_flight && !*browser_tree_load_started.read()
        };
        if should_start {
            browser_tree_load_started.set(true);
            spawn_initial_browser_tree_load(
                state,
                schedule_ui_update_for_tree.clone(),
                async_render_epoch,
            );
        }
    });
    let window_spawn_traced_for_background_refresh = window_spawn_traced.clone();
    use_effect(move || {
        if *background_refresh_defer_started.read() {
            return;
        }
        if !window_spawn_traced_for_background_refresh.load(Ordering::SeqCst) {
            return;
        }
        background_refresh_defer_started.set(true);
        spawn(async move {
            loop {
                if safe_shell_read(state, "background_refresh_scheduler_closing", |shell| {
                    shell.closing_app
                })
                .unwrap_or(true)
                {
                    break;
                }
                maybe_spawn_background_live_session_snapshot(state);
                maybe_spawn_background_copy_generation(state);
                let defer_ms =
                    safe_shell_read(state, "background_refresh_scheduler_read", |shell| {
                        shell
                            .background_refresh_after_ms
                            .saturating_sub(current_millis())
                    })
                    .unwrap_or_default();
                if defer_ms > 0 {
                    let wait_ms = safe_shell_read(
                        state,
                        "background_refresh_scheduler_defer_wait",
                        |shell| {
                            background_refresh_scheduler_wait_ms(shell, current_millis(), defer_ms)
                        },
                    )
                    .unwrap_or(BACKGROUND_REFRESH_WAIT_POLL_MS);
                    sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                if safe_shell_read(state, "background_refresh_scheduler_gate", |shell| {
                    background_refresh_suspended(shell)
                })
                .unwrap_or(false)
                {
                    let wait_ms = safe_shell_read(
                        state,
                        "background_refresh_scheduler_suspended_wait",
                        |shell| {
                            background_refresh_scheduler_wait_ms(
                                shell,
                                current_millis(),
                                BACKGROUND_REFRESH_WAIT_POLL_MS,
                            )
                        },
                    )
                    .unwrap_or(BACKGROUND_REFRESH_WAIT_POLL_MS);
                    sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                maybe_spawn_missing_remote_machine_refreshes(state);
                maybe_spawn_missing_managed_cli_refreshes(state);
                maybe_spawn_background_allocator_trim(state, "background_idle");
                sleep(Duration::from_millis(BACKGROUND_REFRESH_POLL_MS)).await;
            }
        });
    });
    use_effect(move || {
        if *update_check_started.read() {
            return;
        }
        let should_start = {
            let shell = state.read();
            shell.bootstrap.install_context.channel != yggterm_core::InstallChannel::Unknown
        };
        if should_start {
            update_check_started.set(true);
            let install_context = state.read().bootstrap.install_context.clone();
            if install_context.update_policy == yggterm_core::UpdatePolicy::NotifyOnly
                || (install_context.update_policy == yggterm_core::UpdatePolicy::Auto
                    && std::env::var_os("YGGTERM_SKIP_SELF_UPDATE").is_none())
            {
                spawn_update_workflow(state, UpdateWorkflowTrigger::Startup);
            }
        }
    });
    use_effect(move || {
        let (policy_signature, trace_home) = {
            let shell = state.read();
            (
                active_terminal_input_policy_signature(&shell),
                perf_home_dir(&shell.bootstrap.settings_path),
            )
        };
        let previous_policy = last_active_terminal_input_policy.read().clone();
        if previous_policy.as_ref() == Some(&policy_signature) {
            return;
        }
        // Detect a background->foreground transition so the active terminal heals
        // its WebGL glyph atlas on foreground (see window_foreground repaint in
        // terminal_set_input_policy_script_for_active_session).
        let foreground_regained = previous_policy
            .as_ref()
            .is_some_and(|prev| !prev.window_focused && policy_signature.window_focused);
        last_active_terminal_input_policy.set(Some(policy_signature.clone()));
        apply_active_terminal_input_policy(&policy_signature, foreground_regained, &trace_home);
    });
    // ⭐ ARM (or disarm) the LEGACY BROWSER CHORDS for a keyboard the shell's
    // OWN webview holds — see `web_page_chords_serve_over_shell_focus` for why
    // the vendored claimer cannot work this out for itself. Pushed on the EDGE
    // only: the answer is re-derived every render, but the host is told just
    // when it changes.
    use_effect(move || {
        let armed = {
            let shell = state.read();
            let active_session_has_web_surface = shell
                .server
                .active_session_path()
                .is_some_and(|path| shell.has_live_web_surface(path, current_millis()));
            // The omnibox and the find bar, asked of the ACTIVE surface — the
            // same fact the reconciler stands the page's keyboard down for.
            let surface_control_holds_keyboard = shell
                .server
                .active_session_path()
                .and_then(|path| shell.web_surfaces.get(path))
                .is_some_and(web_surface_shell_control_holds_keyboard);
            web_page_chords_serve_over_shell_focus(
                shell.server.active_view_mode(),
                active_session_has_web_surface,
                shell.has_modal_over_viewport(),
                shell_dom_control_holds_keyboard(
                    shell.search_focused,
                    is_command_query(&shell.search_query),
                    titlebar_transient_focus_blocking(
                        shell.titlebar_new_menu_open,
                        shell.titlebar_session_menu_open,
                        shell.titlebar_overflow_menu_open,
                    ),
                    shell.tree_rename_path.is_some(),
                    shell.active_web_find_focused(),
                ),
                surface_control_holds_keyboard,
            )
        };
        if *last_web_page_chords_armed.peek() == Some(armed) {
            return;
        }
        last_web_page_chords_armed.set(Some(armed));
        window().set_web_surface_page_chords_armed(armed);
    });
    use_effect(move || {
        let active = state.read().server.active_session().cloned();
        let Some(session) = active else {
            set_signal_if_changed(last_preview_refresh_marker, None);
            return;
        };
        maybe_request_copy_generation_for_session(state, session);
    });
    let live_store_copy_signature = {
        let shell = state.read();
        shell
            .server
            .live_sessions()
            .iter()
            .map(|session| {
                (
                    session.id.clone(),
                    session.session_path.clone(),
                    session.title.clone(),
                    preview_summary_metadata_value(session, "Summary"),
                )
            })
            .collect::<Vec<_>>()
    };
    use_effect(move || {
        let _ = live_store_copy_signature;
        maybe_prime_live_session_store_copy(state);
    });
    let window_spawn_traced_for_preview_refresh = window_spawn_traced.clone();
    use_effect(move || {
        if !window_spawn_traced_for_preview_refresh.load(Ordering::SeqCst) {
            return;
        }
        let (active, view_mode, server_busy, dirty_epoch) = {
            let shell = state.read();
            (
                shell.server.active_session().cloned(),
                shell.server.active_view_mode(),
                shell.server_busy,
                shell
                    .server
                    .active_session_path()
                    .and_then(|path| shell.remote_preview_dirty_epoch.get(path).copied())
                    .unwrap_or(0),
            )
        };
        let Some(session) = active else {
            set_signal_if_changed(last_preview_refresh_marker, None);
            return;
        };
        if view_mode != WorkspaceViewMode::Rendered {
            set_signal_if_changed(last_preview_refresh_marker, None);
            return;
        }
        if server_busy {
            return;
        }
        if !session_preview_syncs_from_remote(&session.session_path) {
            set_signal_if_changed(last_preview_refresh_marker, None);
            return;
        }
        let needs_refresh = remote_preview_needs_refresh(&session);
        let should_auto_sync = remote_preview_should_auto_sync(&session);
        let refresh_marker = (
            session.session_path.clone(),
            dirty_epoch,
            needs_refresh,
            should_auto_sync,
        );
        if *last_preview_refresh_marker.read() == Some(refresh_marker.clone()) {
            return;
        }
        if !should_auto_sync {
            last_preview_refresh_marker.set(Some(refresh_marker));
            return;
        }
        let has_fetch_target =
            remote_preview_fetch_target(&state.read().server, &session).is_some();
        if has_fetch_target {
            let scheduled = state.with_mut_counted(|shell| {
                shell.record_preview_issue_telemetry(if needs_refresh {
                    "preview_refresh_request_placeholder"
                } else {
                    "preview_refresh_request_active"
                });
                schedule_remote_preview_sync(shell, &session.session_path, 0)
            });
            last_preview_refresh_marker.set(Some(refresh_marker));
            if scheduled {
                spawn_remote_preview_payload_sync(
                    state,
                    session.session_path.clone(),
                    if needs_refresh {
                        "preview_refresh_request_placeholder"
                    } else {
                        "preview_refresh_request_active"
                    },
                );
            }
        } else if needs_refresh {
            let scheduled = state.with_mut_counted(|shell| {
                shell.record_preview_issue_telemetry("preview_refresh_no_target");
                schedule_remote_preview_sync(
                    shell,
                    &session.session_path,
                    REMOTE_PREVIEW_NO_TARGET_RETRY_MS,
                )
            });
            if scheduled {
                schedule_remote_preview_retry_tick(
                    state,
                    session.session_path.clone(),
                    REMOTE_PREVIEW_NO_TARGET_RETRY_MS,
                );
            }
            set_signal_if_changed(last_preview_refresh_marker, None);
        } else {
            last_preview_refresh_marker.set(Some(refresh_marker));
        }
    });
    // Build the per-render snapshot ONCE here and reuse it for the canonical
    // render below. `snapshot()` clones the full ~223-row sidebar projection;
    // this render body previously built it twice (here for the bounds-repair
    // key, then again ~325 lines down for the actual render). The render body
    // never synchronously mutates the `state` signal between the two points, so
    // both builds reflected identical ShellState — collapsing to one is
    // behavior-equivalent and removes a whole row-clone pass per render.
    // See [[finding-gui-latency-render-path-campaign]].
    // Shared through the epoch cache: a root render with no state write in
    // front of it (the measured ~11% forced-wake amplification) reuses the
    // previous merge instead of rebuilding it.
    let snapshot: SharedSnapshot = state.read().snapshot_shared();
    // External search-value sync: the titlebar input is UNCONTROLLED (typing
    // never re-renders its value — the controlled rewrite raced the
    // per-keystroke tree rebuild and ate characters), so external writers
    // (Escape, the clear chip, app-control `search set`) bump
    // `search_value_epoch` and this effect pushes the value into the DOM.
    // An epoch-keyed node rebuild was tried first and does NOT fire: Dioxus
    // keys drive list diffing, not single static children. Reads state
    // inside the closure — an effect subscribes only to signals read during
    // its last run (the sidebar bounds-repair lesson, same day).
    use_effect(move || {
        let (epoch, query) = {
            let shell = state.read();
            (shell.search_value_epoch, shell.search_query.clone())
        };
        if *last_search_value_epoch_sync.read() == Some(epoch) {
            return;
        }
        last_search_value_epoch_sync.set(Some(epoch));
        let Ok(query_literal) = serde_json::to_string(&query) else {
            return;
        };
        let _ = document::eval(&format!(
            "(function() {{
                const input = document.getElementById({SEARCH_INPUT_ID:?});
                if (input && input.value !== {query_literal}) {{
                    input.value = {query_literal};
                }}
            }})();"
        ));
    });
    use_effect(move || {
        let (scroll_path, show_loading_tree, suppress_autoscroll, bounds_repair_key) = {
            let shell = state.read();
            let active_path = shell.server.active_session_path().map(ToOwned::to_owned);
            let snapshot = shell.snapshot();
            let selected_path = shell
                .selection_anchor
                .as_ref()
                .filter(|path| snapshot.rows.iter().any(|row| row.full_path == **path))
                .cloned()
                .or_else(|| {
                    shell
                        .browser
                        .selected_path()
                        .filter(|path| snapshot.rows.iter().any(|row| row.full_path == *path))
                        .map(ToOwned::to_owned)
                });
            // The bounds-repair key MUST be computed inside this effect: an
            // effect only re-runs when a signal it read during its last run
            // changes, so a key captured from the render body left the repair
            // subscribed to nothing but its own dedup signal — it fired once
            // at startup and never again. Collapsing a long subtree then
            // stranded scrollTop past the shrunken scrollHeight (WebKitGTK
            // does not self-clamp on content shrink) and the sidebar stopped
            // scrolling with Live Sessions clipped until the user re-expanded
            // something tall. `rows.len()` is what a collapse changes.
            let bounds_repair_key = format!(
                "{}:{}:{}:{}",
                snapshot.rows.len(),
                snapshot.show_loading_tree,
                active_path.as_deref().unwrap_or("<none>"),
                snapshot.selected_path.as_deref().unwrap_or("<none>")
            );
            let active_visible_path =
                active_path.filter(|path| snapshot.rows.iter().any(|row| row.full_path == *path));
            let suppress_autoscroll = shell.tree_rename_path.is_some()
                || current_millis() < shell.suppress_sidebar_autoscroll_until_ms;
            (
                selected_path.or(active_visible_path),
                snapshot.show_loading_tree,
                suppress_autoscroll,
                bounds_repair_key,
            )
        };
        if *last_sidebar_bounds_repair_key.read() != Some(bounds_repair_key.clone()) {
            last_sidebar_bounds_repair_key.set(Some(bounds_repair_key));
            let _ = document::eval(sidebar_scroll_bounds_repair_script());
        }
        if show_loading_tree {
            set_signal_if_changed(last_sidebar_autoscroll_path, None);
            return;
        }
        let Some(scroll_path) = scroll_path else {
            set_signal_if_changed(last_sidebar_autoscroll_path, None);
            return;
        };
        if suppress_autoscroll {
            set_signal_if_changed(last_sidebar_autoscroll_path, Some(scroll_path));
            return;
        }
        if *last_sidebar_autoscroll_path.read() == Some(scroll_path.clone()) {
            return;
        }
        last_sidebar_autoscroll_path.set(Some(scroll_path.clone()));
        let row_id = sidebar_row_dom_id(&scroll_path);
        let _ = document::eval(&sidebar_autoscroll_script(&row_id));
    });
    use_effect(move || {
        let target_dom_id = {
            let snapshot = state.read().snapshot();
            snapshot.search_content_hit_index.and_then(|ix| {
                snapshot
                    .search_content_hits
                    .get(ix)
                    .map(|hit| hit.dom_id.clone())
            })
        };
        let Some(dom_id) = target_dom_id else {
            return;
        };
        let _ = document::eval(&format!(
            "(function() {{
                const el = document.getElementById({dom_id:?});
                if (el) {{
                    el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'smooth' }});
                }}
            }})();"
        ));
    });
    use_effect(move || {
        if *dock_pulse_started.read() {
            return;
        }
        dock_pulse_started.set(true);
        spawn(async move {
            loop {
                let should_pulse = {
                    let shell = state.read();
                    shell.docked_window_id.is_some()
                        || shell.server.active_session().is_some_and(|session| {
                            session.backend == TerminalBackend::Ghostty
                                && session.terminal_host_mode
                                    == GhosttyTerminalHostMode::ControlledDock
                        })
                };
                if should_pulse {
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
                let sleep_ms = if should_pulse {
                    DOCK_PULSE_ACTIVE_MS
                } else {
                    DOCK_PULSE_IDLE_MS
                };
                let _ =
                    task::spawn_blocking(move || thread::sleep(Duration::from_millis(sleep_ms)))
                        .await;
            }
        });
    });
    let dock_desktop = desktop.clone();
    use_effect(move || {
        let _ = *window_epoch.read();
        let snapshot = state.read().snapshot();
        let request = ghostty_dock_request(&dock_desktop, &snapshot);
        let (should_sync, window_to_hide) = {
            let shell = state.read();
            let should_sync = request.as_ref().is_some_and(|request| {
                shell.docked_window_id.is_none()
                    || shell.last_dock_signature.as_ref() != Some(&request.signature())
            });
            let window_to_hide = if request.is_none() {
                shell.docked_window_id.clone().map(|window_id| {
                    (
                        shell
                            .server
                            .active_session()
                            .and_then(|session| session.terminal_process_id),
                        window_id,
                    )
                })
            } else {
                None
            };
            (should_sync, window_to_hide)
        };
        if let Some(request) = request
            && should_sync
        {
            spawn_dock_sync(state, request);
        } else if let Some((pid, window_id)) = window_to_hide {
            spawn_dock_hide(state, pid, window_id);
        }
    });
    use_future(move || {
        let state = state;
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(140)).await;
                let should_continue = {
                    let shell = state.read();
                    shell.browser_tree_loading_in_flight
                        || (shell.needs_initial_server_sync && !shell.had_cached_startup_snapshot)
                        || shell.server_busy
                };
                if !should_continue {
                    break;
                }
            }
        }
    });
    // HOT-tier pre-attach per [[spec-xterm-gating-ux]] Phase 2: every 5s,
    // pick the top-N most-recently-used keep-alive Codex sessions that the
    // daemon does NOT currently own and silently `terminal_ensure` them so
    // the next click takes the HOT path. Capped at HOT_WARM_MAX_CONCURRENT.
    use_future(move || {
        let mut state = state;
        async move {
            // Defer initial check so the daemon has a chance to publish its
            // current ownership state — warming on a stale snapshot would
            // fire ensures for sessions the daemon JUST claimed.
            tokio::time::sleep(std::time::Duration::from_millis(8_000)).await;
            loop {
                let candidates: Vec<String> = state
                    .with_mut(|shell| shell.tick_hot_warmer(current_millis()));
                for path in candidates {
                    let path_for_task = path.clone();
                    spawn(async move {
                        let endpoint =
                            state.with(|shell| shell.bootstrap.server_endpoint.clone());
                        let result = tokio::task::spawn_blocking(move || {
                            terminal_ensure(&endpoint, &path_for_task)
                        })
                        .await;
                        let trace_path = path.clone();
                        match result {
                            Ok(Ok(_)) => {
                                append_ui_telemetry_event(
                                    "hot_warm_ensure_ok",
                                    json!({"session_path": trace_path}),
                                );
                            }
                            Ok(Err(error)) => {
                                append_ui_telemetry_event(
                                    "hot_warm_ensure_error",
                                    json!({
                                        "session_path": trace_path,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                            Err(error) => {
                                append_ui_telemetry_event(
                                    "hot_warm_ensure_panic",
                                    json!({
                                        "session_path": trace_path,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                        state.with_mut_counted(|shell| shell.finish_hot_warm(&path));
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(
                    HOT_WARM_CHECK_INTERVAL_MS,
                ))
                .await;
            }
        }
    });
    // ⛔ THE INPUT GATE'S DEADLINE. Nothing else in the app polls "is the user
    // still being refused?", and the gate itself is computed from state rather
    // than driven by a timer — so without this loop a row that leaves the ready
    // set with no attempt left to re-enter it stays untypeable for the rest of
    // its life. See INPUT_GATE_STUCK_RESTORE_AFTER_MS.
    use_future(move || {
        let mut state = state;
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(
                    INPUT_GATE_DEADLINE_TICK_MS,
                ))
                .await;
                // ⛔ A tick with nothing to do must not take `with_mut`, which
                // dirties the shell signal on drop whether or not the closure
                // wrote — one full app-root re-render per second, forever, on
                // an app that is doing nothing. `peek()` neither subscribes nor
                // dirties, and both predicates answer "inert" only when the
                // work below is PROVABLY a no-op, so this can never skip a tick
                // that would have changed something.
                let inert = {
                    let shell = state.peek();
                    shell.input_gate_deadline_tick_is_inert()
                        && shell.restore_card_refresh_is_inert()
                };
                if inert {
                    continue;
                }
                let restored = state.with_mut_counted(|shell| {
                    // Same tick: a restore card that is up must keep telling the
                    // truth about which stage it is in, or its bar freezes at
                    // whatever the stage was when it was raised.
                    shell.refresh_terminal_restore_card();
                    shell.tick_input_gate_deadline(current_millis())
                });
                if restored.is_some() {
                    // The policy is derived per render from shell state, and a
                    // set insertion alone does not re-run it — push it now so
                    // the keyboard comes back on this tick, not on the next
                    // unrelated re-render.
                    sync_active_terminal_input_policy(state);
                }
            }
        }
    });
    use_effect(move || {
        let rename_path = state.read().tree_rename_path.clone();
        let rename_depth = state.read().tree_rename_depth;
        let rename_started_at_ms = state.read().tree_rename_started_at_ms;
        let rename_initial_value = state.read().tree_rename_value.clone();
        match rename_path {
            Some(rename_path) => {
                if *last_tree_rename_focus_path.read() == Some(rename_path.clone()) {
                    return;
                }
                last_tree_rename_focus_path.set(Some(rename_path.clone()));
                spawn(async move {
                    let path_literal = match serde_json::to_string(&rename_path) {
                        Ok(value) => value,
                        Err(_) => return,
                    };
                    let rename_token = match serde_json::to_string(&format!(
                        "{}\u{0}{}\u{0}{}",
                        rename_path,
                        rename_depth
                            .map(|depth| depth.to_string())
                            .unwrap_or_default(),
                        rename_started_at_ms
                    )) {
                        Ok(value) => value,
                        Err(_) => return,
                    };
                    let initial_value_literal = match serde_json::to_string(&rename_initial_value) {
                        Ok(value) => value,
                        Err(_) => return,
                    };
                    let depth_selector = rename_depth
                        .map(|depth| format!(r#"[data-tree-rename-row-depth="{depth}"]"#))
                        .unwrap_or_default();
                    for delay_ms in [0_u64, 40, 120, 240, 420, 700, 1000, 1400] {
                        if delay_ms > 0 {
                            sleep(Duration::from_millis(delay_ms)).await;
                        }
                        let _ = document::eval(&format!(
                            r#"(function() {{
                                const renamePath = {path_literal};
                                const renameToken = {rename_token};
                                const initialValue = {initial_value_literal};
                                const selector = '[data-tree-rename-input="1"][data-tree-rename-row-path='
                                    + JSON.stringify(renamePath)
                                    + ']{depth_selector}';
                                const input = document.querySelector(selector);
                                if (!input || !input.isConnected) {{
                                    return;
                                }}
                                const rect = input.getBoundingClientRect();
                                if (rect.width <= 0 || rect.height <= 0) {{
                                    return;
                                }}
                                try {{
                                    input.focus({{ preventScroll: true }});
                                    const currentValue = String(input.value || '');
                                    const valueLength = Number(input.value ? input.value.length : 0);
                                    const tokenAlreadyApplied =
                                        input.getAttribute('data-tree-rename-selected-token') === renameToken
                                        || window.__yggtermTreeRenameSelectToken === renameToken;
                                    if (tokenAlreadyApplied) {{
                                        return;
                                    }}
                                    if (currentValue !== initialValue) {{
                                        window.__yggtermTreeRenameSelectToken = renameToken;
                                        input.setAttribute('data-tree-rename-selected-token', renameToken);
                                        return;
                                    }}
                                    const shouldSelect =
                                        currentValue === initialValue
                                        && document.activeElement === input;
                                    if (shouldSelect && input.select) {{
                                        input.select();
                                        const selectedStart = typeof input.selectionStart === 'number' ? Number(input.selectionStart) : -1;
                                        const selectedEnd = typeof input.selectionEnd === 'number' ? Number(input.selectionEnd) : -1;
                                        if (selectedStart === 0 && selectedEnd === valueLength) {{
                                            window.__yggtermTreeRenameSelectToken = renameToken;
                                            input.setAttribute('data-tree-rename-selected-token', renameToken);
                                        }}
                                    }} else if (typeof input.setSelectionRange === 'function') {{
                                        input.setSelectionRange(valueLength, valueLength);
                                        window.__yggtermTreeRenameSelectToken = renameToken;
                                        input.setAttribute('data-tree-rename-selected-token', renameToken);
                                    }}
                                }} catch (_error) {{}}
                            }})();"#
                        ));
                    }
                });
            }
            None => {
                if last_tree_rename_focus_path.read().is_some() {
                    last_tree_rename_focus_path.set(None);
                }
            }
        }
    });
    let _ = *async_render_epoch.read();
    let tree_rename_depth = state.read().tree_rename_depth;
    // `snapshot` (the canonical SharedSnapshot) was already built once near the
    // top of this render body — reused here instead of rebuilding the ~223-row
    // projection a second time. See [[finding-gui-latency-render-path-campaign]].
    let app_control_backgrounded = state.read().app_control_backgrounded;
    let window_focused = state.read().effective_window_focused();
    let inner = desktop.inner_size();
    // PHYSICAL px → CSS px, once. A menu anchors at `evt.client_coordinates()`,
    // which is CSS; `inner_size()` is physical, so on a scale_factor 2 host the
    // un-converted window was twice the size the click lived in and no menu ever
    // reached its right/bottom flip.
    let context_menu_window_size = window_css_size(
        (inner.width as f64, inner.height as f64),
        desktop.scale_factor(),
    );
    // ★ THE MIRROR, asked ONCE per render. Every side-dependent decision below —
    // workspace order, panel edges, resize signs, menu bands, page-edge reveal —
    // reads this and nothing else. Adding a second place that tests
    // `settings.chrome_orientation` to decide a side is the SSOT violation this
    // whole type exists to prevent.
    let chrome_orientation = snapshot.settings.chrome_orientation;
    // The RAIL's band, derived once, handed to the mounts whose menus are raised
    // from the rail. See [`ContextMenuBand`]: in legacy stacking anything that
    // spills out of the rail is eaten by the native page beside it.
    let context_menu_rail_band = rail_context_menu_band(
        chrome_orientation.edge(ChromeSlot::Rail),
        context_menu_window_size.0,
        snapshot.rail_width as f64,
        ui_zoom_factor(snapshot.settings.ui_font_size),
    );
    // The TREE's band, for the same reason and only when it is needed:
    // `active_web_surface_overlay` is Some exactly while a native page owns the
    // active viewport ([`ShellState::web_surface_overlay_for_session`] on the
    // active session), which is the only state in which DOM spilling out of the
    // tree is eaten.
    let context_menu_sidebar_band = snapshot.active_web_surface_overlay.as_ref().map(|_| {
        sidebar_context_menu_band(
            chrome_orientation.edge(ChromeSlot::Tree),
            context_menu_window_size.0,
            snapshot.sidebar_width as f64,
            ui_zoom_factor(snapshot.settings.ui_font_size),
        )
    });
    let titlebar_snapshot = snapshot.clone();
    let sidebar_snapshot = snapshot.clone();
    let main_snapshot = snapshot.clone();
    let metadata_snapshot = snapshot.clone();
    // ── Sidebar handlers, hoisted into stable callbacks ─────────────────────
    //
    // ⛔ An INLINE closure prop defeats component memoization outright:
    // `Callback::eq` is ptr_eq on the generational box, and rsx allocates a
    // fresh box per render, so `Sidebar`'s props never compared equal and it
    // re-rendered its ~200-row tree on 232 of 234 root renders — including
    // every render whose write changed nothing the sidebar shows (the blink
    // storm, `dioxus_render/component_window` 2026-08-20). Verified directly
    // against dioxus-core 0.7.9: with identical data props, an inline handler
    // re-renders the child (1→2) and a `use_callback`-stable one skips it
    // (1→1). These hooks keep one box alive across renders so the props
    // boundary can do its job; the closure BODIES are unchanged, merely moved.
    let sidebar_on_prev_search_row = use_callback(move |_: ()| {
        if let Some(row) = state.with_mut_counted(|shell| shell.next_search_sidebar_row(-1)) {
            spawn_open_session_row(state, row);
        }
    });
    let sidebar_on_next_search_row = use_callback(move |_: ()| {
        if let Some(row) = state.with_mut_counted(|shell| shell.next_search_sidebar_row(1)) {
            spawn_open_session_row(state, row);
        }
    });
    let sidebar_on_select_all_rows =
        use_callback(move |_: ()| state.with_mut_counted(|shell| shell.select_all_tree_rows()));
    let sidebar_on_navigate_rows = use_callback(move |(delta, to_edge): (i32, bool)| {
        let focused =
            state.with_mut_counted(|shell| shell.navigate_sidebar_selection(delta, to_edge));
        // Keep the keyboard cursor visible + the sidebar the
        // keyboard owner so the next arrow key lands here too.
        if let Some(path) = focused {
            scroll_sidebar_row_into_view(&path);
        }
    });
    let sidebar_on_start_sidebar_resize = use_callback(move |client_x: f64| {
        state.with_mut_counted(|shell| shell.start_sidebar_resize(client_x))
    });
    let sidebar_on_focus_split_pane = use_callback(move |path: String| {
        focus_split_pane(state, &path);
    });
    let sidebar_on_select_row = use_callback(move |(row, mode): (BrowserRow, TreeSelectionMode)| {
        let row_for_log = row.clone();
        if let Err(error) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (should_continue, terminal_activation) = {
                let mut continue_open = true;
                let mut should_reopen_selected_session = true;
                let mut terminal_activation = false;
                state.with_mut_counted(|shell| {
                    if shell.consume_suppressed_tree_click() {
                        continue_open = false;
                        return;
                    }
                    let row_already_selected = shell.selected_tree_paths.contains(&row.full_path)
                        || shell.browser.selected_path() == Some(row.full_path.as_str());
                    should_reopen_selected_session = !(mode == TreeSelectionMode::Replace
                        && matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document)
                        && row_already_selected
                        && shell.server.active_session_path() == Some(row.full_path.as_str()));
                    terminal_activation = mode == TreeSelectionMode::Replace
                        && matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document)
                        && preferred_open_mode_for_row(shell, &row) == WorkspaceViewMode::Terminal;
                    shell.select_tree_row(&row, mode);
                    if terminal_activation {
                        arm_terminal_activation(shell, &row.full_path);
                    }
                });
                (
                    continue_open && should_reopen_selected_session,
                    terminal_activation,
                )
            };
            if terminal_activation {
                clear_sidebar_keyboard_owner();
            } else {
                claim_sidebar_focus_by_path(Some(&row.full_path));
            }
            if !should_continue {
                if terminal_activation {
                    schedule_terminal_focus_after_activation(state, row.full_path.clone());
                }
                return;
            }
            if mode != TreeSelectionMode::Replace {
                return;
            }
            match row.kind {
                BrowserRowKind::Group | BrowserRowKind::Separator => {
                    state.with_mut_counted(|shell| shell.select_row(&row))
                }
                BrowserRowKind::Session | BrowserRowKind::Document => {
                    spawn_open_session_row(state, row.clone());
                    if terminal_activation {
                        schedule_terminal_focus_after_activation(state, row.full_path.clone());
                    }
                }
            }
        })) {
            warn!(
                path=%row_for_log.full_path,
                kind=?row_for_log.kind,
                mode=?mode,
                panic_payload=?error,
                "suppressed sidebar row select panic"
            );
        }
    });
    let sidebar_on_press_highlight_row =
        use_callback(move |(row, mode): (BrowserRow, TreeSelectionMode)| {
            // #14: instant highlight on press (no session open/switch).
            state.with_mut_counted(|shell| shell.select_tree_row(&row, mode));
            claim_sidebar_focus_by_path(Some(&row.full_path));
        });
    let sidebar_on_set_row_expanded = use_callback(move |(row, expanded): (BrowserRow, bool)| {
        state.with_mut_counted(|shell| {
            set_app_control_row_expanded(shell, &row, expanded);
            shell.last_action = if expanded {
                format!("expanded {}", row.label)
            } else {
                format!("collapsed {}", row.label)
            };
            shell.refresh_tree_debug("toggle_group_expanded");
        });
    });
    let sidebar_on_delete_selected_items =
        use_callback(move |hard_delete: bool| queue_delete_selected_items(state, hard_delete));
    let sidebar_on_delete_row = use_callback(move |row: BrowserRow| {
        state.with_mut_counted(|shell| shell.open_delete_dialog_for_row(&row, false));
    });
    let sidebar_on_open_context_menu =
        use_callback(move |(row, position): (BrowserRow, (f64, f64))| {
            let focus_path = row.full_path.clone();
            state.with_mut_counted(|shell| {
                if !shell.selected_tree_paths.contains(&row.full_path) {
                    shell.select_tree_row(&row, TreeSelectionMode::Replace);
                }
                shell.open_context_menu(row, position)
            });
            claim_sidebar_focus_by_path(Some(&focus_path));
        });
    let sidebar_on_start_drag = use_callback(move |(row, pointer): (BrowserRow, (f64, f64))| {
        state.with_mut_counted(|shell| shell.update_tree_drag_pointer(&row, pointer))
    });
    let sidebar_on_drag_hover = use_callback(
        move |(row, pointer, placement): (BrowserRow, (f64, f64), DragDropPlacement)| {
            if state
                .read()
                .drag_hover_update_needed(&row, pointer, placement)
            {
                state.with_mut_counted(|shell| shell.set_drag_hover_target(&row, pointer, placement))
            }
        },
    );
    let sidebar_on_drag_move = use_callback(move |pointer: (f64, f64)| {
        if state.read().drag_pointer_update_needed(pointer) {
            state.with_mut_counted(|shell| shell.update_drag_pointer(pointer))
        } else if state.read().pending_tree_drag.is_some() {
            state.with_mut_counted(|shell| shell.update_pending_tree_drag_pointer(pointer))
        }
    });
    let sidebar_on_drag_leave = use_callback(move |_row: BrowserRow| {});
    let sidebar_on_drop_into_row =
        use_callback(move |_: ()| queue_drop_current_drag_target(state));
    let sidebar_on_end_drag =
        use_callback(move |_: ()| state.with_mut_counted(|shell| shell.clear_drag_state()));
    let sidebar_on_begin_rename = use_callback(move |row: BrowserRow| {
        state.with_mut_counted(|shell| shell.begin_tree_rename(&row));
        sync_active_terminal_input_policy(state);
    });
    let sidebar_on_regenerate_row_title = use_callback(move |row: BrowserRow| {
        queue_rename_field_ai_title_generation(state, row);
    });
    let sidebar_on_update_rename = use_callback(move |value: String| {
        state.with_mut_counted(|shell| shell.update_tree_rename_value(value))
    });
    let sidebar_on_focus_rename = use_callback(move |_: ()| {
        state.with_mut_counted(|shell| shell.note_tree_rename_input_focus())
    });
    let sidebar_on_commit_rename = use_callback(move |row: BrowserRow| {
        let label = {
            let shell = state.read();
            if shell.tree_rename_path.as_deref() != Some(row.full_path.as_str()) {
                return;
            }
            shell.tree_rename_value.clone()
        };
        queue_tree_rename(state, row, label);
    });
    let sidebar_on_cancel_rename = use_callback(move |_: ()| {
        state.with_mut_counted(|shell| shell.cancel_tree_rename());
        sync_active_terminal_input_policy(state);
    });
    // A native child webview paints above ALL DOM. An auto-hidden titlebar is
    // `position:absolute` OVER the content — and a web surface would swallow
    // that whole: the titlebar (app menu, the ychrome/incognito identity, the
    // window buttons) vanishes, and it cannot even be hovered back, because the
    // reveal sensor is under the webview too.
    //
    // The answer is NOT to give up auto-hide over a web surface, and not to
    // move the titlebar into flow either (that made every reveal re-lay-out the
    // whole window — user report 2026-07-13). The titlebar stays the SAME
    // floating overlay everywhere; what moves is the NATIVE WEBVIEW: the
    // surface reconciler clamps its rect below the titlebar's live bottom edge,
    // so the collapsed sensor strip is real hoverable DOM and a revealed
    // titlebar sits on top of everything with only the page dipping under it.
    let titlebar_auto_hide_enabled = snapshot.settings.auto_hide_titlebar;
    let titlebar_reveal_pinned = titlebar_autohide_pinned(&snapshot);
    let titlebar_revealed = titlebar_autohide.revealed(titlebar_auto_hide_enabled, titlebar_reveal_pinned);
    // A HIDDEN sidebar is an auto-hide sidebar — that IS what hidden means now,
    // with no settings toggle of its own (user direction 2026-07-21). Closed
    // means "reveal on hover, over the viewport", not "gone".
    let left_sidebar_auto_hide = !snapshot.sidebar_open;
    // Split out so the DOM can say WHY the panel is open: a gesture pin (this)
    // or a hover. `focus_within` counts as a pin for the same reason the
    // gestures do — a settings field being typed into is not a hover.
    let left_sidebar_pinned = (sidebar_autohide_pinned(&snapshot)
        || left_sidebar_autohide.focus_within_active())
        && left_sidebar_auto_hide;
    let left_sidebar_revealed = left_sidebar_autohide.revealed(
        left_sidebar_auto_hide,
        sidebar_autohide_pinned(&snapshot),
    ) && left_sidebar_auto_hide;
    let right_rail_auto_hide = snapshot.right_panel_mode == RightPanelMode::Hidden;
    let right_rail_pinned = (rail_autohide_pinned(&snapshot)
        || right_rail_autohide.focus_within_active())
        && right_rail_auto_hide;
    let right_rail_revealed = right_rail_autohide
        .revealed(right_rail_auto_hide, rail_autohide_pinned(&snapshot))
        && right_rail_auto_hide;
    let maximized = snapshot.maximized;
    // Distraction-free mode: the user's, sticky. Used ONLY where the question
    // really is "did the user ask for distraction-free" — the window-control
    // icon and the exit affordance below.
    let fullscreen = snapshot.fullscreen;
    // …and the chrome gate every surface shares. A page in ELEMENT fullscreen
    // owns the window exactly as distraction-free mode does, so it hides the
    // same chrome through the same gate; two conditions would be two lists of
    // "what chrome is", and the one that got forgotten is what painted the
    // session tree over the user's video.
    let chrome_hidden = snapshot.chrome_hidden();
    let distraction_free_exit_visible = snapshot.distraction_free_exit_visible();
    let shell_radius = if maximized {
        0
    } else {
        UNMAXIMIZED_SHELL_RADIUS_PX
    };
    let effective_shell_radius =
        shell_effective_radius(shell_radius, maximized, linux_transparent_window);
    let shell_root_background = shell_root_background(snapshot.palette, linux_transparent_window);
    let linux_native_decorations = linux_force_native_decorations();
    let shape_desktop = desktop.clone();
    let trace_home_for_shape = trace_home.clone();
    let chrome_apply_signature = LinuxWindowChromeApplySignature {
        transparent_window: linux_transparent_window,
        radius: effective_shell_radius,
        maximized,
        native_decorations: linux_native_decorations,
        width: inner.width,
        height: inner.height,
    };
    use_effect(move || {
        let _ = *window_epoch.read();
        let already_applied = {
            let last = *last_linux_window_chrome_apply.read();
            !linux_window_chrome_apply_needed(last, chrome_apply_signature)
        };
        if already_applied {
            return;
        }
        last_linux_window_chrome_apply.set(Some(chrome_apply_signature));
        if linux_native_decorations {
            return;
        }
        apply_linux_transparent_window_surface_style(
            &shape_desktop,
            linux_transparent_window,
            effective_shell_radius,
        );
        apply_linux_window_corner_shape(&shape_desktop, effective_shell_radius, maximized);
        apply_linux_compositor_blur(
            &shape_desktop,
            linux_transparent_window,
            &trace_home_for_shape,
        );
        apply_linux_window_shape_reapply_sequence(
            &shape_desktop,
            effective_shell_radius,
            maximized,
        );
    });
    let always_on_top_desktop = desktop.clone();
    let always_on_top = snapshot.always_on_top;
    use_effect(move || {
        apply_linux_always_on_top_state(&always_on_top_desktop, always_on_top);
    });
    // The focus ring's colour is the theme accent, published once as a CSS var
    // so `FORM_DIALOG_FOCUS_CSS` stays a static sheet and still follows the
    // theme (DESIGN.md ▸ Keyboard focus ring).
    let focus_ring_var = format!(
        ":root {{ --yggterm-focus-ring: {}; }} ",
        snapshot.palette.accent
    );
    let shell_document_css = focus_ring_var
        + &"html, body, #main { margin:0; width:100%; height:100%; background:transparent !important; overflow:hidden; } \
         body { overscroll-behavior:none; } \
         [data-yggterm-app-control-backgrounded=\"true\"] .yggterm-loading-dot, \
         [data-yggterm-app-control-backgrounded=\"true\"] .yggterm-tree-spinner, \
         [data-yggterm-window-focused=\"false\"] .yggterm-loading-dot, \
         [data-yggterm-window-focused=\"false\"] .yggterm-tree-spinner { animation:none !important; }"
            .to_string();
    // The session-style row's reveal rule, resolved against THIS window's
    // palette (the frosted chip's wash is the surface's own colour).
    let session_row_hover_css = session_row_hover_css(snapshot.palette);
    // Every text field's skin, resolved against THIS window's palette. Same
    // reason it is here and not in a rail body: one owner, every surface.
    let text_field_css = text_field_css(snapshot.palette);
    let context_menu_overlay = snapshot.context_menu_row.clone();
    rsx! {
        div {
            id: "yggterm-shell-root",
            tabindex: "0",
            "data-yggterm-app-control-backgrounded": if app_control_backgrounded { "true" } else { "false" },
            "data-yggterm-window-focused": if window_focused { "true" } else { "false" },
            style: format!(
                "position:relative; width:100vw; height:100vh; overflow:hidden; background:{}; box-shadow:none; box-sizing:border-box; \
                 border-radius:{}px; clip-path:inset(0 round {}px); -webkit-clip-path:inset(0 round {}px);",
                shell_root_background,
                effective_shell_radius,
                effective_shell_radius,
                effective_shell_radius,
            ),
            onclick: move |_| {
                let active_terminal_session = state.with_mut_counted(|shell| {
                    shell.clear_alt_overlay();
                    if shell.server.active_view_mode() != WorkspaceViewMode::Terminal {
                        return None;
                    }
                    let path = shell
                        .server
                        .active_session()
                        .map(|session| session.session_path.clone())?;
                    // A surface covering the viewport OWNS the keyboard. This
                    // root-level onclick fires for EVERY click in the window —
                    // including clicks on the covering surface's own inputs, which
                    // bubble up here — so reclaiming terminal focus would yank focus
                    // straight out of the field the user just clicked.
                    //
                    // A live WEB surface was taught this (the "click the new-profile
                    // field and it loses focus immediately" bug). The DOCUMENT
                    // surface — yedit's editor — was not, and that is THE
                    // "focus is stolen, spam-click to type" bug: this handler is the
                    // fourth focus path, the one the reclaim/input-policy/allowlist
                    // hardenings all missed because it is not a focus-arbitration
                    // script at all, just a click handler that refocuses.
                    // Same rule the input policy uses
                    // (apply_active_terminal_input_policy): a surface covers the
                    // viewport => the terminal does not get focus.
                    if shell.has_live_web_surface(&path, current_millis())
                        || shell.document_surface_visible_for(&path)
                    {
                        None
                    } else {
                        Some(path)
                    }
                });
                dismiss_titlebar_transients_and_resync_active_terminal(state);
                if let Some(session_path) = active_terminal_session {
                    if let Ok(session_path_literal) = serde_json::to_string(&session_path) {
                        let _ = document::eval(&root_click_terminal_focus_script(
                            &session_path_literal,
                        ));
                    }
                }
            },
            onmouseup: move |_| {
                if !state.read().drag_paths.is_empty() {
                    queue_drop_current_drag_target(state);
                    state.with_mut_counted(|shell| shell.clear_drag_state());
                }
                // The rail's gesture ends here too. Row → pane container → root:
                // by the time this runs a real drop has already been taken and
                // cleared, so this only catches the releases that landed outside
                // the pane entirely (over the terminal, the tree, the titlebar).
                // A gesture must never outlive the mouse button.
                // …and so does the WebTabs rail's, for the same reason and by
                // the same route: the rail's own container takes a real drop
                // first, so this only catches a release that landed outside it.
                // ONE gesture for both, so ONE exit.
                if state.read().row_drag.is_some() {
                    state.with_mut_counted(|shell| shell.clear_app_pane_row_drag());
                }
                state.with_mut_counted(|shell| shell.finish_sidebar_resize());
                state.with_mut_counted(|shell| shell.finish_rail_resize());
            },
            onmousemove: move |evt| {
                let pointer = evt.client_coordinates();
                let primary_down = evt.held_buttons().contains(MouseButton::Primary);
                let drag_active = !state.read().drag_paths.is_empty();
                if drag_active
                    && state
                        .read()
                        .drag_pointer_update_needed((pointer.x, pointer.y))
                {
                    state.with_mut_counted(|shell| shell.update_drag_pointer((pointer.x, pointer.y)));
                }
                if !drag_active && primary_down && state.read().pending_tree_drag.is_some() {
                    state.with_mut_counted(|shell| {
                        shell.update_pending_tree_drag_pointer((pointer.x, pointer.y))
                    });
                }
                if state.read().sidebar_resize_drag.is_some() {
                    state.with_mut_counted(|shell| {
                        shell.handle_sidebar_resize_pointer(pointer.x, primary_down)
                    });
                }
                if state.read().rail_resize_drag.is_some() {
                    state.with_mut_counted(|shell| shell.handle_rail_resize_pointer(pointer.x));
                }
                // The row-drag ghost follows the pointer everywhere, not only
                // over the rows that own the gesture — the cwd tree's ghost has
                // always tracked from the window root, and a ghost that froze
                // at the list edge would read as a dropped drag.
                if primary_down && state.read().row_drag.is_some() {
                    state.with_mut_counted(|shell| {
                        shell.track_row_drag_pointer((pointer.x, pointer.y));
                    });
                }
            },
            onkeydown: move |evt| {
                // ESCAPE ABANDONS A LIVE DRAG, for every row list at once —
                // the way out of a gesture you did not mean to start. It runs
                // before the accelerators below because a drag in flight is a
                // modal state: nothing else should read this key.
                if evt.key() == Key::Escape && state.read().row_drag.is_some() {
                    evt.prevent_default();
                    state.with_mut_counted(|shell| shell.clear_app_pane_row_drag());
                    return;
                }
                // KeyTips chord walking now lives in the below-the-webview JS
                // bridge (keytip_apply_bridge_message), so it works from a
                // focused terminal too; this DOM handler only carries the
                // direct accelerators below.
                let is_accel = evt.modifiers().contains(Modifiers::CONTROL)
                    || evt.modifiers().contains(Modifiers::META);
                // Ctrl+Alt+PageUp/PageDown (session nav) is now a registered direct
                // accelerator handled by the below-the-webview bridge (§11.1), so it
                // fires from a focused terminal too and is no longer hardcoded here.
                if is_accel
                    && evt.modifiers().contains(Modifiers::SHIFT)
                    && matches!(evt.key(), Key::Character(ref key) if key.eq_ignore_ascii_case("p"))
                {
                    evt.prevent_default();
                    state.with_mut_counted(|shell| shell.set_search_focus(true));
                    focus_search_input(true);
                    return;
                }
                // Ctrl+F — find in page. Claimed HERE, at the layer the omnibox
                // claims keys, and NOT in `keytip::DEFAULT_ACCELERATORS`: a bare
                // Ctrl+<letter> belongs to the PTY (`assert_accels_pty_safe`),
                // so a global Ctrl+F would eat readline's forward-char in every
                // terminal in the app. It is only a browser key when a browser
                // owns the viewport, which is exactly what
                // `open_web_find_for_viewport` refuses on — and this handler is
                // only ONE of its two doors: the other is the GTK chord claimer
                // on a focused page webview, which cannot reach this listener at
                // all. Both call the same opener; neither owns the decision.
                if is_accel
                    && !evt.modifiers().contains(Modifiers::SHIFT)
                    && !evt.modifiers().contains(Modifiers::ALT)
                    && matches!(evt.key(), Key::Character(ref key) if key.eq_ignore_ascii_case("f"))
                    && open_web_find_for_viewport(state).is_some()
                {
                    evt.prevent_default();
                    return;
                }
                // The bare "/" search hotkey is GONE (user call 2026-07-23): a
                // plain printable key stole real typing whenever the
                // focus-judgment predicate misfired. Search focus lives in the
                // ALT+ layer (`search.focus`, ALT,S) and Ctrl+Shift+P.
                let preview_navigation_enabled = {
                    let shell = state.read();
                    shell.server.active_view_mode() == WorkspaceViewMode::Rendered && !shell.search_focused
                };
                if preview_navigation_enabled
                    && matches!(evt.key(), Key::PageUp | Key::PageDown | Key::Home | Key::End)
                {
                    evt.prevent_default();
                    let active_session_path = state
                        .read()
                        .server
                        .active_session_path()
                        .map(str::to_string);
                    let session_path_literal = serde_json::to_string(&active_session_path)
                        .unwrap_or_else(|_| "null".to_string());
                    let scroll_direction = if matches!(evt.key(), Key::PageUp) {
                        -1.0
                    } else {
                        1.0
                    };
                    let jump_to_edge =
                        is_accel || matches!(evt.key(), Key::Home | Key::End);
                    let jump_to_top = matches!(evt.key(), Key::Home)
                        || (is_accel && matches!(evt.key(), Key::PageUp));
                    let _ = document::eval(&format!(
                        "(function() {{
                            const activeSessionPath = {session_path_literal};
                            const pickLast = (nodes) => nodes.length ? nodes[nodes.length - 1] : null;
                            const visibleNodes = (nodes) => nodes.filter((node) => node.getClientRects().length > 0);
                            const previewScrolls = Array.from(document.querySelectorAll('[data-preview-scroll=\"1\"]'))
                              .filter((node) => node.isConnected);
                            const matchingPreviewScrolls = previewScrolls.filter((node) => {{
                                const path = node.getAttribute('data-preview-session-path');
                                return activeSessionPath && path === activeSessionPath;
                            }});
                            const activePreviewScrolls = previewScrolls.filter(
                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                            );
                            const activeMatchingPreviewScrolls = matchingPreviewScrolls.filter(
                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                            );
                            const scroller =
                                pickLast(visibleNodes(activeMatchingPreviewScrolls))
                                || pickLast(activeMatchingPreviewScrolls)
                                || pickLast(visibleNodes(matchingPreviewScrolls))
                                || pickLast(matchingPreviewScrolls)
                                || pickLast(visibleNodes(activePreviewScrolls))
                                || pickLast(activePreviewScrolls)
                                || pickLast(visibleNodes(previewScrolls))
                                || pickLast(previewScrolls)
                                || null;
                            if (!scroller) {{
                                return;
                            }}
                            if ({jump_to_edge}) {{
                                scroller.scrollTo({{
                                    top: {jump_to_top} ? 0 : scroller.scrollHeight,
                                    behavior: 'smooth',
                                }});
                                return;
                            }}
                            scroller.scrollBy({{
                                top: Math.round(scroller.clientHeight * 0.9 * {scroll_direction}),
                                behavior: 'smooth',
                            }});
                        }})();"
                    ));
                    return;
                }
                if evt.key() == Key::Escape {
                    if state.read().keymap_editor_open {
                        evt.prevent_default();
                        state.with_mut_counted(|shell| shell.close_keymap_editor());
                        return;
                    }
                    if state.read().pending_delete.is_some() {
                        evt.prevent_default();
                        state.with_mut_counted(|shell| shell.cancel_delete_dialog());
                        return;
                    }
                    let should_clear = {
                        let shell = state.read();
                        shell.search_focused || !shell.search_query.trim().is_empty()
                    };
                    if should_clear {
                        evt.prevent_default();
                        state.with_mut_counted(|shell| {
                            shell.set_search(String::new());
                            shell.set_search_focus(false);
                        });
                        let _ = document::eval(&format!(
                            "(function() {{
                                const input = document.getElementById({SEARCH_INPUT_ID:?});
                                if (input && input.blur) input.blur();
                            }})();"
                        ));
                        return;
                    }
                    if state.read().fullscreen {
                        evt.prevent_default();
                        state.with_mut_counted(|shell| shell.toggle_fullscreen());
                        return;
                    }
                }
                if !is_accel && matches!(evt.key(), Key::Character(ref key) if key == "[" || key == "]") {
                    let has_search = !state.read().search_query.trim().is_empty();
                    if has_search {
                        evt.prevent_default();
                        let step = if matches!(evt.key(), Key::Character(ref key) if key == "[") {
                            -1
                        } else {
                            1
                        };
                        if let Some(row) = state.with_mut_counted(|shell| shell.next_search_sidebar_row(step)) {
                            spawn_open_session_row(state, row);
                        }
                        return;
                    }
                }
                if evt.key() == Key::Delete && state.read().delete_shortcut_should_target_tree() {
                    evt.prevent_default();
                    queue_delete_selected_items(state, evt.modifiers().contains(Modifiers::SHIFT));
                    return;
                }
                let is_terminal_shortcut = is_accel
                    && evt.modifiers().contains(Modifiers::SHIFT)
                    && matches!(evt.key(), Key::Character(ref key) if key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x"));
                if is_terminal_shortcut && state.read().snapshot().active_view_mode != WorkspaceViewMode::Terminal {
                    let (title, message) = match evt.key() {
                        Key::Character(key) if key.eq_ignore_ascii_case("x") => (
                            "Cut to Clipboard",
                            "Moved the current UI selection to the clipboard.",
                        ),
                        _ => (
                            "Copied to Clipboard",
                            "Copied the current UI selection to the clipboard.",
                        ),
                    };
                    state.with_mut_counted(|shell| {
                        shell.push_notification(NotificationTone::Success, title, message);
                    });
                }
            },
            // The shell root suppresses the platform's own right-click menu so a
            // stray right-click on CHROME does not surface a webview menu over the
            // app. But a DOCUMENT (yedit) or WEB (ychrome) surface renders real
            // WebKit content whose native menu (Copy/Cut/Paste/Select-All) is the
            // right menu there — and this blanket preventDefault was killing it,
            // which is why yedit had no right-click copy at all. Stand down for
            // those surfaces and let the engine's menu through.
            //
            // ⚠ Third handler in this family to need the same lesson (after the
            // root ONCLICK focus-steal and the terminal secondary-button funnel):
            // a root-level handler that acts on every event must ask WHO owns the
            // area under the pointer. `DOCUMENT_SURFACE_MENU_OWNER_SELECTORS` is
            // the one list; add a surface there, never a fourth hand-rolled copy.
            // NOTE: the blanket `evt.prevent_default()` that used to live here is
            // gone — see context_menu_policy_script(). Rust cannot ask "what is under
            // the pointer" synchronously (document::eval is async), so the policy
            // has to be decided in JS, where the target element is in hand.
            onmounted: move |_evt| async move {
                let _ = document::eval(&context_menu_policy_script());
            },
            style { "{TOAST_CSS}" }
            // A top-centre toast clears the TITLEBAR instead of sitting on it.
            //
            // `ToastAnchor::TopCenter` is a flat `top:22px` from the window edge,
            // which is right when the titlebar is auto-hidden (nothing is up
            // there) and wrong when it is pinned: the chrome is
            // TITLEBAR_HEIGHT_PX tall, so the card lands against it with no
            // breathing room — the owner's report, 2026-08-07. The offset is
            // yggterm's to know, not the library's: libyggterm has no titlebar
            // and cannot answer "is chrome occupying the top edge".
            //
            // Scoped to the pinned case by the shell's own attribute, so the
            // auto-hide arm keeps the library's 22px unchanged.
            style {
                "[data-yggterm-titlebar-pinned=\"true\"] [data-yggui-toast-anchor=\"top_center\"] {{ \
                 top: {TOAST_TOP_CENTER_PINNED_OFFSET_PX}px !important; }}"
            }
            // The session-style row's reveal rule, declared ONCE for the whole
            // window: the cwdtree sidebar, the ychrome tab rail and every
            // contributed app pane all draw `data-session-row`s, and a rule
            // injected per rail body only reached the bodies that remembered to
            // inject it (the tab rail never did).
            style { "{session_row_hover_css}" }
            // The text field's hover / focus / placeholder rule, declared ONCE
            // for the whole window: the omnibox, the titlebar search, Settings,
            // and every contributed app pane draw the same control.
            style { "{text_field_css}" }
            style { "{DOCUMENT_SURFACE_STANDDOWN_CSS}" }
            style { "{WEB_UNDER_GLASS_CSS}" }
            style { "{MENU_SURFACE_CSS}" }
            style { "{FORM_DIALOG_FOCUS_CSS}" }
            // A rail body is reading material, so its text selects. Declared
            // here for the same reason as the two rules above: every rail —
            // metadata, settings, connect, notifications, the tab rail and every
            // contributed app pane — draws into `.yggui-rail-scroll`.
            style { "{RAIL_TEXT_SELECTION_CSS}" }
            style { "{WEB_SURFACE_VTAB_CSS}" }
            style { "{shell_document_css}" }
            // KeyTips chord breadcrumb: shows the leader + the chord typed so far
            // while the ALT+ overlay is up (Excel's "ALT, then H, then …" trail).
            if snapshot.alt_overlay_active {
                div {
                    "data-yggterm-keytip-breadcrumb": "1",
                    // The chord LEVEL, for the bridge's surface signature: when
                    // this changes the walk re-derives, which is what gives a
                    // panel/menu/dialog the chord descended into its letters.
                    // An attribute rather than the visible text because the text
                    // also carries jump mode's moving label.
                    "data-yggterm-keytip-sequence": format!(
                        "{}{}",
                        snapshot
                            .alt_overlay_modal_scope
                            .clone()
                            .map(|kind| format!("modal:{kind}/"))
                            .unwrap_or_default(),
                        snapshot.alt_overlay_sequence,
                    ),
                    // Sits INSIDE the titlebar's search field, vertically centred in
                    // it — at top:8px the pill hung off the titlebar's bottom border.
                    // The field's box is 3..29 and the pill is 22 tall, so 5 centres it.
                    style: format!(
                        "position:absolute; top:5px; left:50%; transform:translateX(-50%); z-index:400; \
                         display:flex; align-items:center; gap:8px; height:22px; padding:0 13px; border-radius:999px; \
                         background:rgba(95,168,255,0.16); box-shadow: inset 0 0 0 1px rgba(95,168,255,0.42); \
                         color:{}; font-size:11px; font-weight:800; letter-spacing:0.3px; pointer-events:none; \
                         white-space:nowrap;",
                        snapshot.palette.accent,
                    ),
                    span { "⌨ ALT" }
                    // A modal scope has no chord letters of its own — the trail
                    // names the dialog instead, so the breadcrumb never reads as
                    // a bare leader while a dialog owns the keys (§4).
                    if let Some(label) = snapshot.alt_overlay_modal_label.clone() {
                        span { style: "opacity:0.55;", "›" }
                        span { "{label}" }
                    }
                    if !snapshot.alt_overlay_sequence.is_empty() {
                        span { style: "opacity:0.55;", "›" }
                        span { "{snapshot.alt_overlay_sequence.to_uppercase()}" }
                    }
                    // Jump mode is a LIST scope with no badges (§8), so the
                    // breadcrumb IS its display: where the cursor is, and out of
                    // how many. The only feedback when the sidebar is closed.
                    if let Some((index, count, label)) = snapshot.session_jump_status.clone() {
                        span { style: "opacity:0.55;", "›" }
                        span { "Live {index}/{count}" }
                        span {
                            style: "font-weight:700; max-width:280px; overflow:hidden; text-overflow:ellipsis;",
                            "{label}"
                        }
                        span {
                            style: format!("opacity:0.62; font-weight:600; color:{};", snapshot.palette.muted),
                            "· ↑↓ PgUp/PgDn to move · Enter to open · Esc to exit"
                        }
                    } else {
                        span {
                            style: format!("opacity:0.62; font-weight:600; color:{};", snapshot.palette.muted),
                            "· press a highlighted key · Esc to exit"
                        }
                    }
                }
            }
            // The ALT+ keymap editor modal (Settings ▸ "Explore & edit KeyTips").
            // Another VIEW of the command registry: rebinds write through to
            // `~/.yggterm/keymap.json` and re-render every KeyTip badge.
            if snapshot.keymap_editor_open {
                {
                    let palette = snapshot.palette;
                    let dark = palette_is_dark(palette);
                    let keymap = snapshot.keymap.clone();
                    let error = snapshot.keymap_editor_error.clone();
                    // The direct-accelerator column (§11.5): command-id → its chord,
                    // from the EFFECTIVE set (shipping defaults + the user's
                    // keymap.json overrides). Rebindable — the second door to each
                    // command, shown beside its ALT chord.
                    let accels: std::collections::BTreeMap<String, String> =
                        keytip::effective_accelerators(&snapshot.keytip_config)
                            .into_iter()
                            .map(|(id, chord)| (id, chord.display()))
                            .collect();
                    rsx! {
                        div {
                            "data-keytips-editor-modal": "1",
                            // §4 walk root — see the delete overlay's stamp.
                            "data-yggterm-modal-root": "keymap-editor",
                            style: "position:absolute; inset:0; z-index:500; display:flex; align-items:center; \
                                    justify-content:center; background:rgba(6,10,14,0.5); backdrop-filter:blur(2px);",
                            onclick: move |_| state.with_mut_counted(|shell| shell.close_keymap_editor()),
                            div {
                                style: format!(
                                    "width:min(460px, 92vw); max-height:82vh; overflow:auto; display:flex; \
                                     flex-direction:column; gap:12px; padding:18px; border-radius:16px; background:{}; \
                                     box-shadow: 0 24px 60px rgba(0,0,0,0.32), inset 0 0 0 1px {};",
                                    if dark { "rgba(18,24,30,0.99)" } else { "rgba(252,253,255,0.99)" },
                                    if dark { "rgba(120,140,158,0.24)" } else { "rgba(198,212,224,0.7)" },
                                ),
                                onclick: move |evt| evt.stop_propagation(),
                                div {
                                    style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                                    div {
                                        style: "display:flex; flex-direction:column; gap:3px;",
                                        div {
                                            style: format!("font-size:15px; font-weight:800; color:{};", palette.text),
                                            "ALT+ KeyTips"
                                        }
                                        div {
                                            style: format!("font-size:11px; line-height:1.4; color:{};", palette.muted),
                                            "Two doors to every command: tap ALT then its letter (type a new letter to rebind), or its direct accelerator."
                                        }
                                    }
                                    button {
                                        style: format!(
                                            "border:none; background:transparent; color:{}; font-size:18px; \
                                             cursor:pointer; line-height:1; padding:2px 6px;",
                                            palette.muted
                                        ),
                                        onclick: move |_| state.with_mut_counted(|shell| shell.close_keymap_editor()),
                                        "✕"
                                    }
                                }
                                if let Some(message) = error {
                                    div {
                                        style: "font-size:11px; font-weight:700; color:#c0433a; padding:7px 10px; \
                                                border-radius:9px; background:rgba(214,90,80,0.12); \
                                                box-shadow: inset 0 0 0 1px rgba(214,90,80,0.32);",
                                        "{message}"
                                    }
                                }
                                div {
                                    style: "display:flex; flex-direction:column; gap:4px;",
                                    div {
                                        style: "display:flex; align-items:center; justify-content:space-between; gap:10px; padding:2px 10px 4px 10px;",
                                        span {
                                            style: format!("font-size:10px; font-weight:800; letter-spacing:0.4px; text-transform:uppercase; color:{};", palette.muted),
                                            "Command"
                                        }
                                        div {
                                            style: "display:flex; align-items:center; gap:10px;",
                                            span {
                                                style: format!("font-size:10px; font-weight:800; letter-spacing:0.4px; text-transform:uppercase; text-align:right; width:118px; color:{};", palette.muted),
                                                "ALT KeyTip"
                                            }
                                            span {
                                                style: format!("font-size:10px; font-weight:800; letter-spacing:0.4px; text-transform:uppercase; text-align:right; width:118px; color:{};", palette.muted),
                                                "Accelerator"
                                            }
                                        }
                                    }
                                    for spec in command_registry::SHELL_COMMANDS.iter() {
                                        {
                                            let id = spec.id;
                                            let chord = keymap.chord_for_id(id);
                                            let leaf = keymap.keytip_for_id(id);
                                            let accel = accels.get(id).cloned();
                                            let row_letter = leaf.map(|c| c.to_string()).unwrap_or_default();
                                            rsx! {
                                                div {
                                                    // Key includes the letter so a successful rebind REBUILDS the
                                                    // row — its uncontrolled `initial_value` input then shows the
                                                    // new letter without a controlled `value:` fighting the typing.
                                                    key: "keytip-row-{id}-{row_letter}",
                                                    style: format!(
                                                        "display:flex; align-items:center; justify-content:space-between; gap:10px; \
                                                         padding:7px 10px; border-radius:10px; background:{};",
                                                        if dark { "rgba(255,255,255,0.03)" } else { "rgba(120,140,160,0.05)" },
                                                    ),
                                                    div {
                                                        style: "display:flex; flex-direction:column; gap:1px; min-width:0;",
                                                        span {
                                                            style: format!("font-size:12px; font-weight:700; color:{};", palette.text),
                                                            "{spec.title}"
                                                        }
                                                        span {
                                                            style: format!("font-size:10px; color:{}; font-family:monospace;", palette.muted),
                                                            "{id}"
                                                        }
                                                    }
                                                    div {
                                                        // Two columns: the ALT chord (rebindable) and the direct
                                                        // accelerator (read-only). Fixed widths so the columns line
                                                        // up down the list like a real keymap table.
                                                        style: "display:flex; align-items:center; gap:10px;",
                                                        div {
                                                            style: "display:flex; align-items:center; justify-content:flex-end; gap:6px; width:118px;",
                                                            if let Some(letter) = leaf {
                                                                if let Some(prefix) = chord.as_ref().and_then(|c| c.get(..c.len().saturating_sub(1))).filter(|p| !p.is_empty()) {
                                                                    span {
                                                                        style: format!("font-size:10px; color:{}; font-weight:700;", palette.muted),
                                                                        "ALT {prefix.to_uppercase()} ›"
                                                                    }
                                                                } else {
                                                                    span {
                                                                        style: format!("font-size:10px; color:{}; font-weight:700;", palette.muted),
                                                                        "ALT ›"
                                                                    }
                                                                }
                                                                input {
                                                                    r#type: "text",
                                                                    initial_value: "{letter.to_ascii_uppercase()}",
                                                                    style: format!(
                                                                        "width:34px; height:30px; text-align:center; text-transform:uppercase; \
                                                                         font-size:14px; font-weight:800; border-radius:8px; border:none; \
                                                                         background:rgba(95,168,255,0.14); color:{}; \
                                                                         box-shadow: inset 0 0 0 1px rgba(95,168,255,0.42);",
                                                                        palette.accent
                                                                    ),
                                                                    onclick: move |evt| evt.stop_propagation(),
                                                                    // Select the existing letter on focus so a keystroke
                                                                    // REPLACES it (the field holds one letter at a time).
                                                                    onfocus: move |_| { let _ = document::eval("if(document.activeElement&&document.activeElement.select)document.activeElement.select();"); },
                                                                    oninput: move |evt| {
                                                                        if let Some(ch) = evt.value().chars().rev().find(|c| c.is_ascii_alphanumeric()) {
                                                                            state.with_mut_counted(|shell| shell.set_keymap_override(id, ch));
                                                                        }
                                                                    },
                                                                }
                                                            } else {
                                                                span {
                                                                    style: format!("font-size:11px; color:{};", palette.muted),
                                                                    "—"
                                                                }
                                                            }
                                                        }
                                                        div {
                                                            "data-keytips-accel": "{id}",
                                                            style: "display:flex; align-items:center; justify-content:flex-end; width:118px;",
                                                            input {
                                                                // Uncontrolled + keyed by the current chord: a successful
                                                                // rebind rebuilds the row so the field shows the new
                                                                // value, and a controlled `value:` never fights typing.
                                                                key: "accel-{id}-{accel.clone().unwrap_or_default()}",
                                                                r#type: "text",
                                                                initial_value: accel.clone().unwrap_or_default(),
                                                                placeholder: "—",
                                                                title: "Type a PTY-safe chord (Ctrl+Shift+T, Ctrl+Alt+PageDown, F11). Empty resets to the default.",
                                                                style: format!(
                                                                    "width:112px; height:26px; text-align:right; font-size:10px; font-weight:800; \
                                                                     font-family:monospace; border:none; border-radius:7px; padding:0 8px; \
                                                                     background:rgba(120,140,160,0.12); color:{}; outline:none; \
                                                                     box-shadow: inset 0 0 0 1px {};",
                                                                    palette.text, chrome_chip_border(palette),
                                                                ),
                                                                onclick: move |evt| evt.stop_propagation(),
                                                                // The field is typed a character at a time, so only a
                                                                // COMPLETE chord commits: a partial spec ("Ctrl+", "Ctrl+S")
                                                                // is left alone rather than rejected mid-typing. An
                                                                // emptied field clears the override back to the default.
                                                                oninput: move |evt| {
                                                                    let value = evt.value();
                                                                    let trimmed = value.trim().to_string();
                                                                    let complete = trimmed.is_empty()
                                                                        || (!trimmed.ends_with('+')
                                                                            && Chord::parse(&trimmed)
                                                                                .is_some_and(|chord| chord.is_pty_safe()));
                                                                    if complete {
                                                                        state.with_mut_counted(|shell| {
                                                                            shell.set_accel_override(id, &trimmed)
                                                                        });
                                                                    }
                                                                },
                                                                onkeydown: move |evt| evt.stop_propagation(),
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                div {
                                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px; margin-top:2px;",
                                    span {
                                        style: format!("font-size:10px; color:{};", palette.muted),
                                        "Saved to ~/.yggterm/keymap.json"
                                    }
                                    button {
                                        "data-keytips-reset-button": "1",
                                        style: format!(
                                            "border:none; border-radius:9px; padding:7px 14px; font-size:11px; font-weight:800; \
                                             cursor:pointer; background:{}; color:{}; box-shadow: inset 0 0 0 1px {};",
                                            if dark { "rgba(21,28,35,0.94)" } else { "rgba(255,255,255,0.86)" },
                                            palette.text,
                                            if dark { "rgba(93,116,134,0.56)" } else { "rgba(208,219,229,0.85)" },
                                        ),
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            state.with_mut_counted(|shell| shell.reset_keymap());
                                        },
                                        "Reset to Excel preset"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            button {
                id: TREE_DELETE_BUTTON_ID,
                tabindex: "-1",
                aria_hidden: "true",
                style: "position:absolute; width:1px; height:1px; opacity:0; pointer-events:none; overflow:hidden;",
                onclick: move |_| queue_delete_selected_items(state, false),
            }
            button {
                id: TREE_HARD_DELETE_BUTTON_ID,
                tabindex: "-1",
                aria_hidden: "true",
                style: "position:absolute; width:1px; height:1px; opacity:0; pointer-events:none; overflow:hidden;",
                onclick: move |_| queue_delete_selected_items(state, true),
            }
            div {
                "data-yggterm-shell": "1",
                "data-yggterm-app-control-backgrounded": if app_control_backgrounded { "true" } else { "false" },
                "data-yggterm-window-focused": if window_focused { "true" } else { "false" },
                style: shell_style(
                    snapshot.palette,
                    effective_shell_radius,
                    &snapshot.shell_tint,
                    &snapshot.chrome_material_tint,
                    &snapshot.shell_gradient,
                    &snapshot.shell_gradient_background_size,
                    &snapshot.shell_gradient_background_repeat,
                    snapshot.shell_material_blur_px,
                    maximized,
                    linux_transparent_window,
                ),
                // The ONE app-background owner for the under-glass hole
                // treatment (see WEB_UNDER_GLASS_CSS): replicates the frame's
                // paint beneath every sibling; only under glass does it become
                // the visible background — with clip-path holes over pages.
                div {
                    "data-yggterm-app-bg": "1",
                    style: shell_background_layer_style(
                        snapshot.palette,
                        effective_shell_radius,
                        &snapshot.shell_tint,
                        &snapshot.chrome_material_tint,
                        &snapshot.shell_gradient,
                        &snapshot.shell_gradient_background_size,
                        &snapshot.shell_gradient_background_repeat,
                        maximized,
                        linux_transparent_window,
                    ),
                }
                if !chrome_hidden {
                    WindowResizeHandles {}
                }
                if !chrome_hidden {
                    div {
                        "data-yggterm-titlebar": "1",
                        "data-titlebar-auto-hide-enabled": if titlebar_auto_hide_enabled { "true" } else { "false" },
                        "data-titlebar-revealed": if titlebar_revealed { "true" } else { "false" },
                        // Is the reveal held by a GESTURE (a titlebar menu, a
                        // focused search field, KeyTips) rather than by the
                        // pointer resting on the edge? A pinned reveal is going
                        // to stand, so a page beside it may RESIZE; a transient
                        // hover only ever TRANSLATES the page
                        // (`web_surface_place_page_rect`).
                        "data-titlebar-autohide-pin": if titlebar_reveal_pinned { "1" } else { "0" },
                        "data-titlebar-hover-active": if titlebar_autohide_hovered() { "true" } else { "false" },
                        // F.1: a REVEALED auto-hide titlebar floats over the
                        // page hole, so it declares itself a cover — the
                        // synchronous cover push routes its rect back to the
                        // shell's input region (ResizeObserver tracks the
                        // reveal transition). Collapsed, the 6px sensor stays
                        // UNdeclared: the page owns its top edge and the
                        // reveal comes from the host motion observer (the
                        // reserved-strip alternative was rejected).
                        "data-covers-web-surface": if titlebar_auto_hide_enabled && titlebar_revealed { "titlebar" },
                        style: titlebar_wrapper_style(
                            titlebar_auto_hide_enabled,
                            titlebar_revealed,
                            snapshot.palette,
                            linux_compositor_blur_active(),
                            snapshot.shell_material_blur_px,
                        ),
                        onclick: |evt| evt.stop_propagation(),
                        oncontextmenu: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                        // The whole top edge reveals, over every viewport — the
                        // titlebar is yggterm's chrome and un-hiding it is an
                        // explicit user intent. A libyggterm app's viewport is not
                        // special: whoever reaches for the top edge wants the
                        // titlebar, and whoever wants the app's tab bar simply does
                        // not go there. (ychrome's page area used to be exempt,
                        // which made the titlebar unreachable over it and produced
                        // a reveal that depended on where along the edge you
                        // entered — user-rejected 2026-07-10.)
                        onmouseenter: move |_| {
                            if titlebar_auto_hide_enabled {
                                autohide_reveal(
                                    titlebar_autohide_hovered,
                                    titlebar_autohide_lingering,
                                    titlebar_autohide_linger_generation,
                                );
                            }
                        },
                        // `mouseenter` fires ONCE, at the point of entry. If the
                        // collapsing titlebar shrinks out from under a resting
                        // pointer, no further enter ever arrives — so a move inside
                        // the sensor re-reveals. Early-returns once revealed, so a
                        // revealed titlebar costs no signal writes as the pointer
                        // crosses it.
                        onmousemove: move |_| {
                            if titlebar_auto_hide_enabled && !titlebar_autohide_hovered() {
                                autohide_reveal(
                                    titlebar_autohide_hovered,
                                    titlebar_autohide_lingering,
                                    titlebar_autohide_linger_generation,
                                );
                            }
                        },
                        onmouseleave: move |_| {
                            if !titlebar_auto_hide_enabled {
                                return;
                            }
                            autohide_handle_mouse_leave(
                                titlebar_autohide_hovered,
                                titlebar_autohide_lingering,
                                titlebar_autohide_linger_generation,
                            );
                        },
                        div {
                            style: titlebar_inner_style(titlebar_auto_hide_enabled, titlebar_revealed),
                            Titlebar {
                                snapshot: titlebar_snapshot,
                                hovered: hovered,
                                on_toggle_sidebar: move || state.with_mut_counted(|shell| shell.toggle_sidebar()),
                                on_search: move |value: String| state.with_mut_counted(|shell| shell.set_search_from_input(value)),
                                on_clear_search: move |_| {
                                    state.with_mut_counted(|shell| {
                                        shell.set_search(String::new());
                                        shell.set_search_focus(false);
                                    });
                                    let _ = document::eval(&format!(
                                        "(function() {{
                                            const input = document.getElementById({SEARCH_INPUT_ID:?});
                                            if (input && input.blur) input.blur();
                                        }})();"
                                    ));
                                },
                                on_execute_search_command: move |command: String| execute_search_command(state, command),
                                on_set_search_focus: move |focused: bool| {
                                    state.with_mut_counted(|shell| shell.set_search_focus(focused));
                                    sync_active_terminal_input_policy(state);
                                    if focused {
                                        focus_search_input(false);
                                    } else {
                                        reclaim_active_terminal_input_after_search_blur(state);
                                    }
                                },
                                on_prev_search_content: move |_| {
                            if let Some(dom_id) = state.with_mut_counted(|shell| shell.next_search_content_dom_id(-1)) {
                                if let Some(line_index) = dom_id.strip_prefix("__terminal_line__:") {
                                    let session_path = state
                                        .read()
                                        .server
                                        .active_session()
                                        .map(|session| session.session_path.clone());
                                    if let (Some(session_path), Ok(line_index)) = (session_path, line_index.parse::<usize>()) {
                                        let _ = document::eval(&terminal_scroll_to_line_script(
                                            &session_path,
                                            line_index,
                                        ));
                                    }
                                } else if dom_id == PREVIEW_HEADER_SEARCH_HIT_ID {
                                    let active_session_path = state
                                        .read()
                                        .server
                                        .active_session_path()
                                        .map(str::to_string);
                                    let session_path_literal = serde_json::to_string(&active_session_path)
                                        .unwrap_or_else(|_| "null".to_string());
                                    let _ = document::eval(&format!(
                                        "(function() {{
                                            const activeSessionPath = {session_path_literal};
                                            const pickLast = (nodes) => nodes.length ? nodes[nodes.length - 1] : null;
                                            const visibleNodes = (nodes) => nodes.filter((node) => node.getClientRects().length > 0);
                                            const previewScrolls = Array.from(document.querySelectorAll('[data-preview-scroll=\"1\"]'))
                                              .filter((node) => node.isConnected);
                                            const matchingPreviewScrolls = previewScrolls.filter((node) => {{
                                                const path = node.getAttribute('data-preview-session-path');
                                                return activeSessionPath && path === activeSessionPath;
                                            }});
                                            const activePreviewScrolls = previewScrolls.filter(
                                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                                            );
                                            const activeMatchingPreviewScrolls = matchingPreviewScrolls.filter(
                                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                                            );
                                            const scroller =
                                                pickLast(visibleNodes(activeMatchingPreviewScrolls))
                                                || pickLast(activeMatchingPreviewScrolls)
                                                || pickLast(visibleNodes(matchingPreviewScrolls))
                                                || pickLast(matchingPreviewScrolls)
                                                || pickLast(visibleNodes(activePreviewScrolls))
                                                || pickLast(activePreviewScrolls)
                                                || pickLast(visibleNodes(previewScrolls))
                                                || pickLast(previewScrolls)
                                                || null;
                                            if (scroller) {{
                                                scroller.scrollTo({{ top: 0, behavior: 'smooth' }});
                                            }}
                                        }})();"
                                    ));
                                } else {
                                    let _ = document::eval(&format!(
                                        "(function() {{
                                            const el = document.getElementById({dom_id:?});
                                            if (el) {{
                                                el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'smooth' }});
                                            }}
                                        }})();"
                                    ));
                                }
                                }
                            },
                            on_next_search_content: move |_| {
                            if let Some(dom_id) = state.with_mut_counted(|shell| shell.next_search_content_dom_id(1)) {
                                if let Some(line_index) = dom_id.strip_prefix("__terminal_line__:") {
                                    let session_path = state
                                        .read()
                                        .server
                                        .active_session()
                                        .map(|session| session.session_path.clone());
                                    if let (Some(session_path), Ok(line_index)) = (session_path, line_index.parse::<usize>()) {
                                        let _ = document::eval(&terminal_scroll_to_line_script(
                                            &session_path,
                                            line_index,
                                        ));
                                    }
                                } else if dom_id == PREVIEW_HEADER_SEARCH_HIT_ID {
                                    let active_session_path = state
                                        .read()
                                        .server
                                        .active_session_path()
                                        .map(str::to_string);
                                    let session_path_literal = serde_json::to_string(&active_session_path)
                                        .unwrap_or_else(|_| "null".to_string());
                                    let _ = document::eval(&format!(
                                        "(function() {{
                                            const activeSessionPath = {session_path_literal};
                                            const pickLast = (nodes) => nodes.length ? nodes[nodes.length - 1] : null;
                                            const visibleNodes = (nodes) => nodes.filter((node) => node.getClientRects().length > 0);
                                            const previewScrolls = Array.from(document.querySelectorAll('[data-preview-scroll=\"1\"]'))
                                              .filter((node) => node.isConnected);
                                            const matchingPreviewScrolls = previewScrolls.filter((node) => {{
                                                const path = node.getAttribute('data-preview-session-path');
                                                return activeSessionPath && path === activeSessionPath;
                                            }});
                                            const activePreviewScrolls = previewScrolls.filter(
                                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                                            );
                                            const activeMatchingPreviewScrolls = matchingPreviewScrolls.filter(
                                                (node) => node.getAttribute('data-preview-scroll-active') === 'true'
                                            );
                                            const scroller =
                                                pickLast(visibleNodes(activeMatchingPreviewScrolls))
                                                || pickLast(activeMatchingPreviewScrolls)
                                                || pickLast(visibleNodes(matchingPreviewScrolls))
                                                || pickLast(matchingPreviewScrolls)
                                                || pickLast(visibleNodes(activePreviewScrolls))
                                                || pickLast(activePreviewScrolls)
                                                || pickLast(visibleNodes(previewScrolls))
                                                || pickLast(previewScrolls)
                                                || null;
                                            if (scroller) {{
                                                scroller.scrollTo({{ top: 0, behavior: 'smooth' }});
                                            }}
                                        }})();"
                                    ));
                                } else {
                                    let _ = document::eval(&format!(
                                        "(function() {{
                                            const el = document.getElementById({dom_id:?});
                                            if (el) {{
                                                el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'smooth' }});
                                            }}
                                        }})();"
                                    ));
                                }
                                }
                            },
                            on_hover_control: move |control: Option<HoveredControl>| hovered.set(control),
                            on_set_view_mode: move |mode: WorkspaceViewMode| spawn_set_view_mode(state, mode),
                            on_set_document_surface_visible: move |(path, visible): (String, bool)| {
                                state.with_mut_counted(|shell| {
                                    // `document_surface_hidden` is the SSOT for the
                                    // user's toggle; the pane's own `visible` is a
                                    // derivation of it (`document_surface_visible_for`).
                                    if visible {
                                        shell.document_surface_hidden.remove(&path);
                                    } else {
                                        shell.document_surface_hidden.insert(path);
                                    }
                                });
                            },
                            on_document_action: move |(path, pane_id, action, value): (String, String, String, Option<String>)| {
                                spawn(document_pane_run_action(
                                    state,
                                    path,
                                    pane_id,
                                    action,
                                    value,
                                ));
                            },
                            on_toggle_session_menu: move |_| state.with_mut_counted(|shell| shell.toggle_titlebar_session_menu()),
                            on_toggle_new_menu: move |_| state.with_mut_counted(|shell| shell.toggle_titlebar_new_menu()),
                            on_toggle_overflow_menu: move |_| state.with_mut_counted(|shell| shell.toggle_titlebar_overflow_menu()),
                            on_close_overflow_menu: move |_| state.with_mut_counted(|shell| shell.close_titlebar_overflow_menu()),
                            on_start_claude_code: move |_| {
                                if !state.with(|shell| shell.titlebar_new_menu_open) {
                                    suppress_phantom_start_action(
                                        "titlebar_new_claude_code",
                                        json!({}),
                                    );
                                    return;
                                }
                                spawn_start_agent_session_for_row(state, None, SessionKind::ClaudeCode);
                            },
                            // The `+` menu launches at "here" — exactly like the row
                            // menu's "… Here" items, through the same anchored path
                            // (`spawn_start_session_for_row`), so the new row lands
                            // below the active/selected row instead of at the top of
                            // Live Sessions.
                            on_start_session: move |_| {
                                if !state.with(|shell| shell.titlebar_new_menu_open) {
                                    suppress_phantom_start_action("titlebar_new_session", json!({}));
                                    return;
                                }
                                spawn_start_preferred_agent_session_for_row(state, None);
                            },
                            on_start_terminal: move |_| {
                                if !state.with(|shell| shell.titlebar_new_menu_open) {
                                    suppress_phantom_start_action("titlebar_new_terminal", json!({}));
                                    return;
                                }
                                spawn_start_terminal_session_for_row(state, None);
                            },
                            on_launch_app_verb: move |(app, verb): (AppManifest, AppVerb)| {
                                spawn_launch_app_verb_here(state, app, verb, None);
                            },
                            on_refresh_summary: move |_| {
                            let active_session = { state.read().server.active_session().cloned() };
                            if let Some(session) = active_session {
                                state.with_mut_counted(|shell| {
                                    shell.last_action = "queued copy regeneration for active session".to_string();
                                    shell.push_notification(
                                        NotificationTone::Info,
                                        "Regenerating Session Copy",
                                        "Queued title, precis, and summary regeneration for the active session.".to_string(),
                                    );
                                });
                                queue_active_session_title_generation(state, true);
                                spawn_precis_generation(state, session.clone(), true);
                                spawn_summary_generation(state, session, true, true);
                            }
                            },
                            on_begin_active_rename: move |_| {
                            let renamed = state.with_mut_counted(|shell| shell.begin_active_titlebar_rename());
                            if renamed {
                                sync_active_terminal_input_policy(state);
                            }
                            },
                            on_edit_active_summary: move |_| {
                                queue_copy_edit_for_active_session(state, CopyEditField::Summary);
                            },
                            on_toggle_meta: move || {
                            state.with_mut_counted(|shell| shell.toggle_metadata_panel());
                            sync_active_terminal_input_policy(state);
                            },
                            on_toggle_settings: move || {
                            state.with_mut_counted(|shell| shell.toggle_settings_panel());
                            sync_active_terminal_input_policy(state);
                            },
                            on_toggle_connect: move || {
                            state.with_mut_counted(|shell| shell.toggle_connect_panel());
                            sync_active_terminal_input_policy(state);
                            },
                            on_toggle_notifications: move || {
                            state.with_mut_counted(|shell| shell.toggle_notifications_panel());
                            sync_active_terminal_input_policy(state);
                            },
                            on_toggle_app_pane: move |pane_id: String| {
                            // Opening a contributed pane fetches its schema from
                            // the app's control endpoint; closing it needs no
                            // round trip.
                            let opened = state.with_mut_counted(|shell| shell.toggle_app_pane(&pane_id));
                            sync_active_terminal_input_policy(state);
                            if let Some((open_pane, seq)) = opened {
                                spawn(app_pane_fetch_schema(
                                    state,
                                    open_pane.session,
                                    open_pane.pane,
                                    seq,
                                ));
                            }
                            },
                            on_toggle_web_tabs: move || {
                            state.with_mut_counted(|shell| shell.toggle_web_tabs_panel());
                            sync_active_terminal_input_policy(state);
                            },
                            on_restart_update: move || restart_into_pending_update(state),
                            on_request_window_drag: move || {
                            state.with_mut_counted(|shell| shell.note_titlebar_drag_request());
                            },
                            on_toggle_maximized: move || state.with_mut_counted(|shell| {
                            shell.note_titlebar_maximize_toggle_request();
                            shell.toggle_maximized();
                            }),
                            on_toggle_fullscreen: move || state.with_mut_counted(|shell| shell.toggle_fullscreen()),
                                on_toggle_always_on_top: move || state.with_mut_counted(|shell| shell.toggle_always_on_top()),
                                on_close_app: move || spawn_graceful_shutdown_and_close(state),
                                maximized: maximized,
                                fullscreen: fullscreen,
                            }
                        }
                    }
                }
                if snapshot.sidebar_resizing {
                    div {
                        "data-sidebar-resize-overlay": "1",
                        style: "position:absolute; inset:0; z-index:260; background:transparent; cursor:ew-resize;",
                        onmousedown: |evt| evt.stop_propagation(),
                        onmousemove: move |evt| {
                            let pointer = evt.client_coordinates();
                            let primary_down = evt.held_buttons().contains(MouseButton::Primary);
                            state.with_mut_counted(|shell| {
                                shell.handle_sidebar_resize_pointer(pointer.x, primary_down);
                            });
                        },
                        onmouseup: move |_| {
                            state.with_mut_counted(|shell| shell.finish_sidebar_resize());
                        },
                    }
                }
                // The twin for the right rail: a full-window pointer trap so the
                // drag keeps tracking over the terminal/webview, which would
                // otherwise swallow the mousemove.
                if snapshot.rail_resizing {
                    div {
                        "data-rail-resize-overlay": "1",
                        style: "position:absolute; inset:0; z-index:260; background:transparent; cursor:ew-resize;",
                        onmousedown: |evt| evt.stop_propagation(),
                        onmousemove: move |evt| {
                            let pointer = evt.client_coordinates();
                            state.with_mut_counted(|shell| shell.handle_rail_resize_pointer(pointer.x));
                        },
                        onmouseup: move |_| {
                            state.with_mut_counted(|shell| shell.finish_rail_resize());
                        },
                    }
                }
                if distraction_free_exit_visible {
                    div {
                        // ⚠ THE ONLY MOUSE ROUTE OUT OF DISTRACTION-FREE MODE.
                        // In this mode the titlebar and sidebar are not
                        // rendered at all, so this floating strip carries the
                        // exit-fullscreen control and nothing else does. Which
                        // is why it is gated on `distraction_free_exit_visible`
                        // and not on `chrome_hidden`: hiding it whenever chrome
                        // hides would delete the exit from the very mode it
                        // exits. It DOES stand down under a fullscreen page —
                        // that page owns every pixel and the engine owns the way
                        // out of it (Escape) — and comes back untouched after.
                        //
                        // It therefore MUST declare itself a cover. Under glass
                        // (the standard presentation path) the input region is
                        // the window MINUS the page holes PLUS the declared
                        // covers, so chrome that floats over a page and does not
                        // declare gets no input at all: the button is visible,
                        // the click goes through it to the page, and — with the
                        // keyboard route deaf too — the user's only way out is
                        // to kill the app. That is what happened (2026-07-31).
                        "data-covers-web-surface": "fullscreen-window-controls",
                        style: "position:absolute; top:12px; right:14px; z-index:180;",
                        onmousedown: |evt| evt.stop_propagation(),
                        onclick: |evt| evt.stop_propagation(),
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
                            on_hover_control: move |control: Option<HoveredControl>| hovered.set(control),
                            on_toggle_maximized: move || state.with_mut_counted(|shell| shell.toggle_maximized()),
                            on_toggle_fullscreen: move || state.with_mut_counted(|shell| shell.toggle_fullscreen()),
                            on_toggle_always_on_top: move || state.with_mut_counted(|shell| shell.toggle_always_on_top()),
                            on_close_app: move || spawn_graceful_shutdown_and_close(state),
                            maximized: maximized,
                            fullscreen: fullscreen,
                            always_on_top: snapshot.always_on_top,
                            show_always_on_top_button: true,
                            show_fullscreen_button: true,
                            show_window_buttons: true,
                            overlay: true,
                        }
                    }
                }
                div {
                    "data-yggterm-workspace-row": "1",
                    "data-titlebar-auto-hide-enabled": if titlebar_auto_hide_enabled { "true" } else { "false" },
                    "data-titlebar-revealed": if titlebar_revealed { "true" } else { "false" },
                    "data-chrome-mirrored": if chrome_orientation.is_mirrored() { "true" } else { "false" },
                    style: titlebar_autohide_content_offset_style(
                        titlebar_auto_hide_enabled,
                        titlebar_revealed,
                        chrome_orientation,
                    ),
                    if !chrome_hidden {
                        Sidebar {
                        snapshot: sidebar_snapshot,
                        autohide: left_sidebar_autohide,
                        autohide_revealed: left_sidebar_revealed,
                        autohide_pinned: left_sidebar_pinned,
                        on_prev_search_row: sidebar_on_prev_search_row,
                        on_next_search_row: sidebar_on_next_search_row,
                        on_select_all_rows: sidebar_on_select_all_rows,
                        on_navigate_rows: sidebar_on_navigate_rows,
                        on_start_sidebar_resize: sidebar_on_start_sidebar_resize,
                        on_focus_split_pane: sidebar_on_focus_split_pane,
                        on_select_row: sidebar_on_select_row,
                        on_press_highlight_row: sidebar_on_press_highlight_row,
                        on_set_row_expanded: sidebar_on_set_row_expanded,
                        on_delete_selected_items: sidebar_on_delete_selected_items,
                        on_delete_row: sidebar_on_delete_row,
                        on_open_context_menu: sidebar_on_open_context_menu,
                        on_start_drag: sidebar_on_start_drag,
                        on_drag_hover: sidebar_on_drag_hover,
                        on_drag_move: sidebar_on_drag_move,
                        on_drag_leave: sidebar_on_drag_leave,
                        on_drop_into_row: sidebar_on_drop_into_row,
                        on_end_drag: sidebar_on_end_drag,
                        on_begin_rename: sidebar_on_begin_rename,
                        on_regenerate_row_title: sidebar_on_regenerate_row_title,
                        on_update_rename: sidebar_on_update_rename,
                        on_focus_rename: sidebar_on_focus_rename,
                        on_commit_rename: sidebar_on_commit_rename,
                        on_cancel_rename: sidebar_on_cancel_rename,
                        rename_depth: tree_rename_depth,
                    }
                    }
                    div {
                        "data-yggterm-main-surface": "1",
                        style: "display:flex; flex:1; min-width:0; min-height:0;",
                        onmousedown: move |_| {
                            reclaim_active_terminal_input_from_viewport_click(state);
                        },
                        MainSurface {
                            state,
                            snapshot: main_snapshot,
                            on_expand_preview: move || spawn_surface_snapshot_action(
                            state,
                            "expanding preview".to_string(),
                            YggRequestMeta::interactive(
                                format!("preview-expand-{}", current_millis()),
                                "expand_preview",
                                YggSurface::Preview,
                                YggTarget::ActiveSession,
                            ),
                            false,
                            |endpoint| set_all_preview_blocks_folded(&endpoint, false),
                        ),
                        on_collapse_preview: move || spawn_surface_snapshot_action(
                            state,
                            "collapsing preview".to_string(),
                            YggRequestMeta::interactive(
                                format!("preview-collapse-{}", current_millis()),
                                "collapse_preview",
                                YggSurface::Preview,
                                YggTarget::ActiveSession,
                            ),
                            false,
                            |endpoint| set_all_preview_blocks_folded(&endpoint, true),
                        ),
                        on_toggle_preview_block: move |ix: usize| {
                            state.with_mut_counted(|shell| {
                                shell.server.toggle_preview_block(ix);
                            });
                            spawn_surface_snapshot_action(
                                state,
                                format!("toggling preview block {}", ix + 1),
                                YggRequestMeta::interactive(
                                    format!("preview-toggle-{}-{}", ix, current_millis()),
                                    "toggle_preview_block",
                                    YggSurface::Preview,
                                    YggTarget::ActiveSession,
                                ),
                                false,
                                move |endpoint| daemon_toggle_preview_block(&endpoint, ix),
                            )
                        },
                        on_set_preview_layout: move |mode: PreviewLayoutMode| state.with_mut_counted(|shell| shell.set_preview_layout(mode)),
                        on_save_document: move |(path, input): (String, WorkspaceDocumentInput)| {
                            queue_document_save(state, path, input, AfterSaveAction::SaveOnly)
                        },
                            on_run_recipe_document: move |(path, input, run_new_session): (String, WorkspaceDocumentInput, bool)| {
                            queue_document_save(
                                state,
                                path,
                                input,
                                if run_new_session {
                                    AfterSaveAction::RunNewSession
                                } else {
                                    AfterSaveAction::RunHere
                                },
                            )
                            },
                        }
                    }
                    if !chrome_hidden {
                        RightRail {
                            snapshot: metadata_snapshot,
                            autohide: right_rail_autohide,
                            autohide_revealed: right_rail_revealed,
                            autohide_pinned: right_rail_pinned,
                            on_start_rail_resize: move |client_x: f64| {
                                state.with_mut_counted(|shell| shell.start_rail_resize(client_x))
                            },
                            state,
                            on_endpoint_change: move |value: String| state.with_mut_counted(|shell| shell.update_litellm_endpoint(value)),
                            on_api_key_change: move |value: String| state.with_mut_counted(|shell| shell.update_litellm_api_key(value)),
                            on_model_change: move |value: String| state.with_mut_counted(|shell| shell.update_interface_llm_model(value)),
                            on_open_launch_flags: move |_| state.with_mut_counted(|shell| shell.set_launch_flags_open(true)),
                            on_open_cli_install: move |_| state.with_mut_counted(|shell| shell.set_cli_install_open(true)),
                            on_focus_input: move |field_key: String| {
                                focus_settings_field(state, &field_key);
                            },
                            on_blur_input: move |_| {
                                reclaim_active_terminal_input_after_settings_blur(state);
                            },
                            on_set_ui_theme: move |theme: UiTheme| {
                                state.with_mut_counted(|shell| shell.set_ui_theme(theme));
                                apply_active_terminal_zoom(state);
                            },
                            on_open_theme_editor: move |_| state.with_mut_counted(|shell| shell.open_theme_editor()),
                            on_open_keymap_editor: move |_| state.with_mut_counted(|shell| shell.open_keymap_editor()),
                            on_set_notification_delivery: move |mode: NotificationDeliveryMode| {
                                state.with_mut_counted(|shell| shell.update_notification_delivery(mode))
                            },
                            on_set_notification_sound: move |enabled: bool| {
                                state.with_mut_counted(|shell| shell.update_notification_sound(enabled))
                            },
                            on_set_terminal_telemetry: move |enabled: bool| {
                                state.with_mut_counted(|shell| shell.update_terminal_telemetry_enabled(enabled))
                            },
                            on_set_perf_profiling: move |enabled: bool| {
                                state.with_mut_counted(|shell| shell.update_perf_profiling_enabled(enabled))
                            },
                            on_set_titlebar_auto_hide: move |enabled: bool| {
                                state.with_mut_counted(|shell| shell.set_titlebar_auto_hide(enabled));
                                if !enabled {
                                    titlebar_autohide_hovered.set(false);
                                }
                            },
                            on_set_chrome_mirrored: move |mirrored: bool| {
                                state.with_mut_counted(|shell| shell.set_chrome_mirrored(mirrored));
                            },
                            on_adjust_ui_zoom: move |delta: i32| state.with_mut_counted(|shell| shell.adjust_ui_zoom(delta)),
                            on_set_ui_zoom: move |percent: i32| state.with_mut_counted(|shell| shell.set_ui_zoom_percent(percent)),
                            on_adjust_main_zoom: move |delta: i32| {
                                state.with_mut_counted(|shell| shell.adjust_main_zoom(delta));
                                apply_active_terminal_zoom(state);
                            },
                            on_set_main_zoom: move |percent: i32| {
                                state.with_mut_counted(|shell| shell.set_main_zoom_percent(percent));
                                apply_active_terminal_zoom(state);
                            },
                            on_set_terminal_theme_name: move |(theme, value): (UiTheme, String)| {
                                state.with_mut_counted(|shell| shell.set_terminal_theme_name_for(theme, value));
                                if state.read().snapshot().settings.theme == theme {
                                    apply_active_terminal_zoom(state);
                                }
                            },
                            on_trigger_update: move |_| {
                                let action = state.read().snapshot().update_call_to_action.mode;
                                if action == "restart" {
                                    restart_into_pending_update(state);
                                } else {
                                    spawn_update_workflow(state, UpdateWorkflowTrigger::Manual);
                                }
                            },
                            on_daemon_hot_restart: move |_| spawn_manual_daemon_hot_restart(state),
                            on_connect_ssh_custom: move |_| spawn_connect_ssh_custom(state),
                            on_ssh_target_change: move |value: String| state.with_mut_counted(|shell| shell.update_ssh_connect_target(value)),
                            on_ssh_prefix_change: move |value: String| state.with_mut_counted(|shell| shell.update_ssh_connect_prefix(value)),
                            on_clear_notification: move |id: u64| state.with_mut_counted(|shell| shell.clear_notification(id)),
                            on_clear_notifications: move |_| state.with_mut_counted(|shell| shell.clear_notifications()),
                            on_app_pane_action: {
                                let desktop = desktop.clone();
                                move |(pane_id, action, value): (String, String, Option<String>)| {
                                    let desktop = desktop.clone();
                                    spawn(app_pane_run_action(state, desktop, pane_id, action, value));
                                }
                            },
                            // A rail row was dragged to a new slot. Its own verb,
                            // not an `on_app_pane_action` with a smuggled payload:
                            // a reorder carries the pane's whole new order, which
                            // no other action does.
                            on_app_pane_reorder: {
                                let desktop = desktop.clone();
                                move |(pane_id, action, moved, parent, order): (
                                    String,
                                    String,
                                    String,
                                    Option<String>,
                                    Vec<String>,
                                )| {
                                    let desktop = desktop.clone();
                                    spawn(app_pane_run_reorder(
                                        state, desktop, pane_id, action, moved, parent, order,
                                    ));
                                }
                            },
                            on_app_pane_value: move |(widget_id, value): (String, String)| {
                                state.with_mut_counted(|shell| shell.set_app_pane_value(&widget_id, value));
                            },
                        }
                    }
                }
                // Cursor v1: agents working THIS session get a visible pointer.
                // Nothing renders when no agent has acted recently, so the
                // default path pays nothing.
                if !snapshot.agent_cursors.is_empty() {
                    AgentCursorOverlay { cursors: snapshot.agent_cursors.clone() }
                }
                // A contributed rail row's right-click menu (yedit's Rename /
                // Close). Drawn by the SAME ContextMenuOverlay the cwd tree
                // uses — one menu component for every right-click in the app.
                if let Some(menu) = snapshot.app_pane_context_menu.clone() {
                    ContextMenuOverlay {
                        position: menu.position,
                        window_size: context_menu_window_size,
                        // A contributed pane's rows live in the SAME rail, so
                        // their menu is banded like the rail's own.
                        band: Some(context_menu_rail_band),
                        palette: snapshot.palette,
                        items: menu.menu_items(),
                        // The contributed ROW's own label, which the row is
                        // saying right above this menu. No heading.
                        menu_title: String::new(),
                        keytip_tree: snapshot.keytip_tree.clone(),
                        alt_overlay_active: false,
                        alt_overlay_sequence: String::new(),
                        modal_root: None,
                        on_close: move |_| {
                            dismiss_menu(state, ShellMenu::AppPane);
                        },
                        on_action: {
                            let desktop = desktop.clone();
                            let (pane_id, row_id) = (menu.pane_id.clone(), menu.row_id.clone());
                            move |action: String| {
                                let desktop = desktop.clone();
                                state.with_mut_counted(|shell| shell.close_app_pane_context_menu());
                                spawn(app_pane_run_action(
                                    state,
                                    desktop,
                                    pane_id.clone(),
                                    action,
                                    Some(row_id.clone()),
                                ));
                            }
                        },
                    }
                }
                if let Some(row) = context_menu_overlay.clone() {
                    ContextMenuOverlay {
                        position: snapshot.context_menu_position.unwrap_or((18.0, 60.0)),
                        window_size: context_menu_window_size,
                        // The cwd tree is DOM-owned, so over a TERMINAL viewport
                        // this menu keeps WINDOW clamping — banding it would pin
                        // the tree's menu to a strip it has no reason to respect.
                        // Over a NATIVE web surface the spill is invisible and
                        // unclickable (legacy stacking), so there it is banded to
                        // the tree. `None` when no page owns the viewport.
                        band: context_menu_sidebar_band,
                        palette: snapshot.palette,
                        items: snapshot.row_menu_items.clone(),
                        menu_title: snapshot.row_menu_title.clone(),
                        keytip_tree: snapshot.keytip_tree.clone(),
                        // The row menu's DECLARED badges obey the same rule as
                        // every other declared badge: silent while a dialog owns
                        // the layer (`keytip_declared_badges_active`).
                        alt_overlay_active: keytip_declared_badges_active(&snapshot),
                        alt_overlay_sequence: snapshot.alt_overlay_sequence.clone(),
                        modal_root: None,
                        on_close: move |_| {
                            dismiss_menu(state, ShellMenu::Row);
                        },
                        on_action: {
                            let row = row.clone();
                            move |id: String| dispatch_row_menu_action(state, row.clone(), id)
                        },
                    }
                }
                // The WEBTABS RAIL's row menu — the rail was the last row
                // surface in the product with no right-click. Same
                // `ContextMenuOverlay`, so a rail row's menu is the cwd tree's
                // menu in look, keyboard story and dismissal.
                if let Some(menu) = snapshot.web_tab_context_menu.clone() {
                    ContextMenuOverlay {
                        position: menu.position,
                        window_size: context_menu_window_size,
                        // ONE menu, TWO anchors, and they neighbour different
                        // things. THE reported bug for the RAIL: a rail row sits
                        // at the window's right edge, so the menu anchors right
                        // and grew LEFTWARD over the page — where a legacy web
                        // surface composites above it and clipped everything past
                        // the rail's inner edge. The classic STRIP has no band to
                        // be saved by: it hangs over pure page, so it takes the
                        // STASH instead (`strip_dropdown_over_viewport`) and
                        // banding it to a rail that may not even be mounted would
                        // squeeze it into nothing.
                        band: match menu.anchor {
                            WebSurfaceChromeAnchor::Rail => Some(context_menu_rail_band),
                            WebSurfaceChromeAnchor::Strip => None,
                        },
                        palette: snapshot.palette,
                        items: snapshot.web_tab_menu_items.clone(),
                        menu_title: snapshot.web_tab_menu_title.clone(),
                        keytip_tree: snapshot.keytip_tree.clone(),
                        alt_overlay_active: false,
                        alt_overlay_sequence: String::new(),
                        modal_root: None,
                        on_close: move |_| {
                            dismiss_menu(state, ShellMenu::WebTab);
                        },
                        on_action: {
                            let menu = menu.clone();
                            move |id: String| {
                                dispatch_web_tab_menu_action(state, menu.clone(), id)
                            }
                        },
                    }
                }
                // The PROFILE dropdown. ONE mount for BOTH anchor sites (the
                // vertical rail's header badge and the classic strip's badge):
                // the two badges write the same state slot with a different
                // anchor point, so they cannot become two different menus.
                if let Some(switcher) = snapshot.web_profile_switcher.clone() {
                    ContextMenuOverlay {
                        position: switcher.position,
                        window_size: context_menu_window_size,
                        // ONE menu, TWO anchors — and the two anchors neighbour
                        // different things, which is the one place they may
                        // legitimately differ. The rail badge is rail chrome, so
                        // its dropdown is banded. The classic strip's badge hangs
                        // over PURE PAGE: no DOM band exists below the strip, so
                        // geometry cannot save it and the STASH does
                        // (`strip_dropdown_over_viewport`).
                        band: match switcher.anchor {
                            WebSurfaceChromeAnchor::Rail => Some(context_menu_rail_band),
                            WebSurfaceChromeAnchor::Strip => None,
                        },
                        palette: snapshot.palette,
                        items: switcher.menu_items(),
                        menu_title: "Profile".to_string(),
                        keytip_tree: snapshot.keytip_tree.clone(),
                        alt_overlay_active: false,
                        alt_overlay_sequence: String::new(),
                        // Anchored on the classic STRIP this dropdown IS the
                        // strip-dropdown top modal (`render_top_modal`), so it
                        // names itself as the §4 walk root; rail-anchored it is
                        // an ordinary menu and stamps nothing.
                        modal_root: match switcher.anchor {
                            WebSurfaceChromeAnchor::Strip => Some("strip-dropdown".to_string()),
                            WebSurfaceChromeAnchor::Rail => None,
                        },
                        on_close: move |_| {
                            dismiss_menu(state, ShellMenu::WebProfile);
                        },
                        on_action: {
                            let session_path = switcher.session_path.clone();
                            let current = switcher.current_profile.clone();
                            move |id: String| {
                                let Some(profile) = id.strip_prefix("webprofile:") else {
                                    return;
                                };
                                let profile = profile.to_string();
                                state.with_mut_counted(|shell| shell.close_web_profile_switcher());
                                // Choosing the profile you are already on is a
                                // dismissal, not a teardown-and-rebuild.
                                if profile == current {
                                    return;
                                }
                                spawn_web_profile_switch(state, session_path.clone(), profile);
                            }
                        },
                    }
                }
                // F.1: while any whole-viewport transient chrome is up, one
                // full-window cover routes ALL input to the shell (see
                // `chrome_transient_over_viewport`). Invisible and
                // pointer-events:none — it only feeds the input region.
                if chrome_transient_over_viewport(&snapshot) {
                    div {
                        "data-covers-web-surface": "transient-chrome",
                        style: "position:fixed; inset:0; pointer-events:none; background:transparent; z-index:0;",
                    }
                }
                // §12.3 modal marker: the JS key bridge reads this to know a
                // dialog owns Enter/Escape/Backspace while the ALT overlay is
                // closed. Rendered from `render_top_modal`, the same precedence
                // `modal_key_dispatch` uses, so the DOM and the dispatcher can
                // never disagree about which dialog is on top.
                if let Some(top_modal) = render_top_modal(&snapshot) {
                    div {
                        // ONE kind table ([`TopModal::kind`]), shared with the
                        // `data-yggterm-modal-root` subtree stamps and the ALT
                        // layer's modal scope.
                        "data-yggterm-modal-open": top_modal.kind(),
                        // …and the keyboard contract it honours (§12.4), so the
                        // one bridge enforces Form mode's trap without keeping a
                        // second list of which dialogs are which.
                        "data-yggterm-modal-mode": top_modal.keyboard_mode().as_str(),
                        "data-keytip-exempt": "modal-marker",
                        style: "display:none;",
                    }
                    // §12.3's VISIBLE half: the dialog keys have worked at the
                    // modal boundary since 2026-07-22, but nothing on screen
                    // SAID so, and an affordance the user cannot see does not
                    // exist (user-reported: "modals not showing any key menu").
                    // One bar, one owner: the labels come from the SAME
                    // precedence and the `modal_key_hints` table beside the
                    // dispatcher, so the bar can never promise a key the
                    // dispatcher swallows.
                    div {
                        "data-yggterm-modal-key-hints": "1",
                        "data-keytip-exempt": "modal-key-hints",
                        style: format!(
                            "position:fixed; bottom:18px; left:50%; transform:translateX(-50%); z-index:410; \
                             display:flex; align-items:center; gap:12px; height:24px; padding:0 14px; border-radius:999px; \
                             background:rgba(95,168,255,0.16); box-shadow: inset 0 0 0 1px rgba(95,168,255,0.42); \
                             color:{}; font-size:11px; font-weight:700; letter-spacing:0.3px; pointer-events:none; \
                             white-space:nowrap;",
                            snapshot.palette.accent,
                        ),
                        for (key, action) in modal_key_hints(top_modal).iter() {
                            span {
                                span { style: "font-weight:800;", "{key}" }
                                span { style: "opacity:0.62;", " {action}" }
                            }
                        }
                    }
                }
                // The MENU marker, the modal marker's twin. Escape has to close a
                // floating menu without the ALT layer being up, and a keydown at
                // the shell root never sees it — the window-level bridge below
                // the webview is the only listener a focused terminal cannot eat.
                // Rendered from `render_top_menu`, the precedence
                // `dismiss_top_menu` resolves with, so the key the bridge sends
                // and the menu Rust closes are the same menu. No marker means no
                // Escape is intercepted, which is how a menu-less terminal keeps
                // its own Escape.
                if let Some(top_menu) = render_top_menu(&snapshot) {
                    div {
                        "data-yggterm-menu-open": match top_menu {
                            ShellMenu::WebProfile => "web-profile",
                            ShellMenu::WebTab => "web-tab",
                            ShellMenu::Row => "row",
                            ShellMenu::AppPane => "app-pane",
                        },
                        "data-keytip-exempt": "menu-marker",
                        style: "display:none;",
                    }
                }
                if let Some(pending_delete) = snapshot.pending_delete.clone() {
                    DeleteConfirmOverlay {
                        pending: pending_delete,
                        palette: snapshot.palette,
                        on_cancel: move |_| state.with_mut_counted(|shell| shell.cancel_delete_dialog()),
                        on_confirm: move |_| queue_delete_selected_items(state, true),
                        on_confirm_unkept: move |_| queue_delete_unkept_live_sessions(state),
                    }
                }
                if snapshot.pending_classic_tabs_switch {
                    ClassicTabsSwitchOverlay {
                        palette: snapshot.palette,
                        group_count: snapshot
                            .active_web_surface_overlay
                            .as_ref()
                            .map(|overlay| overlay.tabs.iter().filter(|tab| tab.group_size > 0).count())
                            .unwrap_or(0),
                        filed_count: snapshot
                            .active_web_surface_overlay
                            .as_ref()
                            .map(|overlay| overlay.tabs.iter().filter(|tab| tab.group_head.is_some()).count())
                            .unwrap_or(0),
                        on_cancel: move |_| state.with_mut_counted(|shell| shell.cancel_classic_tabs_switch()),
                        on_confirm: move |_| state.with_mut_counted(|shell| shell.confirm_classic_tabs_switch()),
                    }
                }
                // THE OMNIBOX, RAISED. Owner requirement: focusing the address
                // input opens a CENTRED palette with visible results instead of
                // a small corner field with a popover under it — and any route
                // that focuses the input raises it, because the raise keys on
                // the DRAFT existing rather than on which gesture created it.
                //
                // ⛔ It is mounted HERE, with the other over-viewport modals,
                // not inside the rail. A native web surface draws above ALL DOM,
                // so a palette drawn inside the rail's own subtree would be
                // invisible over a browsing session; being a `TopModal` is what
                // makes the reconciler stash the surface first.
                if snapshot.web_command_palette_open
                    && let Some(overlay) = snapshot.active_web_surface_overlay.clone()
                {
                    {
                        let draft = overlay.address_text.clone();
                        let items = web_omnibox_palette_items(&draft, &overlay.address_suggestions);
                        let session = overlay.session.clone();
                        let ssh = state
                            .with(|shell| shell.web_surface_session_ssh_target(&session));
                        let count = items.len();
                        let accept_draft = draft.clone();
                        let (move_session, accept_session, dismiss_session) =
                            (session.clone(), session.clone(), session.clone());
                        rsx! {
                            // Hover, focus and the results' scrollbar — the
                            // things inline styles cannot express. Mounted with
                            // the palette rather than in a global block so the
                            // rules exist exactly while something uses them.
                            style { {YGGUI_COMMAND_PALETTE_CSS} }
                            CommandPalette {
                                palette: CommandPalettePalette::new(
                                    snapshot.palette.text,
                                    snapshot.palette.muted,
                                    snapshot.palette.panel,
                                    snapshot.palette.border,
                                    snapshot.palette.accent_soft,
                                    "rgba(16,24,34,0.34)",
                                ),
                                query: draft,
                                items,
                                // Row 0 is what plain Enter does, so "nothing
                                // chosen yet" and "the go row" are the same
                                // state and the palette always has a target.
                                selected: overlay.address_suggestion_index.unwrap_or(0),
                                placeholder: "Search or enter address".to_string(),
                                empty_label: "Type an address, or a phrase to search for".to_string(),
                                on_query: move |next: String| {
                                    let session = session.clone();
                                    state.with_mut_counted(|shell| {
                                        shell.web_surface_type_address(&session, next);
                                    });
                                },
                                on_move: move |moved: PaletteMove| {
                                    let session = move_session.clone();
                                    state.with_mut_counted(|shell| {
                                        let current = shell
                                            .web_surfaces
                                            .get(&session)
                                            .and_then(|surface| surface.address_suggestion_index)
                                            .unwrap_or(0);
                                        let next = palette_index_after(current, count, moved);
                                        shell.web_surface_set_address_suggestion(&session, next);
                                    });
                                },
                                on_accept: move |id: String| {
                                    let session = accept_session.clone();
                                    let Some(url) = web_omnibox_palette_target(&id, &accept_draft)
                                    else {
                                        return;
                                    };
                                    let tab = state.with(|shell| {
                                        shell
                                            .web_surfaces
                                            .get(&session)
                                            .map(|surface| surface.active_tab)
                                    });
                                    if let Some(tab) = tab {
                                        // Clearing the draft is what SHUTS the
                                        // palette: it is the one fact the raise
                                        // reads, so navigating without clearing
                                        // would leave it standing over the page
                                        // it just loaded.
                                        state.with_mut_counted(|shell| {
                                            shell.web_surface_set_address_draft(&session, None)
                                        });
                                        navigate_web_surface_tab(
                                            state,
                                            session.clone(),
                                            tab,
                                            url,
                                            ssh.clone(),
                                            None,
                                        );
                                    }
                                },
                                on_dismiss: move |_| {
                                    let session = dismiss_session.clone();
                                    state.with_mut_counted(|shell| {
                                        shell.web_surface_set_address_draft(&session, None)
                                    });
                                },
                            }
                        }
                    }
                }
                if let Some(dialog) = snapshot.copy_edit_dialog.clone() {
                    CopyEditOverlay {
                        dialog,
                        palette: snapshot.palette,
                        on_change: move |value: String| update_copy_edit_value(state, value),
                        on_cancel: move |_| cancel_copy_edit(state),
                        on_save: move |_| commit_copy_edit(state),
                    }
                }
                if let Some(dialog) = snapshot.pending_fido2.clone() {
                    Fido2PresenceOverlay {
                        dialog: dialog.clone(),
                        palette: snapshot.palette,
                        // The picker passes the chosen account's credential_id; a
                        // single-account Approve passes None (the app signs the
                        // only match).
                        on_approve: {
                            let dialog = dialog.clone();
                            move |credential_id: Option<String>| {
                                resolve_fido2_dialog(state, dialog.clone(), true, credential_id)
                            }
                        },
                        on_decline: move |_| resolve_fido2_dialog(state, dialog.clone(), false, None),
                    }
                }
                // The capture prompt, mounted AFTER the passkey dialog so it
                // paints above it — the reverse of `TopModal`'s topmost-first
                // list, which is the convention this tree already follows.
                if let Some(dialog) = snapshot.pending_media_capture.clone() {
                    MediaCapturePresenceOverlay {
                        dialog: dialog.clone(),
                        palette: snapshot.palette,
                        on_answer: {
                            let dialog = dialog.clone();
                            move |answer: MediaCaptureAnswer| {
                                resolve_media_capture_dialog(
                                    state,
                                    dioxus_desktop::window(),
                                    dialog.clone(),
                                    answer,
                                    "dialog_click",
                                )
                            }
                        },
                    }
                }
                if snapshot.launch_flags_open {
                    LaunchFlagsOverlay {
                        snapshot: snapshot.clone(),
                        on_close: move |_| state.with_mut_counted(|shell| shell.set_launch_flags_open(false)),
                        on_change: move |(slug, value): (String, String)| state.with_mut_counted(|shell| shell.update_agent_cli_extra_args(slug, value)),
                        on_reset: move |slug: String| state.with_mut_counted(|shell| shell.reset_agent_cli_extra_args(&slug)),
                        // Same two handlers the settings rail's fields use, so
                        // typing in the modal releases the terminal's key grab
                        // and blurring hands it back — a modal that invented its
                        // own focus contract is how a dialog swallows keystrokes.
                        on_focus_input: move |_| focus_settings_field(state, "launch-flags"),
                        on_blur_input: move |_| reclaim_active_terminal_input_after_settings_blur(state),
                    }
                }
                if snapshot.cli_install_open {
                    CliInstallOverlay {
                        palette: snapshot.palette,
                        theme: snapshot.settings.theme,
                        // THIS machine only. The remote hosts are listed with an
                        // honest "not probed" rather than a guess: the GUI can
                        // read its own PATH, and reaching over ssh for the others
                        // is the provisioner's job, not the renderer's.
                        machines: cli_install_machines(&snapshot),
                        consent: yggterm_core::cli_install::InstallConsent::from_wire(
                            &snapshot.settings.agent_cli_install_consent,
                        ),
                        pending: false,
                        on_grant: move |_| state.with_mut_counted(|shell| {
                            shell.set_agent_cli_install_consent(
                                yggterm_core::cli_install::InstallConsent::Granted,
                            )
                        }),
                        on_decline: move |_| state.with_mut_counted(|shell| {
                            shell.set_agent_cli_install_consent(
                                yggterm_core::cli_install::InstallConsent::Declined,
                            )
                        }),
                        on_install_all: move |_| state.with_mut_counted(|shell| {
                            shell.request_recommended_cli_installs()
                        }),
                        on_close: move |_| state.with_mut_counted(|shell| shell.set_cli_install_open(false)),
                    }
                }
                if snapshot.theme_editor_open {
                    ThemeEditorOverlay {
                        snapshot: snapshot.clone(),
                        on_close: move |_| state.with_mut_counted(|shell| shell.close_theme_editor()),
                        on_reset: move |_| state.with_mut_counted(|shell| shell.reset_theme_editor()),
                        on_seed: move |_| state.with_mut_counted(|shell| shell.seed_theme_editor()),
                        on_set_ui_theme: move |theme: UiTheme| state.with_mut_counted(|shell| shell.set_ui_theme(theme)),
                        on_add_stop: move |_| state.with_mut_counted(|shell| shell.add_theme_stop(None)),
                        on_remove_stop: move |_| state.with_mut_counted(|shell| shell.remove_selected_theme_stop()),
                        on_pick_stop: move |index: usize| state.with_mut_counted(|shell| shell.select_theme_stop(index)),
                        on_begin_drag_stop: move |index: usize| state.with_mut_counted(|shell| shell.begin_theme_drag(index)),
                        on_drag_stop: move |(x, y): (f32, f32)| state.with_mut_counted(|shell| shell.move_theme_stop(x, y)),
                        on_end_drag_stop: move |_| state.with_mut_counted(|shell| shell.end_theme_drag()),
                        on_double_click_pad: move |(x, y): (f32, f32)| state.with_mut_counted(|shell| shell.add_theme_stop_at(x, y)),
                        on_update_stop_color: move |value: String| state.with_mut_counted(|shell| shell.update_selected_theme_color(value)),
                        on_pick_swatch: move |value: String| state.with_mut_counted(|shell| shell.update_selected_theme_color(value)),
                        on_set_brightness: move |value: f32| state.with_mut_counted(|shell| shell.update_theme_brightness(value)),
                        on_set_alpha: move |value: f32| state.with_mut_counted(|shell| shell.update_theme_alpha(value)),
                        on_set_grain: move |value: f32| state.with_mut_counted(|shell| shell.update_theme_grain(value)),
                    }
                }
                if !snapshot.notifications.is_empty() {
                    // The wrapper exists ONLY to give the toast an ancestor that
                    // states whether the titlebar is pinned. The obvious
                    // `[data-titlebar-auto-hide-enabled]` element is the workspace
                    // ROW — a sibling of the toast layer, not an ancestor — so a
                    // rule keyed on it silently matched nothing (measured: the
                    // pinned arm left the card at top:22px, unchanged). A
                    // descendant selector needs a real ancestor; this is it.
                    div {
                        "data-yggterm-titlebar-pinned": if titlebar_auto_hide_enabled { "false" } else { "true" },
                        style: "display:contents;",
                        ToastViewport {
                            items: snapshot.notifications.clone(),
                            palette: ToastPalette {
                                text: snapshot.palette.text,
                                muted: snapshot.palette.muted,
                                accent: snapshot.palette.accent,
                                is_dark: palette_is_dark(snapshot.palette),
                            },
                        anchor: toast_anchor(&snapshot),
                        max_age_ms: TOAST_VIEWPORT_MAX_AGE_MS,
                        max_visible: TOAST_VIEWPORT_MAX_VISIBLE,
                        now_ms: current_millis(),
                        // ⛔ DISMISS, not clear. The X on a toast means "stop
                        // covering my screen"; the panel keeps the record. These
                        // were the same call until the user lost an agent's
                        // completion notice by closing its popup.
                        on_clear: move |id: u64| state.with_mut_counted(|shell| shell.dismiss_toast(id)),
                        on_activate: move |session_path: String| {
                            spawn_open_session_from_notification(state, session_path)
                        },
                        }
                    }
                }
                if !snapshot.drag_paths.is_empty() {
                    DragGhost {
                        snapshot: snapshot.clone(),
                    }
                }
                // The SAME ghost the cwd tree has always drawn, for the row
                // lists that had none: the WebTabs rail and every contributed
                // pane. Drawn HERE, once, from the one gesture — a per-surface
                // ghost would be a third card that could drift from the other
                // two, and the pointer leaves the list long before the drop.
                if let Some(drag) = snapshot.row_drag.as_ref().filter(|drag| drag.begun) {
                    DragGhostCard {
                        x: drag.pointer.0,
                        y: drag.pointer.1,
                        primary_label: if drag.label.is_empty() {
                            "Move row".to_string()
                        } else {
                            drag.label.clone()
                        },
                        extra_count: 0,
                        target_hint: drag.target.as_ref().map(row_drop_target_hint),
                        palette: DragGhostPalette {
                            text: snapshot.palette.text,
                            muted: snapshot.palette.muted,
                            accent: snapshot.palette.accent,
                            accent_soft: snapshot.palette.accent_soft,
                        },
                    }
                }
            }
        }
    }
}
