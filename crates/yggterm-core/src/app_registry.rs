//! The libyggterm APP REGISTRY: the launcher family's single source of truth.
//!
//! yggterm's titlebar `+` menu, the cwd-tree context menu, and the start page's
//! "New …" buttons used to hardcode one arm per thing you could launch. That is
//! app chrome in the platform — the same anti-pattern `RightPanelMode::Vault`
//! and `::AppSidebar` were deleted for. A libyggterm app is not a special case
//! the shell knows about; it is an entry in a registry the shell reads.
//!
//! ynpm writes a normalized manifest to **the app's host** when its package is
//! installed or upgraded:
//!
//! ```text
//! ~/.yggterm/apps/<name>.json
//! { "name": "ychrome", "label": "Ychrome", "icon": "🌐",
//!   "binary": "/home/user/.local/bin/ychrome",
//!   "verbs": [ { "id": "new", "label": "New Ychrome", "args": [] } ] }
//! ```
//!
//! The host's daemon scans that directory and validates that `binary` still
//! resolves. A missing binary is omitted from the live menu, but the daemon
//! does not delete the registration; `ynpm remove` owns package cleanup.
//!
//! Because the manifest lives on the host the app runs on, the menus are
//! naturally per-host: ychrome installed on `dev` but not `guihost` means "New
//! Ychrome" appears on `dev` viewports only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The directory ynpm writes normalized app manifests into, under the host's
/// yggterm home.
pub const APP_REGISTRY_DIRNAME: &str = "apps";

/// One thing an app can launch. `args` are passed to `binary` verbatim; the cwd
/// comes from wherever the user invoked the verb (a cwd-tree row, the active
/// session), never from the manifest — a launcher entry describes the app, not
/// the place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppVerb {
    /// Stable id, unique within the app. Rides menu callbacks.
    pub id: String,
    /// What the menu item says, e.g. "New Ychrome".
    pub label: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The ALT+ KeyTip letter this verb *wants* in the New… scope (spec §10). A
    /// single character; the shell's resolver may deny it (reserved, taken, or
    /// folded into a numbered group). Empty/absent = let the ladder choose.
    #[serde(default)]
    pub keytip: String,
    /// ⭐ Does this verb make sense as a SESSION ROW?
    ///
    /// The row context menu spawns a session and puts a row in the sidebar for
    /// it. That is right for "New Ychrome" and wrong for a terminal-invoked
    /// dashboard or a fleet booter: the user gets a row for something that was
    /// never a session, and the owner asked for those to go.
    ///
    /// ⛔ **The APP declares it, because yggterm cannot know.** The verbs to
    /// leave out belong to other apps' manifests, so the only alternatives were
    /// for yggterm to hardcode another app's name — the precise anti-pattern the
    /// libyggterm contract exists to prevent — or to be told. This is being
    /// told.
    ///
    /// Defaults to TRUE, so every manifest written before this field existed
    /// keeps every verb it had. An opt-out cannot silently empty a menu; an
    /// opt-in would have emptied every menu at once.
    ///
    /// ⚠ It governs the ROW menu only. The titlebar `+` and the start page keep
    /// offering everything: "open my dashboard" is a reasonable thing to offer
    /// there and a meaningless thing to spawn a row for, and that difference is
    /// the whole point of the flag.
    #[serde(default = "default_row_spawn")]
    pub row_spawn: bool,
}

/// A verb is a row spawn unless it says otherwise — see [`AppVerb::row_spawn`].
fn default_row_spawn() -> bool {
    true
}

/// ⛔ HAND-WRITTEN, and it must stay that way. `#[derive(Default)]` would make
/// `row_spawn` FALSE while `#[serde(default)]` makes it true, so a verb built in
/// code and the same verb read from disk would disagree about whether it belongs
/// in the row menu — and the code-built one would silently vanish from it.
impl Default for AppVerb {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            args: Vec::new(),
            keytip: String::new(),
            row_spawn: default_row_spawn(),
        }
    }
}

/// Where an installed app wants a launcher entry. This is package metadata,
/// not app runtime state: ynpm writes it when the package is installed and
/// removes it when the package is removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppContextMenu {
    /// `false` keeps an app in the titlebar/start page while excluding it from
    /// row context menus. This is how a dashboard can be launchable without
    /// pretending it is useful for every selected file/folder.
    #[serde(default)]
    pub enabled: bool,
    /// Context names understood by yggterm, currently `workspace` and
    /// `session`. Empty means every context, which is useful for a small app
    /// whose one verb is meaningful everywhere.
    #[serde(default)]
    pub contexts: Vec<String>,
    /// Verb ids to expose. Empty means all verbs whose `row_spawn` is true.
    #[serde(default)]
    pub verbs: Vec<String>,
}

impl AppContextMenu {
    pub fn allows(&self, context: &str, verb_id: &str) -> bool {
        self.enabled
            && (self.contexts.is_empty() || self.contexts.iter().any(|item| item == context))
            && (self.verbs.is_empty() || self.verbs.iter().any(|item| item == verb_id))
    }
}

/// The stable `package.json.yggterm` extension understood by ynpm. Keeping
/// this typed at the platform boundary prevents each app and the package
/// manager from inventing slightly different JSON for the same launcher.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct YggtermPackageMetadata {
    #[serde(default)]
    pub app: Option<YggtermAppMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct YggtermAppMetadata {
    /// Stable app/manifest name. Defaults to the package basename when absent.
    #[serde(default)]
    pub name: Option<String>,
    /// Package-relative executable bin name. If absent, ynpm uses the first
    /// declared bin in sorted order.
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub keytip: String,
    #[serde(default)]
    pub verbs: Vec<AppVerb>,
    /// Accept both the JSON spelling used by this project and the usual npm
    /// camel-case spelling, while serializing the documented snake-case form.
    #[serde(default, alias = "contextMenu")]
    pub context_menu: Option<AppContextMenu>,
}

/// One installed libyggterm app, as its manifest declares it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppManifest {
    /// Registry key. Must equal the manifest's file stem, so an app cannot
    /// squat another app's entry by writing a differently-named file.
    pub name: String,
    #[serde(default)]
    pub label: String,
    /// A single glyph shown beside the menu entry. Optional.
    #[serde(default)]
    pub icon: String,
    /// Absolute path to the executable ON THIS HOST. Absolute on purpose: a bare
    /// name would have to be resolved against a PATH that a non-interactive ssh
    /// session does not have (the trap that makes `ychrome` "not found" over
    /// `terminal send`).
    pub binary: String,
    #[serde(default)]
    pub verbs: Vec<AppVerb>,
    /// The ALT+ KeyTip letter this app wants where it appears as a single node
    /// (spec §10). Same rules as [`AppVerb::keytip`]. Empty/absent = ladder.
    #[serde(default)]
    pub keytip: String,
    /// Package-declared row context-menu policy. `None` is deliberately not
    /// treated as enabled: old runtime-written manifests may still provide
    /// titlebar/start-page entries, but only an ynpm package declaration earns
    /// a row context-menu slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_menu: Option<AppContextMenu>,
}

impl AppManifest {
    /// What the menus should call this app. Falls back to the registry key.
    pub fn display_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.name
        } else {
            &self.label
        }
    }

    /// The shell command a verb launches. Quoting is deliberate: a path with a
    /// space must survive being typed into a PTY.
    pub fn command_for(&self, verb: &AppVerb) -> String {
        let mut command = shell_quote(&self.binary);
        for arg in &verb.args {
            command.push(' ');
            command.push_str(&shell_quote(arg));
        }
        command
    }

    pub fn context_menu_allows(&self, context: &str, verb_id: &str) -> bool {
        self.context_menu
            .as_ref()
            .is_some_and(|menu| menu.allows(context, verb_id))
    }

    /// A manifest the shell will act on: a usable key, a usable binary, and at
    /// least one verb to show. Anything else is a half-written file, and a menu
    /// entry that launches nothing is worse than no entry.
    fn is_usable(&self, file_stem: &str) -> bool {
        !self.name.trim().is_empty()
            && self.name == file_stem
            && is_plain_name(&self.name)
            && !self.binary.trim().is_empty()
            && !self.verbs.is_empty()
            && self.verbs.iter().all(|verb| !verb.id.trim().is_empty())
    }
}

/// Single-quote for `sh`, the way a shell must: `'` closes, escapes, reopens.
fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._-/:=".contains(ch))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// A registry key is a bare filename component. It becomes a path (`apps/<name>.json`)
/// and rides menu callbacks, so `../` and `/` are rejected outright.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(std::path::MAIN_SEPARATOR)
}

pub fn app_registry_dir(yggterm_home: &Path) -> PathBuf {
    yggterm_home.join(APP_REGISTRY_DIRNAME)
}

/// Whether an app's declared binary still exists and is executable on this host.
fn binary_resolves(binary: &str) -> bool {
    let path = Path::new(binary);
    if !path.is_absolute() {
        return false;
    }
    match std::fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        Err(_) => false,
    }
}

/// Read the host's ynpm-owned app registry. A missing binary is not deleted
/// here: package ownership belongs to ynpm, not a daemon scan or an app's
/// first run. The missing entry is simply omitted from menus until ynpm repairs
/// or removes the package.
///
/// `ynpm remove` is the uninstall story; a package manager action removes its
/// registration, while a missing binary is only reported here. Deterministic
/// order (sorted by name) — menus must not reshuffle between scans.
///
/// Returns `(live apps, unresolvable names)`; the second list is diagnostic
/// compatibility for callers and no longer authorizes file deletion.
pub fn scan_app_registry(yggterm_home: &Path) -> (Vec<AppManifest>, Vec<String>) {
    let dir = app_registry_dir(yggterm_home);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut apps = Vec::new();
    let mut missing = Vec::new();
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in paths {
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let manifest = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<AppManifest>(&raw).ok())
            .filter(|manifest| manifest.is_usable(stem));
        let Some(manifest) = manifest else {
            // A malformed or mislabelled manifest is IGNORED, never deleted: it
            // may be a newer yggterm's format, and destroying another version's
            // file is not ours to do.
            continue;
        };
        if binary_resolves(&manifest.binary) {
            apps.push(manifest);
        } else {
            missing.push(manifest.name);
        }
    }
    (apps, missing)
}

/// Write (or refresh) this app's manifest on this host. `ynpm` is the package
/// owner and the normal caller; this helper remains the small file writer used
/// by ynpm and compatibility tooling. An app process must not call it on
/// startup: runtime use is not installation evidence.
pub fn write_app_manifest(yggterm_home: &Path, manifest: &AppManifest) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_plain_name(&manifest.name),
        "app name must be a plain name, not a path: {:?}",
        manifest.name
    );
    let dir = app_registry_dir(yggterm_home);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", manifest.name));
    std::fs::write(&path, serde_json::to_string_pretty(manifest)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str, binary: &str) -> AppManifest {
        AppManifest {
            name: name.to_string(),
            label: "Ychrome".to_string(),
            icon: "🌐".to_string(),
            binary: binary.to_string(),
            verbs: vec![AppVerb {
                id: "new".to_string(),
                label: "New Ychrome".to_string(),
                args: Vec::new(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ygg-registry-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(APP_REGISTRY_DIRNAME)).unwrap();
        dir
    }

    #[test]
    fn a_live_app_is_scanned_and_kept() {
        let home = tempdir("live");
        // Any executable on this host will do; the scan only checks resolvability.
        write_app_manifest(&home, &manifest("ychrome", "/bin/sh")).unwrap();
        let (apps, missing) = scan_app_registry(&home);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "ychrome");
        assert!(missing.is_empty());
        assert!(home.join("apps/ychrome.json").is_file());
    }

    // ynpm owns cleanup: a missing binary disappears from menus, but the scan
    // does not delete the package's registration file.
    #[test]
    fn a_manifest_whose_binary_vanished_is_hidden_but_not_deleted_by_the_scan() {
        let home = tempdir("prune");
        write_app_manifest(&home, &manifest("ghost", "/nonexistent/ghost-binary")).unwrap();
        let (apps, missing) = scan_app_registry(&home);
        assert!(apps.is_empty(), "offered a menu entry for a missing binary");
        assert_eq!(missing, vec!["ghost".to_string()]);
        assert!(
            home.join("apps/ghost.json").exists(),
            "the scan must not own package removal"
        );
    }

    // A non-executable file is not a launchable app, even though it exists.
    #[test]
    fn a_non_executable_binary_does_not_resolve() {
        let home = tempdir("noexec");
        let data = home.join("not-a-program");
        std::fs::write(&data, "text").unwrap();
        write_app_manifest(&home, &manifest("fake", data.to_str().unwrap())).unwrap();
        let (apps, missing) = scan_app_registry(&home);
        assert!(apps.is_empty());
        assert_eq!(missing, vec!["fake".to_string()]);
    }

    // A relative binary would be resolved against a PATH a non-interactive ssh
    // session does not have. Require absolute, or the verb fails only on remote.
    #[test]
    fn a_relative_binary_never_resolves() {
        assert!(!binary_resolves("sh"));
        assert!(!binary_resolves("./ychrome"));
        assert!(binary_resolves("/bin/sh"));
    }

    // The file stem is the key. A manifest claiming another app's name is
    // ignored, so `evil.json` cannot overwrite `ychrome`'s entry.
    #[test]
    fn a_manifest_cannot_squat_another_apps_name() {
        let home = tempdir("squat");
        let mut evil = manifest("ychrome", "/bin/sh");
        evil.name = "ychrome".to_string();
        std::fs::write(
            home.join("apps/evil.json"),
            serde_json::to_string(&evil).unwrap(),
        )
        .unwrap();
        let (apps, missing) = scan_app_registry(&home);
        assert!(apps.is_empty(), "a mislabelled manifest was honoured");
        // Ignored, NOT deleted: it is not ours to destroy.
        assert!(missing.is_empty());
        assert!(home.join("apps/evil.json").is_file());
    }

    #[test]
    fn a_path_traversing_name_is_refused() {
        let home = tempdir("traverse");
        let bad = manifest("../../evil", "/bin/sh");
        assert!(write_app_manifest(&home, &bad).is_err());
    }

    // An app with no verbs has nothing to put in a menu.
    #[test]
    fn a_verbless_manifest_is_ignored() {
        let home = tempdir("verbless");
        let mut app = manifest("quiet", "/bin/sh");
        app.verbs.clear();
        write_app_manifest(&home, &app).unwrap();
        assert!(scan_app_registry(&home).0.is_empty());
    }

    // Menus must not reshuffle between scans.
    #[test]
    fn the_scan_is_ordered_by_name() {
        let home = tempdir("order");
        for name in ["zed", "alpha", "middle"] {
            write_app_manifest(&home, &manifest(name, "/bin/sh")).unwrap();
        }
        let names: Vec<String> = scan_app_registry(&home)
            .0
            .into_iter()
            .map(|app| app.name)
            .collect();
        assert_eq!(names, vec!["alpha", "middle", "zed"]);
    }

    // The command is typed into a PTY, so a path with a space must survive.
    #[test]
    fn a_command_quotes_what_a_shell_would_split() {
        let app = AppManifest {
            name: "x".into(),
            label: String::new(),
            icon: String::new(),
            binary: "/opt/my apps/ychrome".into(),
            verbs: vec![AppVerb {
                id: "new".into(),
                label: "New".into(),
                args: vec!["--profile".into(), "work profile".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            app.command_for(&app.verbs[0]),
            "'/opt/my apps/ychrome' --profile 'work profile'"
        );
        assert_eq!(app.display_label(), "x", "empty label must fall back");
    }

    #[test]
    fn a_quote_in_an_argument_cannot_break_out() {
        let quoted = shell_quote("it's");
        assert_eq!(quoted, r"'it'\''s'");
    }

    #[test]
    fn context_menu_requires_package_opt_in_and_honours_context_and_verb_filters() {
        let mut app = manifest("example", "/bin/sh");
        assert!(!app.context_menu_allows("workspace", "new"));
        app.context_menu = Some(AppContextMenu {
            enabled: true,
            contexts: vec!["session".to_string()],
            verbs: vec!["new".to_string()],
        });
        assert!(app.context_menu_allows("session", "new"));
        assert!(!app.context_menu_allows("workspace", "new"));
        assert!(!app.context_menu_allows("session", "other"));
    }
}
