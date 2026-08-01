//! IMPORT — a decade of browsing, out of the browsers that hold it.
//!
//! The user's actual goal for collections: *"all my history from all my
//! browsers (chromium based brave, vivaldi, chromium, google chrome, helium)
//! and firefox based profiles"*. See `ychrome/docs/collections.md` §Import.
//!
//! Two shapes come out, and they are not interchangeable:
//!
//! * **History is a timeline** → visits, merged into the profile's
//!   `history.jsonl` ([`crate::web_history`]). Not one giant collection nobody
//!   can read.
//! * **Bookmarks are folders** → one collection, with its folder tree
//!   preserved as heading depth ([`crate::web_collection`]).
//!
//! # The three traps
//!
//! Each of these produces PLAUSIBLE GARBAGE rather than an error, which is why
//! each has a lock in the tests below rather than a comment.
//!
//! 1. **The epoch.** Chromium's timestamps are microseconds since
//!    **1601-01-01** (the Windows FILETIME epoch it inherited); Firefox's are
//!    microseconds since **1970-01-01**. Read one as the other and a decade of
//!    history lands in the wrong century — and it looks fine, because every row
//!    is self-consistent. [`chromium_time_to_unix_ms`] and
//!    [`firefox_time_to_unix_ms`] are the only two converters, and both refuse
//!    anything outside [`PLAUSIBLE_MIN_MS`]..=[`PLAUSIBLE_MAX_MS`] — which is
//!    exactly what a swapped epoch produces, in both directions.
//! 2. **The database is locked while that browser is running.** Every read here
//!    goes through [`SqliteSnapshot`]: copy the file (and its `-wal`/`-shm`/
//!    `-journal` sidecars) to a private temp directory, open THE COPY read-only,
//!    delete it on drop. The user's live profile is never opened read-write, and
//!    is never opened at all except by `fs::copy`.
//! 3. **Import must be idempotent.** Re-importing the same profile must not
//!    double anything: history dedupes on `(url, visit_time)` in
//!    [`crate::web_history::merge_web_visits`], bookmarks dedupe on
//!    `(folder path, url)` in [`merge_bookmarks_into_collection`], with
//!    [`Collection::contains_url`] as the primitive underneath.
//!
//! # What this does NOT do
//!
//! No shell-out to `sqlite3` — `rusqlite` (bundled) is already a dependency, so
//! this is in-process. No writes to the source profile, ever. No Session Buddy
//! JSON yet (spec §Import lists it; I5 covers Chromium and Firefox).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::web_collection::{Block, Collection, Item};
use crate::web_collection_store::{
    CollectionKind, CollectionStore, NewCollection, build_collection, slugify, touch,
};
use crate::web_history::{
    HistoryWriteReport, WebHistoryEntry, merge_web_visits, web_history_path_in,
    web_history_url_is_page,
};

// ---------------------------------------------------------------------------
// The clock trap
// ---------------------------------------------------------------------------

/// Microseconds between 1601-01-01 and 1970-01-01 — 11,644,473,600 seconds.
///
/// Chromium stores `visit_time` as microseconds since 1601 because Windows
/// FILETIME does, and it kept that on every platform. This constant is the
/// entire difference between "my history from 2016" and "my history from 1616".
pub const CHROMIUM_EPOCH_OFFSET_MICROS: i64 = 11_644_473_600_000_000;

/// 1990-01-01T00:00:00Z. Nothing older than this is a real browsing visit.
pub const PLAUSIBLE_MIN_MS: i64 = 631_152_000_000;
/// 2100-01-01T00:00:00Z.
pub const PLAUSIBLE_MAX_MS: i64 = 4_102_444_800_000;

/// A converted instant, or `None` when it cannot be one.
///
/// Fixed bounds rather than "now", deliberately: a window that moved with the
/// clock would make the same database import differently on two machines, and
/// this crate does not get to be non-deterministic. The window is wide enough
/// to hold any real history and narrow enough that a swapped epoch always falls
/// outside it — see `a_swapped_epoch_is_refused_in_both_directions`.
fn plausible_ms(ms: i64) -> Option<i64> {
    (PLAUSIBLE_MIN_MS..=PLAUSIBLE_MAX_MS)
        .contains(&ms)
        .then_some(ms)
}

/// Chromium `visit_time` / `last_visit_time` → Unix milliseconds.
pub fn chromium_time_to_unix_ms(raw: i64) -> Option<i64> {
    plausible_ms(raw.checked_sub(CHROMIUM_EPOCH_OFFSET_MICROS)? / 1000)
}

/// Firefox `visit_date` / `last_visit_date` / `dateAdded` → Unix milliseconds.
pub fn firefox_time_to_unix_ms(raw: i64) -> Option<i64> {
    plausible_ms(raw / 1000)
}

/// A UTC calendar day (`YYYY-MM-DD`) for an instant, for the import report.
///
/// UTC because that is what the history page already groups by, so the day this
/// reports and the day the user sees are the same day.
pub fn utc_day(ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp((ms / 1000) as i64)
        .map(|at| at.date().to_string())
        .unwrap_or_else(|_| "invalid".to_string())
}

// ---------------------------------------------------------------------------
// The lock trap: copy, then open the copy read-only
// ---------------------------------------------------------------------------

/// A private, throwaway copy of a SQLite database, opened READ-ONLY.
///
/// Chrome holds `History` open for the whole session and Firefox holds
/// `places.sqlite` open with a WAL; opening either in place would fail, block,
/// or (worst) leave a journal behind in the user's profile. So: copy first,
/// read the copy, delete it. The sidecars come along because the newest visits
/// live in the `-wal`, not in the main file — a copy without it silently loses
/// the last session's browsing, which is exactly the class of failure this
/// module is written to refuse.
pub struct SqliteSnapshot {
    dir: PathBuf,
    db: PathBuf,
}

/// Sidecars SQLite may keep beside a database. `-wal` carries committed pages
/// that have not been checkpointed; `-journal` carries a rollback journal.
const SQLITE_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl SqliteSnapshot {
    /// Copy `source` (and its sidecars) into a private temp directory.
    pub fn take(source: &Path) -> Result<Self> {
        let file_name = source
            .file_name()
            .ok_or_else(|| anyhow!("{} is not a file", source.display()))?
            .to_owned();
        let dir = std::env::temp_dir().join(format!(
            "yggterm-browser-import-{}-{}",
            std::process::id(),
            SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating import scratch {}", dir.display()))?;
        let db = dir.join(&file_name);
        std::fs::copy(source, &db)
            .with_context(|| format!("copying {} for read-only import", source.display()))?;
        make_owner_writable(&db)?;
        for suffix in SQLITE_SIDECARS {
            let mut sidecar_name = file_name.clone();
            sidecar_name.push(suffix);
            let sidecar = source.with_file_name(&sidecar_name);
            if sidecar.exists() {
                let target = dir.join(&sidecar_name);
                std::fs::copy(&sidecar, &target)
                    .with_context(|| format!("copying {}", sidecar.display()))?;
                make_owner_writable(&target)?;
            }
        }
        Ok(Self { dir, db })
    }

    /// The copy's path. Never the source's.
    pub fn path(&self) -> &Path {
        &self.db
    }

    /// Open the COPY read-only. `SQLITE_OPEN_READ_ONLY` is not decoration: it
    /// is the assertion that nothing in this module can write to a database,
    /// including the one it owns.
    pub fn open_read_only(&self) -> Result<Connection> {
        Connection::open_with_flags(&self.db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening snapshot {} read-only", self.db.display()))
    }
}

impl Drop for SqliteSnapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Make our copy writable by us even when the source was not.
///
/// A read-only `History` (or a profile directory the user has locked down)
/// copies to a read-only file, and SQLite cannot build the `-shm` a WAL
/// database needs beside it. The permission belongs to the COPY; the source is
/// never touched.
#[cfg(unix)]
fn make_owner_writable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn make_owner_writable(path: &Path) -> std::io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(path, perms)
}

// ---------------------------------------------------------------------------
// Which browsers, and where they keep their profiles
// ---------------------------------------------------------------------------

/// The two database shapes. Every browser named in the spec is one of these —
/// five Chromium forks and the Firefox family — which is why the readers are
/// per-FAMILY and the browser list is data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserFamily {
    Chromium,
    Firefox,
}

/// A browser this build knows how to import from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserSource {
    /// The `--browser` token. Stable; it appears in `imported_from`.
    pub id: &'static str,
    pub display_name: &'static str,
    pub family: BrowserFamily,
    /// Home-relative user-data directories, in preference order. The FIRST one
    /// that exists is the one discovery uses; the rest cover flatpak and snap
    /// layouts of the same browser.
    pub linux_dirs: &'static [&'static str],
    /// Home-relative user-data directories on macOS.
    pub macos_dirs: &'static [&'static str],
    /// `%LOCALAPPDATA%`-relative user-data directories on Windows.
    pub windows_dirs: &'static [&'static str],
}

/// THE browser table. Adding a browser is a row here — never a new reader,
/// because the family already decided how to read it.
pub const BROWSER_SOURCES: &[BrowserSource] = &[
    BrowserSource {
        id: "chrome",
        display_name: "Google Chrome",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/google-chrome",
            ".var/app/com.google.Chrome/config/google-chrome",
        ],
        macos_dirs: &["Library/Application Support/Google/Chrome"],
        windows_dirs: &["Google/Chrome/User Data"],
    },
    BrowserSource {
        id: "brave",
        display_name: "Brave",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/BraveSoftware/Brave-Browser",
            ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser",
        ],
        macos_dirs: &["Library/Application Support/BraveSoftware/Brave-Browser"],
        windows_dirs: &["BraveSoftware/Brave-Browser/User Data"],
    },
    BrowserSource {
        id: "vivaldi",
        display_name: "Vivaldi",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/vivaldi",
            ".var/app/com.vivaldi.Vivaldi/config/vivaldi",
        ],
        macos_dirs: &["Library/Application Support/Vivaldi"],
        windows_dirs: &["Vivaldi/User Data"],
    },
    BrowserSource {
        id: "chromium",
        display_name: "Chromium",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/chromium",
            ".var/app/org.chromium.Chromium/config/chromium",
            "snap/chromium/common/chromium",
        ],
        macos_dirs: &["Library/Application Support/Chromium"],
        windows_dirs: &["Chromium/User Data"],
    },
    BrowserSource {
        id: "helium",
        display_name: "Helium",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/net.imput.helium",
            ".config/helium",
            ".var/app/net.imput.helium/config/net.imput.helium",
        ],
        macos_dirs: &["Library/Application Support/net.imput.helium"],
        windows_dirs: &["net.imput.helium/User Data"],
    },
    BrowserSource {
        id: "edge",
        display_name: "Microsoft Edge",
        family: BrowserFamily::Chromium,
        linux_dirs: &[
            ".config/microsoft-edge",
            ".config/microsoft-edge-dev",
            ".var/app/com.microsoft.Edge/config/microsoft-edge",
        ],
        macos_dirs: &["Library/Application Support/Microsoft Edge"],
        windows_dirs: &["Microsoft/Edge/User Data"],
    },
    BrowserSource {
        id: "firefox",
        display_name: "Firefox",
        family: BrowserFamily::Firefox,
        linux_dirs: &[
            ".mozilla/firefox",
            ".var/app/org.mozilla.firefox/.mozilla/firefox",
            "snap/firefox/common/.mozilla/firefox",
        ],
        macos_dirs: &["Library/Application Support/Firefox"],
        windows_dirs: &["Mozilla/Firefox"],
    },
    BrowserSource {
        id: "librewolf",
        display_name: "LibreWolf",
        family: BrowserFamily::Firefox,
        linux_dirs: &[
            ".librewolf",
            ".var/app/io.gitlab.librewolf-community/.librewolf",
        ],
        macos_dirs: &["Library/Application Support/LibreWolf"],
        windows_dirs: &["librewolf"],
    },
    BrowserSource {
        id: "zen",
        display_name: "Zen Browser",
        family: BrowserFamily::Firefox,
        linux_dirs: &[".zen", ".var/app/app.zen_browser.zen/.zen"],
        macos_dirs: &["Library/Application Support/zen"],
        windows_dirs: &["zen/Profiles"],
    },
    BrowserSource {
        id: "waterfox",
        display_name: "Waterfox",
        family: BrowserFamily::Firefox,
        linux_dirs: &[".waterfox", ".var/app/net.waterfox.waterfox/.waterfox"],
        macos_dirs: &["Library/Application Support/Waterfox"],
        windows_dirs: &["Waterfox"],
    },
];

/// A browser by its `--browser` token.
pub fn browser_source(id: &str) -> Option<&'static BrowserSource> {
    BROWSER_SOURCES
        .iter()
        .find(|source| source.id.eq_ignore_ascii_case(id.trim()))
}

impl BrowserSource {
    /// Every user-data directory this browser could be using on this platform,
    /// resolved against `home` (and `%LOCALAPPDATA%` on Windows).
    pub fn user_data_candidates(&self, home: &Path) -> Vec<PathBuf> {
        let relative: &[&str] = if cfg!(target_os = "macos") {
            self.macos_dirs
        } else if cfg!(target_os = "windows") {
            self.windows_dirs
        } else {
            self.linux_dirs
        };
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.to_path_buf())
        } else {
            home.to_path_buf()
        };
        relative.iter().map(|tail| base.join(tail)).collect()
    }

    /// The first candidate that exists on disk, if any.
    pub fn installed_user_data_dir(&self, home: &Path) -> Option<PathBuf> {
        self.user_data_candidates(home)
            .into_iter()
            .find(|path| path.is_dir())
    }
}

/// One profile inside one browser — what the user picks between.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrowserProfile {
    pub browser_id: String,
    pub family: BrowserFamily,
    /// The on-disk directory name (`Default`, `Profile 1`,
    /// `0gshn1os.Default Profile`). This is the identity: it is what
    /// `--source-profile` names and what lands in `imported_from`.
    pub dir_name: String,
    /// What the browser calls it (`Local State`'s `info_cache` name, or
    /// `profiles.ini`'s `Name=`). Decoration — never identity.
    pub display_name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

impl BrowserProfile {
    /// The history database, if this profile has one.
    pub fn history_db(&self) -> Option<PathBuf> {
        let path = match self.family {
            BrowserFamily::Chromium => self.path.join("History"),
            BrowserFamily::Firefox => self.path.join("places.sqlite"),
        };
        path.is_file().then_some(path)
    }

    /// Where the bookmarks are. Chromium keeps them in a JSON file; Firefox
    /// keeps them in the same database as the history.
    pub fn bookmarks_source(&self) -> Option<PathBuf> {
        let path = match self.family {
            BrowserFamily::Chromium => self.path.join("Bookmarks"),
            BrowserFamily::Firefox => self.path.join("places.sqlite"),
        };
        path.is_file().then_some(path)
    }
}

/// An installed browser and the profiles inside it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredBrowser {
    pub browser_id: String,
    pub display_name: String,
    pub family: BrowserFamily,
    pub user_data_dir: PathBuf,
    pub profiles: Vec<BrowserProfile>,
}

/// Every browser in [`BROWSER_SOURCES`] that is actually installed under
/// `home`, with its profiles. Table order, so the answer is the same every run.
pub fn discover_browsers(home: &Path) -> Vec<DiscoveredBrowser> {
    BROWSER_SOURCES
        .iter()
        .filter_map(|source| {
            let user_data_dir = source.installed_user_data_dir(home)?;
            let profiles = discover_profiles(source, &user_data_dir);
            (!profiles.is_empty()).then(|| DiscoveredBrowser {
                browser_id: source.id.to_string(),
                display_name: source.display_name.to_string(),
                family: source.family,
                user_data_dir,
                profiles,
            })
        })
        .collect()
}

/// The profiles inside one user-data directory.
pub fn discover_profiles(source: &BrowserSource, user_data_dir: &Path) -> Vec<BrowserProfile> {
    match source.family {
        BrowserFamily::Chromium => discover_chromium_profiles(source.id, user_data_dir),
        BrowserFamily::Firefox => discover_firefox_profiles(source.id, user_data_dir),
    }
}

/// Directories inside a Chromium user-data dir that are not user profiles.
const CHROMIUM_NON_PROFILE_DIRS: [&str; 3] = ["System Profile", "Guest Profile", "Crash Reports"];

/// Chromium profiles: `Default`, `Profile 1`, `Profile 2`, … plus whatever else
/// carries a `History` or `Preferences`.
///
/// Ordered `Default` first, then `Profile N` NUMERICALLY (so `Profile 10` does
/// not sort before `Profile 2`), then anything else alphabetically. A picker
/// whose order depended on `read_dir` would shuffle between runs.
pub fn discover_chromium_profiles(browser_id: &str, user_data_dir: &Path) -> Vec<BrowserProfile> {
    let names = chromium_profile_display_names(user_data_dir);
    let Ok(entries) = std::fs::read_dir(user_data_dir) else {
        return Vec::new();
    };
    let mut profiles: Vec<BrowserProfile> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if CHROMIUM_NON_PROFILE_DIRS.contains(&dir_name.as_str()) {
                return None;
            }
            let path = entry.path();
            if !path.join("History").is_file() && !path.join("Preferences").is_file() {
                return None;
            }
            Some(BrowserProfile {
                browser_id: browser_id.to_string(),
                family: BrowserFamily::Chromium,
                display_name: names
                    .get(&dir_name)
                    .cloned()
                    .unwrap_or_else(|| dir_name.clone()),
                is_default: dir_name == "Default",
                dir_name,
                path,
            })
        })
        .collect();
    profiles.sort_by_key(|profile| chromium_profile_sort_key(&profile.dir_name));
    profiles
}

/// `Default` first, then `Profile <n>` by n, then the rest by name.
fn chromium_profile_sort_key(dir_name: &str) -> (u8, u64, String) {
    if dir_name == "Default" {
        return (0, 0, String::new());
    }
    if let Some(number) = dir_name
        .strip_prefix("Profile ")
        .and_then(|tail| tail.trim().parse::<u64>().ok())
    {
        return (1, number, String::new());
    }
    (2, 0, dir_name.to_string())
}

/// `Local State` → `profile.info_cache.<dir>.name`: the names the browser's own
/// profile menu shows. Missing or unreadable is not an error — the directory
/// name is a perfectly good label.
fn chromium_profile_display_names(user_data_dir: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Ok(raw) = std::fs::read_to_string(user_data_dir.join("Local State")) else {
        return names;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return names;
    };
    let Some(cache) = value
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(Value::as_object)
    else {
        return names;
    };
    for (dir_name, info) in cache {
        if let Some(name) = info
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            names.insert(dir_name.clone(), name.to_string());
        }
    }
    names
}

/// Firefox profiles, from `profiles.ini`.
///
/// Two different keys claim the word "default" and they routinely disagree:
/// a `[ProfileN]` section can carry the legacy `Default=1`, while an
/// `[InstallXXXX]` section carries `Default=<path>` for the profile that
/// install actually launches. The install's answer wins when there is one,
/// because that is the profile the user's browser opens.
pub fn discover_firefox_profiles(browser_id: &str, root: &Path) -> Vec<BrowserProfile> {
    let Ok(raw) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let sections = parse_ini(&raw);
    let install_default = sections
        .iter()
        .filter(|(name, _)| name.starts_with("Install"))
        .find_map(|(_, keys)| keys.get("Default").cloned());

    let mut profiles: Vec<(u64, BrowserProfile)> = Vec::new();
    for (name, keys) in &sections {
        let Some(index) = name
            .strip_prefix("Profile")
            .and_then(|tail| tail.trim().parse::<u64>().ok())
        else {
            continue;
        };
        let Some(relative_path) = keys.get("Path") else {
            continue;
        };
        let is_relative = keys.get("IsRelative").map(String::as_str) != Some("0");
        let path = if is_relative {
            root.join(relative_path)
        } else {
            PathBuf::from(relative_path)
        };
        let is_default = match &install_default {
            Some(default_path) => default_path == relative_path,
            None => keys.get("Default").map(String::as_str) == Some("1"),
        };
        profiles.push((
            index,
            BrowserProfile {
                browser_id: browser_id.to_string(),
                family: BrowserFamily::Firefox,
                dir_name: relative_path.clone(),
                display_name: keys
                    .get("Name")
                    .cloned()
                    .unwrap_or_else(|| relative_path.clone()),
                path,
                is_default,
            },
        ));
    }
    // By section index, not by file order: `profiles.ini` routinely lists
    // `[Profile1]` before `[Profile0]`, and a picker must not reshuffle.
    profiles.sort_by_key(|(index, _)| *index);
    profiles.into_iter().map(|(_, profile)| profile).collect()
}

/// A minimal INI reader: `[Section]` then `key=value`, comments and blanks
/// skipped. Section order is preserved.
fn parse_ini(raw: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.trim().to_string(), HashMap::new()));
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && let Some((_, keys)) = sections.last_mut()
        {
            keys.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    sections
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// What came out of a history database, and what did not.
#[derive(Debug, Clone, Default)]
pub struct VisitHarvest {
    pub visits: Vec<WebHistoryEntry>,
    /// Rows the query returned, before any filtering.
    pub rows_read: usize,
    /// Rows whose timestamp could not be a real instant — the epoch guard.
    pub rejected_timestamp: usize,
    /// Rows for URLs that are not pages (`chrome://`, `about:`, `file://`).
    pub skipped_not_page: usize,
    /// Visits recovered from the URL table because their per-visit rows were
    /// gone. Chromium expires `visits` at ~90 days while `urls` keeps
    /// `last_visit_time`, so for a decade-old profile this is most of what is
    /// left.
    pub recovered_from_urls_table: usize,
}

impl VisitHarvest {
    /// `converted_ms` is the OUTPUT of one of the two epoch converters — the
    /// only way a timestamp may reach the journal. `None` means that converter
    /// refused it, which is the epoch guard doing its job.
    fn push(
        &mut self,
        seen: &mut HashSet<(String, u64)>,
        url: String,
        title: String,
        converted_ms: Option<i64>,
    ) -> bool {
        self.rows_read += 1;
        let Some(ts_ms) = converted_ms else {
            self.rejected_timestamp += 1;
            return false;
        };
        // The converters bound their output to a plausible window, so this cast
        // cannot wrap: everything they return is positive.
        let ts_ms = ts_ms as u64;
        if !web_history_url_is_page(&url) {
            self.skipped_not_page += 1;
            return false;
        }
        if !seen.insert((url.clone(), ts_ms)) {
            return false;
        }
        self.visits.push(WebHistoryEntry::new(ts_ms, url, title));
        true
    }

    pub fn oldest_ms(&self) -> Option<u64> {
        self.visits.iter().map(|visit| visit.ts_ms).min()
    }

    pub fn newest_ms(&self) -> Option<u64> {
        self.visits.iter().map(|visit| visit.ts_ms).max()
    }
}

/// Read a Chromium `History` database.
///
/// Two passes into one deduped harvest, because they are two different facts:
/// `visits` is the timeline, and `urls.last_visit_time` is the only surviving
/// trace of pages whose visit rows Chromium has already expired. Both dedupe on
/// `(url, ts)`, so a URL present in both contributes once.
pub fn read_chromium_visits(history_db: &Path) -> Result<VisitHarvest> {
    let snapshot = SqliteSnapshot::take(history_db)?;
    let conn = snapshot.open_read_only()?;
    let mut harvest = VisitHarvest::default();
    let mut seen: HashSet<(String, u64)> = HashSet::new();

    let mut visits = conn
        .prepare(
            "SELECT u.url, u.title, v.visit_time \
             FROM visits v JOIN urls u ON u.id = v.url \
             ORDER BY v.visit_time ASC, u.url ASC",
        )
        .context("preparing the Chromium visits query")?;
    let rows = visits.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (url, title, raw) = row?;
        harvest.push(&mut seen, url, title, chromium_time_to_unix_ms(raw));
    }
    drop(visits);

    let mut urls = conn
        .prepare(
            "SELECT url, title, last_visit_time FROM urls ORDER BY last_visit_time ASC, url ASC",
        )
        .context("preparing the Chromium urls query")?;
    let rows = urls.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (url, title, raw) = row?;
        if harvest.push(&mut seen, url, title, chromium_time_to_unix_ms(raw)) {
            harvest.recovered_from_urls_table += 1;
        }
    }
    Ok(harvest)
}

/// Read a Firefox `places.sqlite`. Same two passes, same reason
/// (`moz_places.last_visit_date` outlives expired `moz_historyvisits` rows).
pub fn read_firefox_visits(places_db: &Path) -> Result<VisitHarvest> {
    let snapshot = SqliteSnapshot::take(places_db)?;
    let conn = snapshot.open_read_only()?;
    let mut harvest = VisitHarvest::default();
    let mut seen: HashSet<(String, u64)> = HashSet::new();

    let mut visits = conn
        .prepare(
            "SELECT p.url, p.title, v.visit_date \
             FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id \
             ORDER BY v.visit_date ASC, p.url ASC",
        )
        .context("preparing the Firefox visits query")?;
    let rows = visits.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (url, title, raw) = row?;
        harvest.push(&mut seen, url, title, firefox_time_to_unix_ms(raw));
    }
    drop(visits);

    let mut places = conn
        .prepare(
            "SELECT url, title, last_visit_date FROM moz_places \
             WHERE last_visit_date IS NOT NULL ORDER BY last_visit_date ASC, url ASC",
        )
        .context("preparing the Firefox places query")?;
    let rows = places.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (url, title, raw) = row?;
        if harvest.push(&mut seen, url, title, firefox_time_to_unix_ms(raw)) {
            harvest.recovered_from_urls_table += 1;
        }
    }
    Ok(harvest)
}

// ---------------------------------------------------------------------------
// Bookmarks
// ---------------------------------------------------------------------------

/// One bookmark, with the folders it sits under. The path IS the tree: heading
/// depth in the collection is `2 + position in this path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkLeaf {
    pub folder_path: Vec<String>,
    pub title: String,
    pub url: String,
}

impl BookmarkLeaf {
    /// The dedupe identity: the same URL filed in two different folders is two
    /// bookmarks, not a duplicate.
    pub fn placement_key(&self) -> (String, String) {
        (self.folder_path.join("/"), self.url.clone())
    }
}

/// How deep a bookmark tree may nest before this stops descending. Real trees
/// are a handful deep; the cap exists so a hostile or corrupt file cannot
/// recurse the stack away.
const MAX_BOOKMARK_DEPTH: usize = 32;

/// The Chromium roots, in the order they are written into the collection.
/// Fixed rather than JSON-object order, which is not guaranteed.
const CHROMIUM_ROOT_ORDER: [&str; 3] = ["bookmark_bar", "other", "synced"];

/// Read a Chromium `Bookmarks` file (JSON) into leaves, in tree order.
pub fn read_chromium_bookmarks(bookmarks_file: &Path) -> Result<Vec<BookmarkLeaf>> {
    let raw = std::fs::read_to_string(bookmarks_file)
        .with_context(|| format!("reading {}", bookmarks_file.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", bookmarks_file.display()))?;
    let roots = value
        .get("roots")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("{} has no bookmark roots", bookmarks_file.display()))?;

    let mut keys: Vec<&String> = roots.keys().collect();
    keys.sort();
    let ordered: Vec<String> = CHROMIUM_ROOT_ORDER
        .iter()
        .map(|key| key.to_string())
        .chain(
            keys.into_iter()
                .filter(|key| !CHROMIUM_ROOT_ORDER.contains(&key.as_str()))
                .cloned(),
        )
        .collect();

    let mut leaves = Vec::new();
    for key in ordered {
        let Some(node) = roots.get(&key).filter(|node| node.is_object()) else {
            continue;
        };
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| humanize_root_key(&key));
        let mut path = vec![name];
        walk_chromium_node(node, &mut path, &mut leaves);
    }
    Ok(leaves)
}

fn humanize_root_key(key: &str) -> String {
    let spaced = key.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Bookmarks".to_string(),
    }
}

fn walk_chromium_node(node: &Value, path: &mut Vec<String>, out: &mut Vec<BookmarkLeaf>) {
    if path.len() > MAX_BOOKMARK_DEPTH {
        return;
    }
    let Some(children) = node.get("children").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        let name = child
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        match child
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "url" => {
                let Some(url) = child
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                else {
                    continue;
                };
                out.push(BookmarkLeaf {
                    folder_path: path.clone(),
                    title: if name.is_empty() {
                        url.to_string()
                    } else {
                        name
                    },
                    url: url.to_string(),
                });
            }
            "folder" => {
                path.push(if name.is_empty() {
                    "Untitled folder".to_string()
                } else {
                    name
                });
                walk_chromium_node(child, path, out);
                path.pop();
            }
            _ => {}
        }
    }
}

/// Firefox root folders carry an empty title and are identified by GUID.
/// `tags________` is deliberately absent: a tag folder holds the SAME
/// bookmarks again, and importing it would duplicate every tagged link under a
/// tree the user never made.
fn firefox_root_folder_name(guid: &str) -> Option<&'static str> {
    match guid {
        "toolbar_____" => Some("Bookmarks Toolbar"),
        "menu________" => Some("Bookmarks Menu"),
        "unfiled_____" => Some("Other Bookmarks"),
        "mobile______" => Some("Mobile Bookmarks"),
        _ => None,
    }
}

/// Firefox `moz_bookmarks` row types.
const MOZ_TYPE_BOOKMARK: i64 = 1;
const MOZ_TYPE_FOLDER: i64 = 2;

struct MozBookmark {
    id: i64,
    parent: i64,
    kind: i64,
    title: String,
    guid: String,
    url: Option<String>,
}

/// Read a Firefox `places.sqlite` bookmark tree into leaves, in tree order.
pub fn read_firefox_bookmarks(places_db: &Path) -> Result<Vec<BookmarkLeaf>> {
    let snapshot = SqliteSnapshot::take(places_db)?;
    let conn = snapshot.open_read_only()?;
    let mut statement = conn
        .prepare(
            "SELECT b.id, b.parent, b.type, COALESCE(b.title, ''), COALESCE(b.guid, ''), p.url \
             FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk \
             ORDER BY b.parent ASC, b.position ASC, b.id ASC",
        )
        .context("preparing the Firefox bookmarks query")?;
    let rows = statement.query_map([], |row| {
        Ok(MozBookmark {
            id: row.get(0)?,
            parent: row.get(1)?,
            kind: row.get(2)?,
            title: row.get(3)?,
            guid: row.get(4)?,
            url: row.get(5)?,
        })
    })?;

    let mut children: HashMap<i64, Vec<MozBookmark>> = HashMap::new();
    let mut root_ids: Vec<i64> = Vec::new();
    for row in rows {
        let node = row?;
        if node.guid == "root________" {
            root_ids.push(node.id);
            continue;
        }
        children.entry(node.parent).or_default().push(node);
    }
    if root_ids.is_empty() {
        // A places.sqlite without the canonical root GUID: parent 0 is the
        // tree's floor in every schema version that has shipped.
        root_ids.push(0);
    }

    let mut leaves = Vec::new();
    for root in root_ids {
        let mut path = Vec::new();
        walk_firefox_children(root, &children, &mut path, &mut leaves);
    }
    Ok(leaves)
}

fn walk_firefox_children(
    parent: i64,
    children: &HashMap<i64, Vec<MozBookmark>>,
    path: &mut Vec<String>,
    out: &mut Vec<BookmarkLeaf>,
) {
    if path.len() > MAX_BOOKMARK_DEPTH {
        return;
    }
    let Some(nodes) = children.get(&parent) else {
        return;
    };
    for node in nodes {
        match node.kind {
            MOZ_TYPE_BOOKMARK => {
                let Some(url) = node.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) else {
                    continue;
                };
                out.push(BookmarkLeaf {
                    folder_path: path.clone(),
                    title: if node.title.trim().is_empty() {
                        url.to_string()
                    } else {
                        node.title.trim().to_string()
                    },
                    url: url.to_string(),
                });
            }
            MOZ_TYPE_FOLDER => {
                if node.guid == "tags________" {
                    continue;
                }
                let name = match firefox_root_folder_name(&node.guid) {
                    Some(root_name) => root_name.to_string(),
                    None if node.title.trim().is_empty() => "Untitled folder".to_string(),
                    None => node.title.trim().to_string(),
                };
                path.push(name);
                walk_firefox_children(node.id, children, path, out);
                path.pop();
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Bookmarks -> a collection, with the folder tree intact
// ---------------------------------------------------------------------------

/// What a bookmark merge did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct BookmarkMergeReport {
    pub added: usize,
    /// Already present at the same `(folder path, url)`.
    pub duplicates: usize,
}

/// Every `(folder path, url)` the collection already holds.
///
/// The path is rebuilt from heading DEPTH, which is how the format encodes
/// nesting, so a bookmark filed under `Bookmarks bar/Rust` is distinguished
/// from the same URL filed under `Other bookmarks`.
fn existing_placements(collection: &Collection) -> HashSet<(String, String)> {
    let mut path: Vec<String> = Vec::new();
    let mut placements = HashSet::new();
    for block in &collection.blocks {
        match block {
            Block::Folder { depth, name } => {
                path.truncate(depth.saturating_sub(2));
                path.push(name.clone());
            }
            Block::Item(item) => {
                placements.insert((path.join("/"), item.url.clone()));
            }
            Block::Raw(_) => {}
        }
    }
    placements
}

/// Where the folder at `index` stops: the next heading at the same depth or
/// shallower, or the end of the document.
fn folder_section_end(blocks: &[Block], index: usize) -> usize {
    let Some(Block::Folder { depth, .. }) = blocks.get(index) else {
        return blocks.len();
    };
    let depth = *depth;
    blocks
        .iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, block)| matches!(block, Block::Folder { depth: other, .. } if *other <= depth))
        .map(|(position, _)| position)
        .unwrap_or(blocks.len())
}

/// Find the folder `name` at `depth` within `[start, end)`.
fn find_folder(
    blocks: &[Block],
    name: &str,
    depth: usize,
    start: usize,
    end: usize,
) -> Option<usize> {
    blocks[start..end.min(blocks.len())]
        .iter()
        .position(
            |block| matches!(block, Block::Folder { depth: d, name: n } if *d == depth && n == name),
        )
        .map(|offset| start + offset)
}

/// Resolve (creating what is missing) the folder path, returning the index of
/// its deepest heading. A missing folder is appended at the END of its parent's
/// section, so an import never lands in the middle of what the user arranged.
fn ensure_folder_path(collection: &mut Collection, path: &[String]) -> Option<usize> {
    let mut start = 0usize;
    let mut end = collection.blocks.len();
    let mut deepest = None;
    for (level, name) in path.iter().enumerate() {
        let depth = 2 + level;
        match find_folder(&collection.blocks, name, depth, start, end) {
            Some(index) => {
                deepest = Some(index);
                start = index + 1;
                end = folder_section_end(&collection.blocks, index);
            }
            None => {
                let mut insert_at = end.min(collection.blocks.len());
                while insert_at > start
                    && matches!(&collection.blocks[insert_at - 1], Block::Raw(raw) if raw.trim().is_empty())
                {
                    insert_at -= 1;
                }
                collection.blocks.splice(
                    insert_at..insert_at,
                    [
                        Block::Raw(String::new()),
                        Block::Folder {
                            depth,
                            name: name.clone(),
                        },
                        Block::Raw(String::new()),
                    ],
                );
                let index = insert_at + 1;
                deepest = Some(index);
                start = index + 1;
                end = index + 2;
            }
        }
    }
    deepest
}

/// Append an item at the end of a folder's section (or of the document, for a
/// pathless item), stepping back over the blank lines that close it so the item
/// joins the list rather than landing after the gap below it.
///
/// The step-back stops at a FLOOR: the blank line that follows a heading is
/// part of the format's shape (`## Papers`, blank, then items — see the spec's
/// example), not trailing space to be reclaimed. Without the floor the first
/// item of a new folder lands flush against its heading and the file this
/// writes stops looking like the file the parser's own docs show.
fn append_item_to_section(collection: &mut Collection, folder_index: Option<usize>, item: Item) {
    let insert_at = match folder_index {
        Some(index) => {
            let floor = match collection.blocks.get(index + 1) {
                Some(Block::Raw(raw)) if raw.trim().is_empty() => index + 2,
                _ => index + 1,
            };
            let mut end = folder_section_end(&collection.blocks, index);
            while end > floor
                && matches!(&collection.blocks[end - 1], Block::Raw(raw) if raw.trim().is_empty())
            {
                end -= 1;
            }
            end
        }
        None => collection.blocks.len(),
    };
    collection.blocks.insert(insert_at, Block::Item(item));
}

/// Merge bookmark leaves into a collection, folders and all.
///
/// Idempotent by `(folder path, url)`: a second import of the same profile adds
/// nothing and rewrites nothing. The same URL in two folders is kept twice,
/// because that is what the user's tree says.
pub fn merge_bookmarks_into_collection(
    collection: &mut Collection,
    leaves: &[BookmarkLeaf],
) -> BookmarkMergeReport {
    let mut placements = existing_placements(collection);
    let mut report = BookmarkMergeReport::default();
    for leaf in leaves {
        if !placements.insert(leaf.placement_key()) {
            report.duplicates += 1;
            continue;
        }
        let folder = ensure_folder_path(collection, &leaf.folder_path);
        append_item_to_section(
            collection,
            folder,
            Item::new(leaf.title.clone(), leaf.url.clone()),
        );
        report.added += 1;
    }
    report
}

// ---------------------------------------------------------------------------
// The import itself
// ---------------------------------------------------------------------------

/// What to import, and where to.
#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub source: BrowserProfile,
    /// `~/.yggterm/web-profiles`.
    pub profiles_root: PathBuf,
    /// The yggterm web profile the import lands in.
    pub target_profile: String,
    pub history: bool,
    pub bookmarks: bool,
    /// Override the derived collection id. `None` ⇒
    /// `bookmarks-<browser>-<source profile>`, which is DERIVED rather than
    /// generated so that a second import finds the same file.
    pub collection_id: Option<String>,
    /// Injected clock — this crate does not read the wall clock for a decision.
    pub now_ms: u64,
    /// The machine's UTC offset, for the frontmatter stamps. An argument for
    /// the same reason `now_ms` is, and the same one the collection store takes
    /// so an imported collection is stamped like a hand-made one.
    pub utc_offset_secs: i32,
    /// Read everything, write nothing.
    pub dry_run: bool,
}

impl ImportRequest {
    pub fn collection_id(&self) -> String {
        self.collection_id.clone().unwrap_or_else(|| {
            format!(
                "bookmarks-{}-{}",
                slugify(&self.source.browser_id),
                slugify(&self.source.dir_name)
            )
        })
    }
}

/// The history half of an import report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HistoryImportReport {
    pub source_db: Option<String>,
    pub journal: Option<String>,
    pub rows_read: usize,
    pub rejected_timestamp: usize,
    pub skipped_not_page: usize,
    pub recovered_from_urls_table: usize,
    pub visits_offered: usize,
    pub visits_written: usize,
    pub duplicates: usize,
    pub rewrote_journal: bool,
    pub oldest_ms: Option<u64>,
    pub newest_ms: Option<u64>,
    /// The two dates the epoch trap would ruin, spelled out so a wrong century
    /// is visible in the report rather than only in the file.
    pub oldest_utc_day: Option<String>,
    pub newest_utc_day: Option<String>,
}

/// The bookmarks half.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BookmarkImportReport {
    pub source_file: Option<String>,
    pub collection: Option<String>,
    pub collection_id: Option<String>,
    pub read: usize,
    pub added: usize,
    pub duplicates: usize,
    pub folders: usize,
}

/// One import, fully accounted for.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub browser: String,
    pub source_profile: String,
    pub source_display_name: String,
    pub source_path: String,
    pub target_profile: String,
    pub dry_run: bool,
    pub history: HistoryImportReport,
    pub bookmarks: BookmarkImportReport,
}

/// Import one browser profile into one yggterm web profile.
///
/// History goes to the profile's `history.jsonl` as visits; bookmarks go to one
/// collection with their folder tree. Both halves are idempotent, so this is
/// safe to re-run — and re-running is the normal way to pick up what the
/// browser has done since.
pub fn import_browser_profile(request: &ImportRequest) -> Result<ImportReport> {
    let mut report = ImportReport {
        browser: request.source.browser_id.clone(),
        source_profile: request.source.dir_name.clone(),
        source_display_name: request.source.display_name.clone(),
        source_path: request.source.path.display().to_string(),
        target_profile: request.target_profile.clone(),
        dry_run: request.dry_run,
        ..ImportReport::default()
    };

    if request.history
        && let Some(db) = request.source.history_db()
    {
        report.history.source_db = Some(db.display().to_string());
        let harvest = match request.source.family {
            BrowserFamily::Chromium => read_chromium_visits(&db)?,
            BrowserFamily::Firefox => read_firefox_visits(&db)?,
        };
        report.history.rows_read = harvest.rows_read;
        report.history.rejected_timestamp = harvest.rejected_timestamp;
        report.history.skipped_not_page = harvest.skipped_not_page;
        report.history.recovered_from_urls_table = harvest.recovered_from_urls_table;
        report.history.visits_offered = harvest.visits.len();
        report.history.oldest_ms = harvest.oldest_ms();
        report.history.newest_ms = harvest.newest_ms();
        report.history.oldest_utc_day = harvest.oldest_ms().map(utc_day);
        report.history.newest_utc_day = harvest.newest_ms().map(utc_day);

        if let Some(journal) = web_history_path_in(&request.profiles_root, &request.target_profile)
        {
            report.history.journal = Some(journal.display().to_string());
            if !request.dry_run {
                let written: HistoryWriteReport = merge_web_visits(&journal, &harvest.visits)
                    .with_context(|| format!("writing {}", journal.display()))?;
                report.history.visits_written = written.appended;
                report.history.duplicates = written.duplicates;
                report.history.rewrote_journal = written.rewrote;
            }
        }
    }

    if request.bookmarks
        && let Some(source) = request.source.bookmarks_source()
    {
        report.bookmarks.source_file = Some(source.display().to_string());
        let leaves = match request.source.family {
            BrowserFamily::Chromium => read_chromium_bookmarks(&source)?,
            BrowserFamily::Firefox => read_firefox_bookmarks(&source)?,
        };
        report.bookmarks.read = leaves.len();
        let id = request.collection_id();
        report.bookmarks.collection_id = Some(id.clone());
        // The STORE owns where a collection lives, what an id may be, and how a
        // file is written — an import is just another writer of one.
        if let Some(store) =
            CollectionStore::for_profile_in(&request.profiles_root, &request.target_profile)
        {
            report.bookmarks.collection =
                store.path_for(&id).map(|path| path.display().to_string());
            let mut collection = match store.load(&id) {
                Ok(existing) => existing,
                Err(_) => new_import_collection(request, &id),
            };
            let merged = merge_bookmarks_into_collection(&mut collection, &leaves);
            report.bookmarks.added = merged.added;
            report.bookmarks.duplicates = merged.duplicates;
            report.bookmarks.folders = collection.folders().len();
            if !request.dry_run && merged.added > 0 {
                touch(&mut collection, request.now_ms, request.utc_offset_secs);
                store
                    .save(&id, &collection)
                    .with_context(|| format!("writing collection {id}"))?;
            }
        }
    }

    Ok(report)
}

/// A fresh collection for an import — built by the STORE, then stamped with
/// where it came from.
///
/// Going through [`build_collection`] rather than setting six fields here is
/// the point: a collection an import made and a collection `collection new`
/// made must be the same kind of file, in the same field order, with the same
/// timestamp format. `imported_from` is the one key this adds, and it is the
/// spec's provenance key (`ychrome/docs/collections.md` §Where they live:
/// *"records where it came from in `imported_from`"*) — what makes a re-import
/// legible a year later.
fn new_import_collection(request: &ImportRequest, id: &str) -> Collection {
    let browser = display_name_for(&request.source.browser_id);
    let name = format!("{browser} bookmarks — {}", request.source.display_name);
    let note = format!(
        "Imported from {browser} ({}). Folders are that browser's folders; \
         re-running the import adds what is new and touches nothing else.",
        request.source.display_name
    );
    let mut collection = build_collection(
        &NewCollection {
            id,
            name: Some(&name),
            profile: &request.target_profile,
            tags: &[],
            note: Some(&note),
        },
        CollectionKind::Collection,
        request.now_ms,
        request.utc_offset_secs,
    );
    collection.set_field(
        "imported_from",
        format!(
            "{{ browser: {}, profile: {}, path: {} }}",
            request.source.browser_id,
            request.source.dir_name,
            request.source.path.display()
        ),
    );
    collection
}

fn display_name_for(browser_id: &str) -> String {
    browser_source(browser_id)
        .map(|source| source.display_name.to_string())
        .unwrap_or_else(|| browser_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_history::read_web_visits;

    /// A scratch directory that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yggterm-import-test-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
        fn join(&self, tail: &str) -> PathBuf {
            self.0.join(tail)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 2021-06-01T00:00:00Z, in all three spellings.
    const UNIX_MS: u64 = 1_622_505_600_000;
    const CHROMIUM_MICROS: i64 = 13_266_979_200_000_000;
    const FIREFOX_MICROS: i64 = 1_622_505_600_000_000;

    // -----------------------------------------------------------------
    // Trap 1: the epoch
    // -----------------------------------------------------------------

    /// ⚠ THE EPOCH LOCK. Both converters land the SAME instant, pinned to a
    /// date a human can check: 2021-06-01. If either constant drifts, a decade
    /// of history moves century and nothing else in the system complains.
    #[test]
    fn both_families_convert_to_the_same_instant() {
        assert_eq!(
            chromium_time_to_unix_ms(CHROMIUM_MICROS),
            Some(UNIX_MS as i64)
        );
        assert_eq!(
            firefox_time_to_unix_ms(FIREFOX_MICROS),
            Some(UNIX_MS as i64)
        );
        assert_eq!(utc_day(UNIX_MS), "2021-06-01");
        // The offset itself, stated once more as arithmetic: 1601 -> 1970.
        assert_eq!(
            CHROMIUM_EPOCH_OFFSET_MICROS / 1_000_000,
            11_644_473_600,
            "seconds between 1601-01-01 and 1970-01-01"
        );
    }

    /// ⚠ THE CENTURY LOCK — the trap stated as a falsification.
    ///
    /// Reading a Chromium stamp as Firefox's puts it in the year 2390; reading
    /// a Firefox stamp as Chromium's puts it before 1601 (negative). Neither
    /// may be accepted, because both look like perfectly ordinary numbers.
    #[test]
    fn a_swapped_epoch_is_refused_in_both_directions() {
        assert_eq!(
            firefox_time_to_unix_ms(CHROMIUM_MICROS),
            None,
            "a Chromium stamp read as Firefox's lands in the 24th century"
        );
        assert_eq!(
            chromium_time_to_unix_ms(FIREFOX_MICROS),
            None,
            "a Firefox stamp read as Chromium's lands before 1601"
        );
        // …and the wrong answers are what a missing guard WOULD have produced.
        assert_eq!(CHROMIUM_MICROS / 1000, 13_266_979_200_000);
        assert!(utc_day(13_266_979_200_000).starts_with("2390"));
    }

    #[test]
    fn implausible_stamps_are_refused() {
        assert_eq!(chromium_time_to_unix_ms(0), None, "never visited");
        assert_eq!(firefox_time_to_unix_ms(0), None);
        assert_eq!(
            chromium_time_to_unix_ms(i64::MIN),
            None,
            "no overflow panic"
        );
        assert_eq!(firefox_time_to_unix_ms(-1), None);
        assert_eq!(
            firefox_time_to_unix_ms(PLAUSIBLE_MIN_MS * 1000),
            Some(PLAUSIBLE_MIN_MS),
            "the boundary is inclusive"
        );
    }

    // -----------------------------------------------------------------
    // Fixtures — built here, never read from the user's real profiles
    // -----------------------------------------------------------------

    /// A Chromium `History` database with the schema columns this reads.
    fn chromium_history_fixture(path: &Path, rows: &[(&str, &str, i64)]) {
        let conn = Connection::open(path).expect("create fixture");
        conn.execute_batch(
            "CREATE TABLE urls(id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, \
             visit_count INTEGER DEFAULT 0, typed_count INTEGER DEFAULT 0, \
             last_visit_time INTEGER NOT NULL, hidden INTEGER DEFAULT 0);
             CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER NOT NULL, \
             visit_time INTEGER NOT NULL, from_visit INTEGER, transition INTEGER DEFAULT 0);",
        )
        .expect("schema");
        for (index, (url, title, visit_time)) in rows.iter().enumerate() {
            let id = index as i64 + 1;
            conn.execute(
                "INSERT INTO urls(id, url, title, last_visit_time) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, url, title, visit_time],
            )
            .expect("insert url");
            conn.execute(
                "INSERT INTO visits(url, visit_time) VALUES (?1, ?2)",
                rusqlite::params![id, visit_time],
            )
            .expect("insert visit");
        }
        conn.close().expect("close fixture");
    }

    /// A Firefox `places.sqlite` with the columns this reads.
    fn firefox_places_fixture(path: &Path) -> Connection {
        let conn = Connection::open(path).expect("create fixture");
        conn.execute_batch(
            "CREATE TABLE moz_places(id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, \
             visit_count INTEGER DEFAULT 0, last_visit_date INTEGER);
             CREATE TABLE moz_historyvisits(id INTEGER PRIMARY KEY, from_visit INTEGER, \
             place_id INTEGER, visit_date INTEGER, visit_type INTEGER);
             CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, \
             parent INTEGER, position INTEGER, title LONGVARCHAR, guid TEXT);",
        )
        .expect("schema");
        conn
    }

    // -----------------------------------------------------------------
    // Trap 2: the database is locked, and the profile is not ours to write
    // -----------------------------------------------------------------

    /// ⚠ THE LOCKED-DB LOCK. Chrome holds `History` open for the whole
    /// session. This holds an EXCLUSIVE transaction on the fixture — the state
    /// a running browser leaves it in — and the import must still work.
    #[test]
    fn a_database_the_browser_is_holding_still_imports() {
        let scratch = Scratch::new("locked");
        let db = scratch.join("History");
        chromium_history_fixture(&db, &[("https://example.org/a", "A", CHROMIUM_MICROS)]);

        let holder = Connection::open(&db).expect("open as the browser would");
        holder
            .execute_batch("BEGIN EXCLUSIVE; INSERT INTO urls(url, last_visit_time) VALUES ('https://example.org/pending', 0);")
            .expect("hold an exclusive write transaction");

        let harvest = read_chromium_visits(&db).expect("import past the lock");
        assert_eq!(harvest.visits.len(), 1);
        assert_eq!(harvest.visits[0].ts_ms, UNIX_MS);
        drop(holder);
    }

    /// ⚠ THE READ-ONLY LOCK. The user's profile is not ours to write. With the
    /// source DIRECTORY read-only, any attempt to open it read-write — or to
    /// leave a journal beside it — fails outright, so this passing is proof
    /// that the read went through a copy.
    #[cfg(unix)]
    #[test]
    fn the_source_profile_is_never_opened_for_writing() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("readonly-source");
        let profile = scratch.join("Default");
        std::fs::create_dir_all(&profile).expect("profile dir");
        let db = profile.join("History");
        chromium_history_fixture(&db, &[("https://example.org/a", "A", CHROMIUM_MICROS)]);

        let mut file_perms = std::fs::metadata(&db).expect("stat").permissions();
        file_perms.set_mode(0o444);
        std::fs::set_permissions(&db, file_perms).expect("chmod file");
        let mut dir_perms = std::fs::metadata(&profile).expect("stat").permissions();
        dir_perms.set_mode(0o555);
        std::fs::set_permissions(&profile, dir_perms).expect("chmod dir");

        let harvest = read_chromium_visits(&db).expect("read a read-only profile");
        assert_eq!(harvest.visits.len(), 1);

        // Nothing new appeared beside it — no journal, no wal, no shm.
        let leftovers: Vec<String> = std::fs::read_dir(&profile)
            .expect("list")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "History")
            .collect();
        assert!(
            leftovers.is_empty(),
            "the import left files in the user's profile: {leftovers:?}"
        );

        let mut restore = std::fs::metadata(&profile).expect("stat").permissions();
        restore.set_mode(0o755);
        std::fs::set_permissions(&profile, restore).expect("restore dir mode");
    }

    /// The newest browsing lives in the `-wal`, not in the main file. A
    /// snapshot that copied only the database would silently drop the last
    /// session — dates right, rows missing.
    #[test]
    fn a_snapshot_carries_the_write_ahead_log() {
        let scratch = Scratch::new("wal");
        let db = scratch.join("places.sqlite");
        let conn = firefox_places_fixture(&db);
        conn.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");
        conn.execute(
            "INSERT INTO moz_places(id, url, title, last_visit_date) VALUES (1, 'https://example.org/wal', 'W', ?1)",
            rusqlite::params![FIREFOX_MICROS],
        )
        .expect("insert");
        conn.execute(
            "INSERT INTO moz_historyvisits(place_id, visit_date) VALUES (1, ?1)",
            rusqlite::params![FIREFOX_MICROS],
        )
        .expect("insert visit");
        // Deliberately NOT checkpointed and NOT closed: the rows are in the
        // -wal, which is exactly the state a running Firefox leaves behind.
        assert!(
            scratch.join("places.sqlite-wal").exists(),
            "the fixture must actually be in WAL mode for this lock to prove anything"
        );

        let harvest = read_firefox_visits(&db).expect("import");
        assert_eq!(
            harvest.visits.len(),
            1,
            "a snapshot without the -wal would find nothing"
        );
        assert_eq!(harvest.visits[0].ts_ms, UNIX_MS);
        drop(conn);
    }

    #[test]
    fn a_snapshot_deletes_itself() {
        let scratch = Scratch::new("snapshot-drop");
        let db = scratch.join("History");
        chromium_history_fixture(&db, &[("https://example.org/a", "A", CHROMIUM_MICROS)]);
        let copy_path = {
            let snapshot = SqliteSnapshot::take(&db).expect("snapshot");
            let path = snapshot.path().to_path_buf();
            assert!(path.exists());
            assert_ne!(path, db, "the copy is never the source");
            path
        };
        assert!(
            !copy_path.exists(),
            "the snapshot must clean up after itself"
        );
    }

    // -----------------------------------------------------------------
    // History import
    // -----------------------------------------------------------------

    #[test]
    fn chromium_history_lands_with_the_right_dates() {
        let scratch = Scratch::new("chromium-history");
        let db = scratch.join("History");
        // 2016-03-04T05:06:07Z and 2021-06-01T00:00:00Z.
        let old_unix_ms = 1_457_067_967_000;
        let old_chromium = (old_unix_ms / 1000 + 11_644_473_600) * 1_000_000;
        chromium_history_fixture(
            &db,
            &[
                ("https://example.org/old", "Old", old_chromium),
                ("https://example.org/new", "New", CHROMIUM_MICROS),
                ("chrome://settings", "Settings", CHROMIUM_MICROS),
            ],
        );
        let harvest = read_chromium_visits(&db).expect("import");
        assert_eq!(harvest.visits.len(), 2, "chrome:// is not browsing history");
        assert_eq!(harvest.skipped_not_page, 2, "counted in both passes");
        assert_eq!(utc_day(harvest.oldest_ms().unwrap()), "2016-03-04");
        assert_eq!(utc_day(harvest.newest_ms().unwrap()), "2021-06-01");
        assert_eq!(harvest.visits[0].url, "https://example.org/old");
        assert_eq!(harvest.visits[0].title, "Old");
    }

    /// Chromium expires `visits` rows at ~90 days but keeps `urls` with a
    /// `last_visit_time`. For the decade-old profile this feature exists to
    /// rescue, that table is most of what survives — so it is a source, not a
    /// fallback.
    #[test]
    fn a_url_whose_visit_rows_were_expired_is_still_imported() {
        let scratch = Scratch::new("expired-visits");
        let db = scratch.join("History");
        chromium_history_fixture(
            &db,
            &[("https://example.org/kept", "Kept", CHROMIUM_MICROS)],
        );
        let conn = Connection::open(&db).expect("open fixture");
        conn.execute_batch("DELETE FROM visits;")
            .expect("expire visits");
        conn.close().expect("close");

        let harvest = read_chromium_visits(&db).expect("import");
        assert_eq!(harvest.visits.len(), 1);
        assert_eq!(harvest.recovered_from_urls_table, 1);
        assert_eq!(harvest.visits[0].ts_ms, UNIX_MS);
    }

    #[test]
    fn firefox_history_lands_with_the_right_dates() {
        let scratch = Scratch::new("firefox-history");
        let db = scratch.join("places.sqlite");
        let conn = firefox_places_fixture(&db);
        conn.execute(
            "INSERT INTO moz_places(id, url, title, last_visit_date) VALUES (1, 'https://example.org/f', 'F', ?1)",
            rusqlite::params![FIREFOX_MICROS],
        )
        .expect("place");
        conn.execute(
            "INSERT INTO moz_historyvisits(place_id, visit_date) VALUES (1, ?1)",
            rusqlite::params![FIREFOX_MICROS],
        )
        .expect("visit");
        conn.close().expect("close");

        let harvest = read_firefox_visits(&db).expect("import");
        assert_eq!(harvest.visits.len(), 1);
        assert_eq!(utc_day(harvest.visits[0].ts_ms), "2021-06-01");
    }

    // -----------------------------------------------------------------
    // Bookmarks
    // -----------------------------------------------------------------

    fn chromium_bookmarks_fixture(path: &Path) {
        let body = serde_json::json!({
            "version": 1,
            "roots": {
                "bookmark_bar": {
                    "type": "folder",
                    "name": "Bookmarks bar",
                    "children": [
                        {"type": "url", "name": "Rust", "url": "https://rust-lang.org"},
                        {"type": "folder", "name": "Reading", "children": [
                            {"type": "url", "name": "APM", "url": "https://example.org/apm.pdf"},
                            {"type": "folder", "name": "Papers", "children": [
                                {"type": "url", "name": "Deep", "url": "https://example.org/deep"}
                            ]}
                        ]}
                    ]
                },
                "other": {
                    "type": "folder",
                    "name": "Other bookmarks",
                    "children": [
                        {"type": "url", "name": "Rust again", "url": "https://rust-lang.org"}
                    ]
                },
                "synced": {"type": "folder", "name": "Mobile bookmarks", "children": []}
            }
        });
        std::fs::write(path, serde_json::to_string_pretty(&body).expect("json")).expect("write");
    }

    /// ⚠ BOOKMARKS ARE FOLDERS. The tree is heading DEPTH, not a flat dump —
    /// which is the difference between a collection the user can read and a
    /// thousand-line list nobody opens.
    #[test]
    fn chromium_bookmarks_keep_their_folder_tree() {
        let scratch = Scratch::new("chromium-bookmarks");
        let file = scratch.join("Bookmarks");
        chromium_bookmarks_fixture(&file);
        let leaves = read_chromium_bookmarks(&file).expect("read");
        assert_eq!(leaves.len(), 4);
        assert_eq!(leaves[0].folder_path, vec!["Bookmarks bar"]);
        assert_eq!(leaves[1].folder_path, vec!["Bookmarks bar", "Reading"]);
        assert_eq!(
            leaves[2].folder_path,
            vec!["Bookmarks bar", "Reading", "Papers"]
        );
        assert_eq!(leaves[3].folder_path, vec!["Other bookmarks"]);

        let mut collection = Collection::default();
        collection.set_field("id", "x");
        let report = merge_bookmarks_into_collection(&mut collection, &leaves);
        assert_eq!(report.added, 4);
        let markdown = collection.to_markdown();
        assert!(markdown.contains("\n## Bookmarks bar\n"), "{markdown}");
        assert!(markdown.contains("\n### Reading\n"), "{markdown}");
        assert!(markdown.contains("\n#### Papers\n"), "{markdown}");
        assert!(markdown.contains("\n## Other bookmarks\n"), "{markdown}");
        assert!(
            markdown.find("### Reading").unwrap() < markdown.find("#### Papers").unwrap(),
            "nesting must follow the tree: {markdown}"
        );
        // …and what came out is still a collection this project can parse.
        let reparsed = Collection::parse(&markdown);
        assert_eq!(reparsed.item_count(), 4);
        assert_eq!(
            reparsed.to_markdown(),
            markdown,
            "round trip stays identity"
        );
    }

    /// The file this writes must look like the file the format's own
    /// documentation shows: a blank line between a heading and its list. This
    /// is the whole rendering, asserted verbatim, because "close enough"
    /// markdown is how a format quietly grows a second dialect.
    #[test]
    fn an_imported_tree_renders_the_way_the_format_documents_itself() {
        let mut collection = Collection::default();
        collection.set_field("id", "x");
        merge_bookmarks_into_collection(
            &mut collection,
            &[
                BookmarkLeaf {
                    folder_path: vec!["Bar".into()],
                    title: "One".into(),
                    url: "https://example.org/1".into(),
                },
                BookmarkLeaf {
                    folder_path: vec!["Bar".into()],
                    title: "Two".into(),
                    url: "https://example.org/2".into(),
                },
                BookmarkLeaf {
                    folder_path: vec!["Bar".into(), "Deep".into()],
                    title: "Three".into(),
                    url: "https://example.org/3".into(),
                },
                BookmarkLeaf {
                    folder_path: vec!["Other".into()],
                    title: "Four".into(),
                    url: "https://example.org/4".into(),
                },
            ],
        );
        assert_eq!(
            collection.to_markdown(),
            "---\nid: x\n---\n\
             \n## Bar\n\
             \n- [One](https://example.org/1)\n\
             - [Two](https://example.org/2)\n\
             \n### Deep\n\
             \n- [Three](https://example.org/3)\n\
             \n## Other\n\
             \n- [Four](https://example.org/4)"
        );
    }

    /// ⚠ THE PLACEMENT LOCK. Dedupe is on `(url, folder path)`, so the SAME
    /// url in two folders is two bookmarks — the user filed it twice on
    /// purpose — while the same url in the same folder is one.
    #[test]
    fn the_same_url_in_two_folders_is_two_bookmarks() {
        let leaves = vec![
            BookmarkLeaf {
                folder_path: vec!["Bar".into()],
                title: "R".into(),
                url: "https://rust-lang.org".into(),
            },
            BookmarkLeaf {
                folder_path: vec!["Other".into()],
                title: "R".into(),
                url: "https://rust-lang.org".into(),
            },
            BookmarkLeaf {
                folder_path: vec!["Bar".into()],
                title: "R".into(),
                url: "https://rust-lang.org".into(),
            },
        ];
        let mut collection = Collection::default();
        let report = merge_bookmarks_into_collection(&mut collection, &leaves);
        assert_eq!(report.added, 2);
        assert_eq!(report.duplicates, 1);
        assert!(collection.contains_url("https://rust-lang.org"));
    }

    /// ⚠ THE BOOKMARK IDEMPOTENCE LOCK. A second import must be byte-for-byte
    /// nothing.
    #[test]
    fn re_merging_the_same_bookmarks_changes_nothing() {
        let scratch = Scratch::new("bookmark-idempotent");
        let file = scratch.join("Bookmarks");
        chromium_bookmarks_fixture(&file);
        let leaves = read_chromium_bookmarks(&file).expect("read");

        let mut collection = Collection::default();
        collection.set_field("id", "x");
        merge_bookmarks_into_collection(&mut collection, &leaves);
        let first = collection.to_markdown();

        let mut second_pass = Collection::parse(&first);
        let report = merge_bookmarks_into_collection(&mut second_pass, &leaves);
        assert_eq!(report.added, 0);
        assert_eq!(report.duplicates, 4);
        assert_eq!(
            second_pass.to_markdown(),
            first,
            "a re-import must leave the collection byte-identical"
        );
    }

    /// A hand-made collection is the user's. An import adds to it without
    /// moving anything they arranged.
    #[test]
    fn merging_into_a_hand_written_collection_appends_and_never_reorders() {
        let source = "---\nid: mine\nname: Mine\n---\n\nMy notes.\n\n## Bookmarks bar\n\n- [Kept](https://example.org/kept)\n\n## Later\n\n- [Also kept](https://example.org/also)\n";
        let mut collection = Collection::parse(source);
        let report = merge_bookmarks_into_collection(
            &mut collection,
            &[BookmarkLeaf {
                folder_path: vec!["Bookmarks bar".into()],
                title: "New".into(),
                url: "https://example.org/new".into(),
            }],
        );
        assert_eq!(report.added, 1);
        let out = collection.to_markdown();
        assert!(out.contains("My notes."), "the prose stays: {out}");
        let bar = out.split("## Later").next().expect("bar section");
        assert!(
            bar.find("Kept").unwrap() < bar.find("New").unwrap(),
            "an import must land AFTER what the user arranged: {out}"
        );
        assert!(
            out.contains("## Later\n\n- [Also kept](https://example.org/also)"),
            "the next section is untouched: {out}"
        );
    }

    #[test]
    fn firefox_bookmarks_use_the_root_guids_for_names_and_skip_tags() {
        let scratch = Scratch::new("firefox-bookmarks");
        let db = scratch.join("places.sqlite");
        let conn = firefox_places_fixture(&db);
        conn.execute_batch(
            "INSERT INTO moz_places(id, url, title) VALUES (1, 'https://rust-lang.org', 'Rust');
             INSERT INTO moz_places(id, url, title) VALUES (2, 'https://example.org/x', 'X');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (1, 2, NULL, 0, 0, '', 'root________');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (2, 2, NULL, 1, 0, '', 'toolbar_____');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (3, 2, NULL, 1, 1, '', 'tags________');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (4, 1, 1, 2, 0, 'Rust', 'aaaaaaaaaaaa');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (5, 2, NULL, 2, 1, 'Study', 'bbbbbbbbbbbb');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (6, 1, 2, 5, 0, 'X', 'cccccccccccc');
             INSERT INTO moz_bookmarks(id, type, fk, parent, position, title, guid) VALUES (7, 1, 1, 3, 0, 'Rust tagged', 'dddddddddddd');",
        )
        .expect("seed");
        conn.close().expect("close");

        let leaves = read_firefox_bookmarks(&db).expect("read");
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| (leaf.folder_path.join("/"), leaf.url.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("Bookmarks Toolbar".to_string(), "https://rust-lang.org"),
                (
                    "Bookmarks Toolbar/Study".to_string(),
                    "https://example.org/x"
                ),
            ],
            "the tags root must not duplicate every tagged bookmark"
        );
    }

    // -----------------------------------------------------------------
    // Profile discovery
    // -----------------------------------------------------------------

    #[test]
    fn chromium_profiles_are_ordered_default_first_then_numerically() {
        let scratch = Scratch::new("chromium-profiles");
        let user_data = scratch.join("chromium");
        for name in [
            "Default",
            "Profile 1",
            "Profile 2",
            "Profile 10",
            "System Profile",
            "Guest Profile",
            "ShaderCache",
        ] {
            std::fs::create_dir_all(user_data.join(name)).expect("dir");
        }
        for name in ["Default", "Profile 1", "Profile 2", "Profile 10"] {
            std::fs::write(user_data.join(name).join("Preferences"), "{}").expect("prefs");
        }
        std::fs::write(
            user_data.join("Local State"),
            serde_json::json!({"profile": {"info_cache": {
                "Default": {"name": "Avikalpa"},
                "Profile 1": {"name": "Work"}
            }}})
            .to_string(),
        )
        .expect("local state");

        let profiles = discover_chromium_profiles("chromium", &user_data);
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.dir_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Default", "Profile 1", "Profile 2", "Profile 10"],
            "Profile 10 must not sort before Profile 2, and non-profile dirs are not profiles"
        );
        assert_eq!(profiles[0].display_name, "Avikalpa");
        assert!(profiles[0].is_default);
        assert_eq!(
            profiles[2].display_name, "Profile 2",
            "no name is the dir name"
        );
    }

    /// A real `profiles.ini` — the shape Zen writes, with `[Profile1]` listed
    /// BEFORE `[Profile0]` and an `[Install…]` section naming a different
    /// default than the legacy `Default=1` flag.
    #[test]
    fn firefox_profiles_ini_is_read_in_index_order_with_the_installs_default() {
        let scratch = Scratch::new("firefox-profiles");
        let root = scratch.join("firefox");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile1]\nName=Default Profile\nIsRelative=1\nPath=0gshn1os.Default Profile\nDefault=1\n\n\
             [Profile0]\nName=Default (release)\nIsRelative=1\nPath=pbj8o23o.Default (release)\n\n\
             [General]\nStartWithLastProfile=1\nVersion=2\n\n\
             [Install2953CB39A2589173]\nDefault=pbj8o23o.Default (release)\nLocked=1\n",
        )
        .expect("ini");

        let profiles = discover_firefox_profiles("zen", &root);
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Default (release)", "Default Profile"],
            "sections sort by index, not by their order in the file"
        );
        assert!(
            profiles[0].is_default,
            "the install's Default names the profile the browser actually opens"
        );
        assert!(
            !profiles[1].is_default,
            "the legacy Default=1 flag loses to it"
        );
        assert_eq!(profiles[0].path, root.join("pbj8o23o.Default (release)"));
    }

    #[test]
    fn an_absolute_profile_path_is_not_joined_to_the_root() {
        let scratch = Scratch::new("firefox-absolute");
        let root = scratch.join("firefox");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile0]\nName=Elsewhere\nIsRelative=0\nPath=/srv/profiles/elsewhere\n",
        )
        .expect("ini");
        let profiles = discover_firefox_profiles("firefox", &root);
        assert_eq!(profiles[0].path, PathBuf::from("/srv/profiles/elsewhere"));
    }

    #[test]
    fn the_browser_table_is_addressable_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for source in BROWSER_SOURCES {
            assert!(seen.insert(source.id), "{} appears twice", source.id);
            assert_eq!(browser_source(source.id), Some(source));
            assert!(
                !source.linux_dirs.is_empty(),
                "{} names no user-data directory",
                source.id
            );
        }
        // The five Chromium forks the user named, plus Edge, plus Firefox.
        for id in ["chrome", "brave", "vivaldi", "chromium", "helium", "edge"] {
            assert_eq!(
                browser_source(id).map(|source| source.family),
                Some(BrowserFamily::Chromium),
                "{id} is a Chromium fork"
            );
        }
        assert_eq!(
            browser_source("firefox").map(|source| source.family),
            Some(BrowserFamily::Firefox)
        );
        assert_eq!(browser_source("netscape"), None);
    }

    // -----------------------------------------------------------------
    // The whole import, end to end
    // -----------------------------------------------------------------

    fn chromium_profile_fixture(scratch: &Scratch) -> BrowserProfile {
        let profile_dir = scratch.join("user-data/Default");
        std::fs::create_dir_all(&profile_dir).expect("profile dir");
        chromium_history_fixture(
            &profile_dir.join("History"),
            &[
                ("https://example.org/one", "One", CHROMIUM_MICROS),
                (
                    "https://example.org/two",
                    "Two",
                    CHROMIUM_MICROS + 86_400_000_000,
                ),
            ],
        );
        chromium_bookmarks_fixture(&profile_dir.join("Bookmarks"));
        BrowserProfile {
            browser_id: "helium".to_string(),
            family: BrowserFamily::Chromium,
            dir_name: "Default".to_string(),
            display_name: "Person 1".to_string(),
            path: profile_dir,
            is_default: true,
        }
    }

    /// ⚠ THE IMPORT IDEMPOTENCE LOCK — the acceptance criterion, end to end:
    /// *"importing it twice changes nothing."* Both files, byte-for-byte.
    #[test]
    fn importing_the_same_profile_twice_changes_nothing() {
        let scratch = Scratch::new("end-to-end");
        let source = chromium_profile_fixture(&scratch);
        let request = ImportRequest {
            source,
            profiles_root: scratch.join("web-profiles"),
            target_profile: "default".to_string(),
            history: true,
            bookmarks: true,
            collection_id: None,
            now_ms: 1_754_000_000_000,
            utc_offset_secs: 5 * 3600 + 1800,
            dry_run: false,
        };

        let first = import_browser_profile(&request).expect("first import");
        assert_eq!(first.history.visits_written, 2);
        assert_eq!(first.bookmarks.added, 4);
        assert_eq!(
            first.bookmarks.collection_id.as_deref(),
            Some("bookmarks-helium-default")
        );

        let journal = PathBuf::from(first.history.journal.clone().expect("journal path"));
        let collection =
            PathBuf::from(first.bookmarks.collection.clone().expect("collection path"));
        let journal_after_first = std::fs::read_to_string(&journal).expect("journal");
        let collection_after_first = std::fs::read_to_string(&collection).expect("collection");

        let second = import_browser_profile(&request).expect("second import");
        assert_eq!(second.history.visits_written, 0);
        assert_eq!(second.history.duplicates, 2);
        assert_eq!(second.bookmarks.added, 0);
        assert_eq!(second.bookmarks.duplicates, 4);
        assert_eq!(
            std::fs::read_to_string(&journal).expect("journal"),
            journal_after_first,
            "a second import must not double the history"
        );
        assert_eq!(
            std::fs::read_to_string(&collection).expect("collection"),
            collection_after_first,
            "a second import must not touch the collection"
        );
    }

    #[test]
    fn an_import_records_where_it_came_from() {
        let scratch = Scratch::new("provenance");
        let source = chromium_profile_fixture(&scratch);
        let source_path = source.path.display().to_string();
        let request = ImportRequest {
            source,
            profiles_root: scratch.join("web-profiles"),
            target_profile: "work".to_string(),
            history: false,
            bookmarks: true,
            collection_id: None,
            now_ms: 1_754_000_000_000,
            utc_offset_secs: 5 * 3600 + 1800,
            dry_run: false,
        };
        let report = import_browser_profile(&request).expect("import");
        let body =
            std::fs::read_to_string(report.bookmarks.collection.expect("path")).expect("read");
        let parsed = Collection::parse(&body);
        assert_eq!(
            parsed.field("imported_from"),
            Some(format!("{{ browser: helium, profile: Default, path: {source_path} }}").as_str()),
            "provenance is the spec's frontmatter key"
        );
        assert_eq!(parsed.field("profile"), Some("work"));
        assert_eq!(parsed.field("kind"), Some("collection"));
        // Stamped by the STORE, in the store's format and the caller's offset —
        // the same line `collection new` would write.
        assert_eq!(
            parsed.field("created_at"),
            Some("2025-08-01T03:43:20+05:30")
        );
        assert_eq!(parsed.id(), Some("bookmarks-helium-default"));
        // The whole file still round-trips through the format's own parser.
        assert_eq!(parsed.to_markdown(), body);
    }

    #[test]
    fn a_dry_run_reads_everything_and_writes_nothing() {
        let scratch = Scratch::new("dry-run");
        let source = chromium_profile_fixture(&scratch);
        let root = scratch.join("web-profiles");
        let request = ImportRequest {
            source,
            profiles_root: root.clone(),
            target_profile: "default".to_string(),
            history: true,
            bookmarks: true,
            collection_id: None,
            now_ms: 1_754_000_000_000,
            utc_offset_secs: 5 * 3600 + 1800,
            dry_run: true,
        };
        let report = import_browser_profile(&request).expect("dry run");
        assert_eq!(report.history.visits_offered, 2);
        assert_eq!(report.history.visits_written, 0);
        assert_eq!(report.bookmarks.read, 4);
        assert!(!root.exists(), "a dry run must not create the profile jar");
    }

    /// The ephemeral profile keeps nothing on disk — an import must respect
    /// that as much as a page load does.
    #[test]
    fn importing_into_the_temp_profile_writes_no_history() {
        let scratch = Scratch::new("temp-profile");
        let source = chromium_profile_fixture(&scratch);
        let root = scratch.join("web-profiles");
        let request = ImportRequest {
            source,
            profiles_root: root.clone(),
            target_profile: "temp".to_string(),
            history: true,
            bookmarks: false,
            collection_id: None,
            now_ms: 1_754_000_000_000,
            utc_offset_secs: 5 * 3600 + 1800,
            dry_run: false,
        };
        let report = import_browser_profile(&request).expect("import");
        assert_eq!(report.history.visits_offered, 2);
        assert_eq!(report.history.journal, None);
        assert_eq!(report.history.visits_written, 0);
        assert!(!root.join("temp").exists());
    }

    /// The imported visits are in the journal in the shape the omnibox and the
    /// history page read — same file, same records, no second format.
    #[test]
    fn imported_visits_are_ordinary_journal_records() {
        let scratch = Scratch::new("journal-shape");
        let source = chromium_profile_fixture(&scratch);
        let request = ImportRequest {
            source,
            profiles_root: scratch.join("web-profiles"),
            target_profile: "default".to_string(),
            history: true,
            bookmarks: false,
            collection_id: None,
            now_ms: 1_754_000_000_000,
            utc_offset_secs: 5 * 3600 + 1800,
            dry_run: false,
        };
        let report = import_browser_profile(&request).expect("import");
        let journal = PathBuf::from(report.history.journal.expect("journal"));
        let visits = read_web_visits(&journal);
        assert_eq!(visits.len(), 2);
        assert_eq!(visits[0].url, "https://example.org/one");
        assert_eq!(utc_day(visits[0].ts_ms), "2021-06-01");
        assert_eq!(utc_day(visits[1].ts_ms), "2021-06-02");
        assert!(
            visits[0].ts_ms < visits[1].ts_ms,
            "the journal stays in visit order"
        );
    }

    #[test]
    fn slugs_are_file_names() {
        assert_eq!(slugify("Default"), "default");
        assert_eq!(slugify("Profile 1"), "profile-1");
        assert_eq!(
            slugify("0gshn1os.Default Profile"),
            "0gshn1os-default-profile"
        );
        assert_eq!(slugify("  --weird--  "), "weird");
    }
}
