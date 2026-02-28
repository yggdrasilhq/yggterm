use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use yggterm_core::{AppSettings, SessionNode, SessionStore, UiTheme};

#[derive(Debug, Parser)]
#[command(name = "yggterm", version, about = "Yggdrasil Terminal")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize local state directories (~/.yggterm by default)
    Init,
    /// Create a nested session folder path (example: team/backend/api)
    MkSession { path: String },
    /// Print session tree
    Tree,
    /// Print environment and integration readiness
    Doctor,
    /// Print Zed upstream integration plan markers
    ZedPlan,
    /// Launch desktop GUI shell
    Gui,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .without_time()
        .init();

    let cli = Cli::parse();
    let store = SessionStore::open_or_init()?;

    match cli.command.unwrap_or(Command::Tree) {
        Command::Init => {
            println!("Initialized YGGTERM_HOME at {}", store.home_dir().display());
            println!("Sessions root at {}", store.sessions_root().display());
            println!("Settings file at {}", store.settings_path().display());
        }
        Command::MkSession { path } => {
            let created = store.create_session_path(&path)?;
            println!("Created session path: {}", created.display());
        }
        Command::Tree => {
            let platform = yggterm_platform::host_platform();
            let _ = yggterm_ghostty_bridge::initialize_bridge();
            let tree = store.load_tree()?;
            println!("Host platform: {:?}", platform);
            print!("{}", yggterm_ui::render_session_tree_text(&tree)?);
        }
        Command::Doctor => {
            let platform = yggterm_platform::host_platform();
            let env = yggterm_ghostty_bridge::GhosttyEnvironment::discover();
            let bridge = yggterm_ghostty_bridge::initialize_bridge();
            println!("Host platform: {:?}", platform);
            println!("YGGTERM_HOME: {}", store.home_dir().display());
            println!(
                "Ghostty header discovered: {}",
                env.header_path.unwrap_or_else(|| "not found".to_string())
            );
            match bridge {
                Ok(()) => println!("Ghostty bridge init status: enabled"),
                Err(e) => {
                    println!("Ghostty bridge init status: disabled");
                    println!("Bridge detail: {e}");
                    println!("Hint: use packaged .deb build (ghostty-ffi) or build with --features ghostty-ffi");
                }
            }
        }
        Command::ZedPlan => {
            let plan = yggterm_zed_shell::shell_plan();
            println!("Use workspace::Item: {}", plan.uses_upstream_workspace_item);
            println!("Use project_panel tree: {}", plan.uses_upstream_project_panel);
            println!("Use terminal_view items: {}", plan.uses_upstream_terminal_view);
            println!(
                "Center viewport replaced by terminals: {}",
                plan.center_viewport_replaced_by_terminals
            );
            for marker in yggterm_zed_shell::upstream_type_markers() {
                println!("Marker: {marker}");
            }
        }
        Command::Gui => launch_gui(store)?,
    }

    Ok(())
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
            .with_inner_size([1320.0, 840.0]),
        ..Default::default()
    };

    eframe::run_native("Yggdrasil Terminal", options, Box::new(|_| Ok(Box::new(app))))
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
        push_output_line(&output, format!("[yggterm] terminal started in {working_dir}"));

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
}

impl eframe::App for YggtermGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(ctx, &self.settings);

        for term in &mut self.terminals {
            term.poll_status();
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("≡", |ui| {
                    if ui.button("New Terminal (Selected Session)").clicked() {
                        self.open_or_focus_selected();
                        ui.close_menu();
                    }
                    if ui.button("Close Active Terminal").clicked() {
                        self.close_active_terminal();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Toggle Session Tree").clicked() {
                        self.settings.show_tree = !self.settings.show_tree;
                        self.persist_settings();
                        ui.close_menu();
                    }
                    if ui.button("Toggle Settings Pane").clicked() {
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

                ui.heading("Yggdrasil Terminal");
                ui.separator();
                ui.label("Zed-style chrome + session-managed terminals");
                ui.separator();
                match self.backend_mode() {
                    BackendMode::GhosttyRequested => {
                        ui.label("Backend: Ghostty requested (surface embedding next)");
                    }
                    BackendMode::PtyFallback => {
                        ui.label("Backend: PTY fallback");
                    }
                }
            });
        });

        if self.settings.show_tree {
            egui::SidePanel::left("sessions_panel")
                .resizable(true)
                .default_width(self.settings.tree_width)
                .show(ctx, |ui| {
                    ui.heading("Session Tree");
                    ui.horizontal(|ui| {
                        ui.label("Filter");
                        ui.text_edit_singleline(&mut self.tree_filter);
                    });
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.new_session_input);
                        if ui.button("New").clicked() {
                            self.create_session_from_input();
                        }
                    });
                    ui.separator();

                    let root_path = self.tree.path.display().to_string();
                    let mut open_now = None;
                    render_session_node(
                        ui,
                        &self.tree,
                        0,
                        &mut self.selected_path,
                        &root_path,
                        self.tree_filter.trim(),
                        &mut open_now,
                    );
                    if let Some(path) = open_now {
                        self.open_or_focus_by_path(path);
                    }

                    ui.separator();
                    if ui.button("Open / Focus Terminal").clicked() {
                        self.open_or_focus_selected();
                    }
                    if let Some(path) = &self.selected_path {
                        ui.small(format!("Selected: {path}"));
                    }
                });
        }

        if self.settings.show_settings {
            egui::SidePanel::right("settings_panel")
                .resizable(true)
                .default_width(280.0)
                .show(ctx, |ui| {
                    ui.heading("Settings");
                    ui.separator();

                    egui::ComboBox::from_label("Theme")
                        .selected_text(match self.settings.theme {
                            UiTheme::ZedDark => "Zed Dark",
                            UiTheme::ZedLight => "Zed Light",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.theme, UiTheme::ZedDark, "Zed Dark");
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
                        egui::Slider::new(&mut self.settings.terminal_font_size, 10.0..=22.0)
                            .text("Terminal font size"),
                    );

                    ui.checkbox(&mut self.settings.prefer_ghostty_backend, "Prefer Ghostty backend");
                    ui.checkbox(&mut self.settings.show_tree, "Show session tree");
                    ui.checkbox(&mut self.settings.show_settings, "Keep settings pane open");

                    if ui.button("Save Settings").clicked() {
                        self.persist_settings();
                    }

                    if let Some(err) = &self.save_error {
                        ui.separator();
                        ui.colored_label(ui.visuals().error_fg_color, err);
                    }
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Terminal Workspace");
            ui.separator();

            if self.terminals.is_empty() {
                ui.label("No terminals open. Select a session in the tree and open one.");
                ui.label("Menu ≡ -> New Terminal (Selected Session)");
                ui.add_space(8.0);
                ui.label("Ghostty status:");
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
            ui.horizontal_wrapped(|ui| {
                for term in &self.terminals {
                    let label = if term.is_alive {
                        format!("{} #{}", term.title, term.id)
                    } else {
                        format!("{} #{} (done)", term.title, term.id)
                    };
                    if ui
                        .selectable_label(self.active_terminal_id == Some(term.id), label)
                        .clicked()
                    {
                        self.active_terminal_id = Some(term.id);
                    }
                    if ui.small_button("x").clicked() {
                        close_id = Some(term.id);
                    }
                }
            });

            if let Some(id) = close_id {
                if let Some(idx) = self.terminals.iter().position(|t| t.id == id) {
                    self.terminals[idx].terminate();
                    self.terminals.remove(idx);
                    self.active_terminal_id = self.terminals.last().map(|t| t.id);
                }
            }

            ui.separator();

            if let Some(idx) = self.active_index() {
                let term = &mut self.terminals[idx];

                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Session: {}", term.session_path));
                    ui.separator();
                    ui.label(format!("Working dir: {}", term.working_dir));
                    ui.separator();
                    ui.label(if term.is_alive { "Running" } else { "Exited" });
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut term.cmd_input)
                            .hint_text("Type command and press Enter"),
                    );
                    let send_with_enter =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if send_with_enter || ui.button("Send").clicked() {
                        term.send_command();
                    }
                });

                ui.separator();

                let lines = match term.output.lock() {
                    Ok(guard) => guard.clone(),
                    Err(_) => vec!["[yggterm] output lock poisoned".to_string()],
                };

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let rich = egui::RichText::new("").size(self.settings.terminal_font_size);
                        ui.label(rich);
                        for line in lines {
                            ui.label(egui::RichText::new(line).monospace().size(self.settings.terminal_font_size));
                        }
                    });
            }
        });

    }
}

fn apply_theme(ctx: &egui::Context, settings: &AppSettings) {
    match settings.theme {
        UiTheme::ZedDark => {
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = egui::Color32::from_rgb(33, 39, 49);
            visuals.panel_fill = egui::Color32::from_rgb(39, 46, 58);
            visuals.faint_bg_color = egui::Color32::from_rgb(48, 58, 73);
            visuals.extreme_bg_color = egui::Color32::from_rgb(27, 33, 43);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(41, 49, 62);
            visuals.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgb(81, 98, 120));
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(52, 62, 79);
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(63, 76, 98);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(71, 128, 247);
            visuals.selection.bg_fill = egui::Color32::from_rgb(71, 128, 247);
            ctx.set_visuals(visuals);
        }
        UiTheme::ZedLight => {
            let mut visuals = egui::Visuals::light();
            visuals.window_fill = egui::Color32::from_rgb(244, 247, 252);
            visuals.panel_fill = egui::Color32::from_rgb(233, 239, 248);
            visuals.faint_bg_color = egui::Color32::from_rgb(224, 232, 245);
            visuals.extreme_bg_color = egui::Color32::from_rgb(213, 224, 241);
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(203, 217, 241);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(61, 117, 234);
            visuals.selection.bg_fill = egui::Color32::from_rgb(145, 179, 240);
            ctx.set_visuals(visuals);
        }
    }

    let mut style = (*ctx.style()).clone();
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
        format!("[root] {}", node.name)
    } else if node.children.is_empty() {
        format!("[session] {}", node.name)
    } else {
        format!("[group] {}", node.name)
    };

    if node.children.is_empty() {
        let is_selected = selected_path.as_deref() == Some(full_path);
        let response = ui.selectable_label(is_selected, label);
        if response.clicked() {
            *selected_path = Some(full_path.to_string());
        }
        if response.double_clicked() {
            *selected_path = Some(full_path.to_string());
            *open_now = Some(full_path.to_string());
        }
        return true;
    }

    let header = egui::CollapsingHeader::new(label)
        .default_open(depth < 2)
        .id_salt(full_path);
    header.show(ui, |ui| {
        let is_selected = selected_path.as_deref() == Some(full_path);
        if ui.selectable_label(is_selected, "Select group").clicked() {
            *selected_path = Some(full_path.to_string());
        }

        for (child_path, child) in child_entries {
            let _ = render_session_node(ui, child, depth + 1, selected_path, &child_path, filter, open_now);
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
    node.children.iter().any(|child| node_matches_filter(child, filter_lc))
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
