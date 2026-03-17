use anyhow::{Context, Result};
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yggterm_core::{AppSettings, SessionNode, SessionStore, UiTheme};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let store = SessionStore::open_or_init()?;
    launch_gpui_gui(store)
}

fn launch_gpui_gui(store: SessionStore) -> Result<()> {
    let tree = store.load_tree()?;
    let browser_tree = store.load_codex_tree().unwrap_or_else(|_| tree.clone());
    let settings = store.load_settings().unwrap_or_default();
    let ghostty_bridge = yggterm_ghostty_bridge::bridge_status();

    yggterm_zed_shell::launch_gpui_shell(yggterm_zed_shell::ShellBootstrap {
        tree,
        browser_tree,
        theme: settings.theme,
        ghostty_bridge_enabled: ghostty_bridge.linked_runtime_available(),
        ghostty_embedded_surface_supported: ghostty_bridge.embedded_surface_available(),
        ghostty_bridge_detail: ghostty_bridge.detail,
        prefer_ghostty_backend: settings.prefer_ghostty_backend,
    })
}

fn launch_gui(store: SessionStore) -> Result<()> {
    let tree = store.load_tree()?;
    let settings = store.load_settings().unwrap_or_default();
    let ghostty_bridge_enabled = yggterm_ghostty_bridge::initialize_bridge().is_ok();

    let app = YggtermGuiApp {
        store,
        tree,
        settings,
        selected_path: None,
        terminals: Vec::new(),
        active_terminal_id: None,
        next_terminal_id: 1,
        tree_filter: String::new(),
        ghostty_bridge_enabled,
        save_error: None,
        new_session_input: String::new(),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Yggdrasil Terminal")
            .with_inner_size([1460.0, 920.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Yggdrasil Terminal",
        options,
        Box::new(|_| Ok(Box::new(app))),
    )
    .map_err(|err| anyhow::anyhow!("failed to launch GUI: {err}"))?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum BackendMode {
    GhosttyRequested,
    PtyFallback,
}

struct YggtermGuiApp {
    store: SessionStore,
    tree: SessionNode,
    settings: AppSettings,
    selected_path: Option<String>,
    terminals: Vec<ManagedTerminal>,
    active_terminal_id: Option<u64>,
    next_terminal_id: u64,
    tree_filter: String,
    ghostty_bridge_enabled: bool,
    save_error: Option<String>,
    new_session_input: String,
}

struct ManagedTerminal {
    id: u64,
    title: String,
    session_path: String,
    working_dir: String,
    child: Child,
    stdin: ChildStdin,
    output: Arc<Mutex<Vec<String>>>,
    cmd_input: String,
    is_alive: bool,
}

impl ManagedTerminal {
    fn new(id: u64, session_path: String, working_dir: String) -> Result<Self> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = ProcessCommand::new(&shell);
        if shell.ends_with("bash") || shell.ends_with("zsh") || shell.ends_with("fish") {
            cmd.arg("-i");
        }

        cmd.current_dir(&working_dir)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn shell in {working_dir}"))?;

        let stdin = child.stdin.take().context("failed to open child stdin")?;
        let stdout = child.stdout.take().context("failed to open child stdout")?;
        let stderr = child.stderr.take().context("failed to open child stderr")?;

        let output = Arc::new(Mutex::new(Vec::new()));
        push_output_line(
            &output,
            format!("[yggterm] terminal started in {working_dir}"),
        );

        {
            let output_clone = Arc::clone(&output);
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) => push_output_line(&output_clone, line),
                        Err(err) => {
                            push_output_line(&output_clone, format!("[stdout read error] {err}"));
                            break;
                        }
                    }
                }
            });
        }

        {
            let output_clone = Arc::clone(&output);
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(line) => push_output_line(&output_clone, format!("[stderr] {line}")),
                        Err(err) => {
                            push_output_line(&output_clone, format!("[stderr read error] {err}"));
                            break;
                        }
                    }
                }
            });
        }

        let title = std::path::Path::new(&working_dir)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "session".to_string());

        Ok(Self {
            id,
            title,
            session_path,
            working_dir,
            child,
            stdin,
            output,
            cmd_input: String::new(),
            is_alive: true,
        })
    }

    fn send_command(&mut self) {
        let cmd = self.cmd_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        if writeln!(self.stdin, "{cmd}").is_err() || self.stdin.flush().is_err() {
            push_output_line(
                &self.output,
                "[yggterm] failed to write command to terminal".to_string(),
            );
        } else {
            push_output_line(&self.output, format!("> {cmd}"));
        }
        self.cmd_input.clear();
    }

    fn poll_status(&mut self) {
        if !self.is_alive {
            return;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.is_alive = false;
                push_output_line(&self.output, format!("[yggterm] terminal exited: {status}"));
            }
            Ok(None) => {}
            Err(err) => {
                self.is_alive = false;
                push_output_line(
                    &self.output,
                    format!("[yggterm] terminal status error: {err}"),
                );
            }
        }
    }

    fn terminate(&mut self) {
        if self.is_alive {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.is_alive = false;
        }
    }
}

impl Drop for YggtermGuiApp {
    fn drop(&mut self) {
        for terminal in &mut self.terminals {
            terminal.terminate();
        }
        let _ = self.store.save_settings(&self.settings);
    }
}

impl YggtermGuiApp {
    fn backend_mode(&self) -> BackendMode {
        if self.settings.prefer_ghostty_backend && self.ghostty_bridge_enabled {
            BackendMode::GhosttyRequested
        } else {
            BackendMode::PtyFallback
        }
    }

    fn active_index(&self) -> Option<usize> {
        let id = self.active_terminal_id?;
        self.terminals.iter().position(|t| t.id == id)
    }

    fn open_or_focus_selected(&mut self) {
        let Some(path) = &self.selected_path else {
            return;
        };
        self.open_or_focus_by_path(path.clone());
    }

    fn open_or_focus_by_path(&mut self, path: String) {
        if let Some(term) = self.terminals.iter().find(|t| t.session_path == path) {
            self.active_terminal_id = Some(term.id);
            return;
        }

        let id = self.next_terminal_id;
        self.next_terminal_id += 1;
        match ManagedTerminal::new(id, path.clone(), path) {
            Ok(term) => {
                self.active_terminal_id = Some(term.id);
                self.terminals.push(term);
            }
            Err(err) => {
                self.save_error = Some(format!("failed to open terminal: {err}"));
            }
        }
    }

    fn close_active_terminal(&mut self) {
        if let Some(idx) = self.active_index() {
            self.terminals[idx].terminate();
            self.terminals.remove(idx);
            self.active_terminal_id = self.terminals.last().map(|t| t.id);
        }
    }

    fn create_session_from_input(&mut self) {
        let input = self.new_session_input.trim().to_string();
        if input.is_empty() {
            return;
        }
        match self.store.create_session_path(&input) {
            Ok(path) => {
                self.new_session_input.clear();
                self.selected_path = Some(path.display().to_string());
                if let Ok(tree) = self.store.load_tree() {
                    self.tree = tree;
                }
            }
            Err(err) => {
                self.save_error = Some(format!("failed to create session: {err}"));
            }
        }
    }

    fn persist_settings(&mut self) {
        self.save_error = self
            .store
            .save_settings(&self.settings)
            .err()
            .map(|e| format!("failed to save settings: {e}"));
    }

    fn total_session_count(&self) -> usize {
        count_leaf_sessions(&self.tree)
    }
}

impl eframe::App for YggtermGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx, &self.settings);
        ctx.request_repaint_after(Duration::from_millis(250));

        for term in &mut self.terminals {
            term.poll_status();
        }

        let total_sessions = self.total_session_count();
        let selected_label = self
            .selected_path
            .as_deref()
            .map(short_session_label)
            .unwrap_or("No session selected")
            .to_string();
        let backend_label = match self.backend_mode() {
            BackendMode::GhosttyRequested => "Ghostty requested",
            BackendMode::PtyFallback => "PTY fallback",
        };

        egui::TopBottomPanel::top("top_bar")
            .exact_height(66.0)
            .show(ctx, |ui| {
                chrome_frame(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.menu_button("Menu", |ui| {
                            if ui.button("Open Selected Terminal").clicked() {
                                self.open_or_focus_selected();
                                ui.close_menu();
                            }
                            if ui.button("Close Active Terminal").clicked() {
                                self.close_active_terminal();
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Toggle Session Sidebar").clicked() {
                                self.settings.show_tree = !self.settings.show_tree;
                                self.persist_settings();
                                ui.close_menu();
                            }
                            if ui.button("Toggle Settings Panel").clicked() {
                                self.settings.show_settings = !self.settings.show_settings;
                                self.persist_settings();
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label("Theme");
                            if ui.button("Zed Dark").clicked() {
                                self.settings.theme = UiTheme::ZedDark;
                                self.persist_settings();
                                ui.close_menu();
                            }
                            if ui.button("Zed Light").clicked() {
                                self.settings.theme = UiTheme::ZedLight;
                                self.persist_settings();
                                ui.close_menu();
                            }
                        });

                        if ui
                            .selectable_label(self.settings.show_tree, "Sidebar")
                            .clicked()
                        {
                            self.settings.show_tree = !self.settings.show_tree;
                            self.persist_settings();
                        }
                        if ui
                            .selectable_label(self.settings.show_settings, "Settings")
                            .clicked()
                        {
                            self.settings.show_settings = !self.settings.show_settings;
                            self.persist_settings();
                        }

                        ui.separator();
                        ui.vertical(|ui| {
                            ui.heading("Yggdrasil Terminal");
                            ui.small("Remote-first Ghostty workspace shaped after Zed");
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            status_chip(ui, backend_label);
                            status_chip(ui, &format!("{} open tabs", self.terminals.len()));
                            status_chip(ui, &format!("{} sessions", total_sessions));
                        });
                    });
                });
            });

        if self.settings.show_tree {
            egui::SidePanel::left("sessions_panel")
                .resizable(true)
                .default_width(self.settings.tree_width)
                .show(ctx, |ui| {
                    chrome_frame(ui).show(ui, |ui| {
                        ui.label(egui::RichText::new("SESSION SIDEBAR").small().strong());
                        ui.heading("Virtual sessions");
                        ui.label(
                            "Tree nodes represent saved session metadata. The current scaffold still uses directories under ~/.yggterm/sessions.",
                        );

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            status_chip(ui, "Codex");
                            status_chip(ui, "SSH");
                            status_chip(ui, "Ghostty");
                            status_chip(ui, "Restore groups");
                        });

                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Search").small().strong());
                        ui.add(
                            egui::TextEdit::singleline(&mut self.tree_filter)
                                .hint_text("remote/prod/codex-session-tui"),
                        );

                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("New virtual path").small().strong());
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_session_input)
                                    .hint_text("machines/pi/ghostty-admin"),
                            );
                            if ui.button("Add").clicked() {
                                self.create_session_from_input();
                            }
                        });

                        ui.add_space(10.0);
                        surface_frame(ui).show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Saved sessions")
                                    .small()
                                    .strong(),
                            );
                            ui.add_space(6.0);

                            let root_path = self.tree.path.display().to_string();
                            let mut open_now = None;
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    render_session_node(
                                        ui,
                                        &self.tree,
                                        0,
                                        &mut self.selected_path,
                                        &root_path,
                                        self.tree_filter.trim(),
                                        &mut open_now,
                                    );
                                });
                            if let Some(path) = open_now {
                                self.open_or_focus_by_path(path);
                            }
                        });

                        ui.add_space(10.0);
                        if ui.button("Open or focus selected terminal").clicked() {
                            self.open_or_focus_selected();
                        }
                        ui.small(format!("Selected: {selected_label}"));
                    });
                });
        }

        if self.settings.show_settings {
            egui::SidePanel::right("settings_panel")
                .resizable(true)
                .default_width(280.0)
                .show(ctx, |ui| {
                    chrome_frame(ui).show(ui, |ui| {
                        ui.label(egui::RichText::new("WORKSPACE SETTINGS").small().strong());
                        ui.heading("Shell preferences");
                        ui.label("Tune the current scaffold while the GPUI shell takes shape.");
                        ui.separator();

                        egui::ComboBox::from_label("Theme")
                            .selected_text(match self.settings.theme {
                                UiTheme::ZedDark => "Zed Dark",
                                UiTheme::ZedLight => "Zed Light",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    UiTheme::ZedDark,
                                    "Zed Dark",
                                );
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    UiTheme::ZedLight,
                                    "Zed Light",
                                );
                            });

                        ui.add(
                            egui::Slider::new(&mut self.settings.ui_font_size, 12.0..=22.0)
                                .text("UI font size"),
                        );
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings.terminal_font_size,
                                10.0..=22.0,
                            )
                            .text("Terminal font size"),
                        );

                        ui.checkbox(
                            &mut self.settings.prefer_ghostty_backend,
                            "Prefer Ghostty backend",
                        );
                        ui.checkbox(&mut self.settings.show_tree, "Show session sidebar");
                        ui.checkbox(&mut self.settings.show_settings, "Keep settings panel open");

                        ui.add_space(8.0);
                        surface_frame(ui).show(ui, |ui| {
                            ui.label(egui::RichText::new("Roadmap cues").small().strong());
                            ui.small("The scaffold should converge on these GPUI-era behaviors:");
                            ui.label("- restore all sessions and layout");
                            ui.label("- screenshot and clipboard relay into remote sessions");
                            ui.label("- metadata-driven groups for machines, teams, and Codex workspaces");
                        });

                        ui.add_space(8.0);
                        if ui.button("Save settings").clicked() {
                            self.persist_settings();
                        }

                        if let Some(err) = &self.save_error {
                            ui.separator();
                            ui.colored_label(ui.visuals().error_fg_color, err);
                        }
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            surface_frame(ui).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("WORKSPACE").small().strong());
                        ui.heading("Terminal viewport");
                    });
                    ui.separator();
                    if ui.button("Open selected terminal").clicked() {
                        self.open_or_focus_selected();
                    }
                    if ui.button("Close active").clicked() {
                        self.close_active_terminal();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        status_chip(ui, backend_label);
                        status_chip(ui, &selected_label);
                    });
                });
                ui.separator();

                if self.terminals.is_empty() {
                    ui.label("No live terminals yet. Open a saved session from the left sidebar.");
                    ui.add_space(8.0);
                    ui.label("- target shell: GPUI and Zed-like chrome");
                    ui.label("- target engine: Ghostty surfaces in the center pane");
                    ui.label(
                        "- target workflow: many remote sessions with durable restore metadata",
                    );
                    ui.add_space(8.0);
                    if ui.button("Open selected terminal").clicked() {
                        self.open_or_focus_selected();
                    }
                    ui.add_space(8.0);
                    ui.monospace(format!(
                        "bridge={} prefer_ghostty={} active_mode={}",
                        self.ghostty_bridge_enabled,
                        self.settings.prefer_ghostty_backend,
                        match self.backend_mode() {
                            BackendMode::GhosttyRequested => "ghostty_requested",
                            BackendMode::PtyFallback => "pty_fallback",
                        }
                    ));
                    return;
                }

                let mut close_id = None;
                chrome_frame(ui).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for term in &self.terminals {
                            let label = if term.is_alive {
                                format!("{} #{}", term.title, term.id)
                            } else {
                                format!("{} #{} done", term.title, term.id)
                            };

                            let button = egui::Button::new(label)
                                .selected(self.active_terminal_id == Some(term.id))
                                .corner_radius(6);
                            if ui.add(button).clicked() {
                                self.active_terminal_id = Some(term.id);
                            }
                            if ui.small_button("x").clicked() {
                                close_id = Some(term.id);
                            }
                        }
                    });
                });

                if let Some(id) = close_id {
                    if let Some(idx) = self.terminals.iter().position(|t| t.id == id) {
                        self.terminals[idx].terminate();
                        self.terminals.remove(idx);
                        self.active_terminal_id = self.terminals.last().map(|t| t.id);
                    }
                }

                ui.add_space(10.0);

                if let Some(idx) = self.active_index() {
                    let term = &mut self.terminals[idx];

                    surface_frame(ui).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            status_chip(
                                ui,
                                &format!("Session {}", short_session_label(&term.session_path)),
                            );
                            status_chip(ui, if term.is_alive { "Running" } else { "Exited" });
                            status_chip(ui, "Remote clipboard planned");
                            status_chip(ui, "Restore-all planned");
                        });

                        ui.add_space(8.0);
                        ui.label(format!("Working dir: {}", term.working_dir));

                        ui.add_space(8.0);
                        chrome_frame(ui).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Command");
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut term.cmd_input)
                                        .hint_text("Type command and press Enter"),
                                );
                                let send_with_enter = response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if send_with_enter || ui.button("Send").clicked() {
                                    term.send_command();
                                }
                            });
                        });

                        ui.add_space(8.0);

                        let lines = match term.output.lock() {
                            Ok(guard) => guard.clone(),
                            Err(_) => vec!["[yggterm] output lock poisoned".to_string()],
                        };

                        terminal_surface_frame(ui).show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for line in lines {
                                        ui.label(
                                            egui::RichText::new(line)
                                                .monospace()
                                                .size(self.settings.terminal_font_size),
                                        );
                                    }
                                });
                        });
                    });
                }
            });
        });

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(34.0)
            .show(ctx, |ui| {
                chrome_frame(ui).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.small(format!("Selected: {selected_label}"));
                        ui.separator();
                        ui.small(format!("Saved sessions: {total_sessions}"));
                        ui.separator();
                        ui.small(format!("Open terminals: {}", self.terminals.len()));
                        ui.separator();
                        ui.small(
                            "Reference Zed window is running on this X11 session for chrome checks",
                        );
                    });
                });
            });
    }
}

fn apply_theme(ctx: &egui::Context, settings: &AppSettings) {
    match settings.theme {
        UiTheme::ZedDark => {
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = egui::Color32::from_rgb(19, 22, 29);
            visuals.panel_fill = egui::Color32::from_rgb(26, 31, 40);
            visuals.faint_bg_color = egui::Color32::from_rgb(33, 39, 51);
            visuals.extreme_bg_color = egui::Color32::from_rgb(14, 17, 22);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 36, 47);
            visuals.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgb(61, 72, 91));
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(39, 46, 60);
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(49, 59, 77);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(43, 119, 242);
            visuals.selection.bg_fill = egui::Color32::from_rgb(43, 119, 242);
            ctx.set_visuals(visuals);
        }
        UiTheme::ZedLight => {
            let mut visuals = egui::Visuals::light();
            visuals.window_fill = egui::Color32::from_rgb(243, 246, 251);
            visuals.panel_fill = egui::Color32::from_rgb(232, 237, 245);
            visuals.faint_bg_color = egui::Color32::from_rgb(223, 230, 241);
            visuals.extreme_bg_color = egui::Color32::from_rgb(208, 218, 234);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(233, 239, 248);
            visuals.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgb(178, 188, 205));
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(201, 214, 238);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(61, 117, 234);
            visuals.selection.bg_fill = egui::Color32::from_rgb(132, 169, 238);
            ctx.set_visuals(visuals);
        }
    }

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.visuals.window_corner_radius = 10.into();
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(settings.ui_font_size, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(settings.ui_font_size, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(settings.terminal_font_size, egui::FontFamily::Monospace),
    );
    ctx.set_style(style);
}

fn render_session_node(
    ui: &mut egui::Ui,
    node: &SessionNode,
    depth: usize,
    selected_path: &mut Option<String>,
    full_path: &str,
    filter: &str,
    open_now: &mut Option<String>,
) -> bool {
    let filter_lc = filter.to_lowercase();
    let visible = node_matches_filter(node, &filter_lc);
    if !visible {
        return false;
    }

    let child_entries: Vec<(String, &SessionNode)> = node
        .children
        .iter()
        .map(|child| (child.path.display().to_string(), child))
        .collect();

    let label = if depth == 0 {
        format!("{} ({})", node.name, count_leaf_sessions(node))
    } else if node.children.is_empty() {
        node.name.clone()
    } else {
        format!("{} ({})", node.name, count_leaf_sessions(node))
    };

    if node.children.is_empty() {
        let is_selected = selected_path.as_deref() == Some(full_path);
        let response = ui.selectable_label(is_selected, egui::RichText::new(label).monospace());
        if response.clicked() {
            *selected_path = Some(full_path.to_string());
        }
        if response.double_clicked() {
            *selected_path = Some(full_path.to_string());
            *open_now = Some(full_path.to_string());
        }
        return true;
    }

    let header = egui::CollapsingHeader::new(egui::RichText::new(label).strong())
        .default_open(depth < 2)
        .id_salt(full_path);
    header.show(ui, |ui| {
        let is_selected = selected_path.as_deref() == Some(full_path);
        if ui.selectable_label(is_selected, "Focus group").clicked() {
            *selected_path = Some(full_path.to_string());
        }

        for (child_path, child) in child_entries {
            let _ = render_session_node(
                ui,
                child,
                depth + 1,
                selected_path,
                &child_path,
                filter,
                open_now,
            );
        }
    });

    true
}

fn node_matches_filter(node: &SessionNode, filter_lc: &str) -> bool {
    if filter_lc.is_empty() {
        return true;
    }
    if node.name.to_lowercase().contains(filter_lc) {
        return true;
    }
    node.children
        .iter()
        .any(|child| node_matches_filter(child, filter_lc))
}

fn push_output_line(output: &Arc<Mutex<Vec<String>>>, line: String) {
    if let Ok(mut lines) = output.lock() {
        lines.push(line);
        if lines.len() > 5000 {
            let drain = lines.len() - 5000;
            lines.drain(0..drain);
        }
    }
}

fn chrome_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
}

fn surface_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(10)
        .inner_margin(egui::Margin::same(12))
}

fn terminal_surface_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(10)
        .inner_margin(egui::Margin::same(12))
}

fn status_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).small());
        });
}

fn short_session_label(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn count_leaf_sessions(node: &SessionNode) -> usize {
    if node.children.is_empty() {
        return 1;
    }

    node.children.iter().map(count_leaf_sessions).sum()
}
