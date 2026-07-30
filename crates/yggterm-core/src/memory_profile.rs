//! Where yggterm's memory actually goes.
//!
//! # Why this exists, and why it does not measure "the yggterm process"
//!
//! On 2026-07-30 the user reported yggterm consuming a 16 GB machine —
//! *"far far surpassing chrome with 100 tabs"* — while every instrument we had
//! showed yggterm's own processes at a few hundred MB. Both readings were true.
//! The memory was real, it was ours, and **it was in processes not named
//! yggterm**: 232 `xdg-desktop-portal` / `ksecretd` / `at-spi-bus-launcher` /
//! `xdg-permission-store` / `dbus-daemon` processes, on **34 private D-Bus
//! session buses**, holding **4.5 GB** — orphaned to init, some three weeks old.
//!
//! The mechanism: a GTK/WebKit process started without a D-Bus session (an agent
//! launching a shadow view over ssh, a headless probe, a cron run) makes GLib
//! **autolaunch its own session bus**, which then activates that whole helper set.
//! Nothing ever reaps it. One launch leaks ~130 MB permanently.
//!
//! So the load-bearing rule of this module:
//!
//! > **A process tree is not a process name.** Attributing yggterm's cost by
//! > matching `comm == "yggterm"` is what hid four and a half gigabytes for three
//! > weeks. What yggterm SPAWNS is yggterm's cost, including after it is orphaned
//! > to init and has lost every visible link back to us.
//!
//! And the second rule, learned the same day:
//!
//! > **RSS undercounts, and on a thrashing box it undercounts by a factor of
//! > four.** One web process read `rss = 123 MB` while holding `swap = 406 MB`.
//! > The number that has to fit in the machine is `rss + swap`, so that is what
//! > this module calls COMMITTED and what every total is built from.
//!
//! Pure by construction: it takes already-sampled [`ProcSample`]s and returns a
//! report. The `/proc` walk lives at the call site, which is what lets the whole
//! attribution be driven from fixtures — including leak shapes that would need a
//! three-week-old desktop to reproduce live.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One process, as sampled from `/proc/<pid>/`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcSample {
    pub pid: i32,
    pub ppid: i32,
    /// `/proc/<pid>/comm` — TRUNCATED TO 15 BYTES BY THE KERNEL. Never match a
    /// long name against it (`WebKitWebProcess` arrives as `WebKitWebProces`);
    /// that is why [`Role::classify`] matches on prefixes.
    pub comm: String,
    /// `VmRSS` in kB.
    pub rss_kb: u64,
    /// `VmSwap` in kB. Zero when unreadable, which is the safe direction: it
    /// makes a total too SMALL, and a probe that overstates a leak gets ignored.
    pub swap_kb: u64,
    /// `DBUS_SESSION_BUS_ADDRESS` from the environ, when readable. `None` covers
    /// both "not set" and "permission denied" — the caller must not invent one.
    pub dbus_session: Option<String>,
    /// Whether anything in the environ or cmdline ties this process to yggterm
    /// (a `YGGTERM_*` variable, a shadow runtime dir, our binary path). Computed
    /// at the call site because it needs the raw environ.
    pub yggterm_marked: bool,
}

impl ProcSample {
    /// What has to fit in the machine.
    pub fn committed_kb(&self) -> u64 {
        self.rss_kb.saturating_add(self.swap_kb)
    }
}

/// What a process is, for attribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    /// The GUI, its supervisor, and the headless daemons.
    Yggterm,
    /// One WebKit web-content process — in our shell, one per web TAB.
    WebContent,
    /// WebKit's network and GPU processes.
    WebSupport,
    /// ychrome's client and daemon.
    Ychrome,
    /// A headless compositor backing a shadow view client.
    ShadowCompositor,
    /// A D-Bus session-bus daemon.
    DbusDaemon,
    /// A service the session bus ACTIVATES: the portal, the secret service, the
    /// accessibility bus, the permission store. Harmless once. The leak is that
    /// there is one set per autolaunched bus and nobody reaps them.
    SessionHelper,
    /// A crash handler. Accumulates one per crash and never exits on its own.
    CrashHandler,
    /// Anything else.
    Other,
}

impl Role {
    /// Classify by command name. **Prefix matching, because `/proc/<pid>/comm` is
    /// truncated to 15 bytes** — `WebKitWebProcess` arrives as `WebKitWebProces`
    /// and an equality test against the full name silently matches nothing,
    /// which would file every web process under `Other` and hide the very cost
    /// this module exists to show.
    pub fn classify(comm: &str) -> Role {
        const HELPERS: [&str; 5] = [
            "xdg-desktop-por",
            "ksecretd",
            "at-spi-bus-laun",
            "at-spi2-registr",
            "xdg-permission-",
        ];
        if comm.starts_with("WebKitWebProc") {
            return Role::WebContent;
        }
        if comm.starts_with("WebKitNetwork") || comm.starts_with("WebKitGPU") {
            return Role::WebSupport;
        }
        if comm.starts_with("ychrome") {
            return Role::Ychrome;
        }
        if comm.starts_with("yggterm") {
            return Role::Yggterm;
        }
        if comm == "sway" {
            return Role::ShadowCompositor;
        }
        if comm.starts_with("dbus-daemon") || comm.starts_with("dbus-broker") {
            return Role::DbusDaemon;
        }
        if HELPERS.iter().any(|h| comm.starts_with(h)) {
            return Role::SessionHelper;
        }
        if comm.starts_with("drkonqi") {
            return Role::CrashHandler;
        }
        Role::Other
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Yggterm => "yggterm (GUI, supervisor, daemons)",
            Role::WebContent => "web content (one process per web tab)",
            Role::WebSupport => "web support (network, GPU)",
            Role::Ychrome => "ychrome (client, daemon)",
            Role::ShadowCompositor => "shadow compositors",
            Role::DbusDaemon => "session bus daemons",
            Role::SessionHelper => "session helpers (portal, secrets, a11y)",
            Role::CrashHandler => "crash handlers",
            Role::Other => "other",
        }
    }
}

/// Is this a PRIVATE, autolaunched session bus rather than the user's real one?
///
/// The user's login session bus lives at `$XDG_RUNTIME_DIR/bus`. GLib's
/// autolaunch fallback instead creates `/tmp/dbus-XXXXXXXX`, and **that address
/// is the fingerprint of the leak**: every process on such a bus was started by
/// something that had no session bus to inherit, which for this machine means an
/// agent-launched yggterm or WebKit process.
pub fn is_private_bus(address: &str) -> bool {
    address.contains("/tmp/dbus-")
}

/// One line of the report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleTotal {
    pub role: Role,
    pub label: String,
    pub procs: usize,
    pub rss_kb: u64,
    pub swap_kb: u64,
    pub committed_kb: u64,
}

/// The leak this module was built to make impossible to miss again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivateBusLeak {
    /// How many distinct autolaunched buses are represented.
    pub buses: usize,
    /// Processes living on them.
    pub procs: usize,
    pub committed_kb: u64,
    /// Bus addresses, sorted, so a caller can reap by bus.
    pub bus_addresses: Vec<String>,
}

/// The whole picture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryProfile {
    pub by_role: Vec<RoleTotal>,
    /// Everything attributable to yggterm's plane: our own processes, the web
    /// fleet, ychrome, shadow compositors, and every helper on a private bus.
    pub yggterm_plane_committed_kb: u64,
    /// The part of that which is pure leak — orphaned helpers nothing will reap.
    pub private_bus_leak: PrivateBusLeak,
    /// Web-content processes and their mean cost: the per-tab number, which is
    /// the one that decides whether "100 tabs" is affordable.
    pub web_content_procs: usize,
    pub web_content_mean_committed_kb: u64,
    /// Total across every sample, for a sanity check against `free`.
    pub total_committed_kb: u64,
    /// Findings worth putting in front of a human, most serious first.
    pub warnings: Vec<String>,
}

/// Roles that are yggterm's own cost by construction.
fn is_plane_role(role: Role) -> bool {
    matches!(
        role,
        Role::Yggterm
            | Role::WebContent
            | Role::WebSupport
            | Role::Ychrome
            | Role::ShadowCompositor
    )
}

/// Build the profile.
pub fn profile(samples: &[ProcSample]) -> MemoryProfile {
    let mut totals: BTreeMap<Role, RoleTotal> = BTreeMap::new();
    let mut leak_buses: BTreeSet<String> = BTreeSet::new();
    let mut leak_procs = 0usize;
    let mut leak_committed = 0u64;
    let mut plane_committed = 0u64;
    let mut web_procs = 0usize;
    let mut web_committed = 0u64;
    let mut total_committed = 0u64;

    for sample in samples {
        let role = Role::classify(&sample.comm);
        let committed = sample.committed_kb();
        total_committed = total_committed.saturating_add(committed);

        let entry = totals.entry(role).or_insert_with(|| RoleTotal {
            role,
            label: role.label().to_string(),
            procs: 0,
            rss_kb: 0,
            swap_kb: 0,
            committed_kb: 0,
        });
        entry.procs += 1;
        entry.rss_kb = entry.rss_kb.saturating_add(sample.rss_kb);
        entry.swap_kb = entry.swap_kb.saturating_add(sample.swap_kb);
        entry.committed_kb = entry.committed_kb.saturating_add(committed);

        if role == Role::WebContent {
            web_procs += 1;
            web_committed = web_committed.saturating_add(committed);
        }

        let on_private_bus = sample
            .dbus_session
            .as_deref()
            .map(is_private_bus)
            .unwrap_or(false);

        // A helper on a private bus is the leak. Counted as OURS even though
        // nothing in its name, its parent (init, once orphaned) or its cmdline
        // says yggterm — that missing link is exactly why it went unseen.
        if on_private_bus && matches!(role, Role::SessionHelper | Role::DbusDaemon) {
            leak_procs += 1;
            leak_committed = leak_committed.saturating_add(committed);
            if let Some(bus) = sample.dbus_session.as_deref() {
                leak_buses.insert(bus.to_string());
            }
        }

        if is_plane_role(role) || (on_private_bus && role == Role::SessionHelper) {
            plane_committed = plane_committed.saturating_add(committed);
        }
    }

    let leak = PrivateBusLeak {
        buses: leak_buses.len(),
        procs: leak_procs,
        committed_kb: leak_committed,
        bus_addresses: leak_buses.into_iter().collect(),
    };

    let mut warnings = Vec::new();
    if leak.buses > 1 {
        warnings.push(format!(
            "{} autolaunched D-Bus session buses are holding {} helper processes and {} MB. \
             A GTK/WebKit process started with no session bus to inherit makes GLib autolaunch \
             its own, which activates a portal/secrets/a11y set that nothing ever reaps. \
             One launch leaks it permanently.",
            leak.buses,
            leak.procs,
            leak.committed_kb / 1024
        ));
    }
    if let Some(crash) = totals.get(&Role::CrashHandler) {
        if crash.procs > 1 {
            warnings.push(format!(
                "{} crash handlers are still resident, holding {} MB — one per crash, and they \
                 do not exit on their own.",
                crash.procs,
                crash.committed_kb / 1024
            ));
        }
    }
    // A swap-heavy web fleet is the audio-glitch mechanism, not just bulk.
    if let Some(web) = totals.get(&Role::WebContent) {
        if web.swap_kb > web.rss_kb && web.procs > 0 {
            warnings.push(format!(
                "web content holds more on DISK than in RAM ({} MB swapped vs {} MB resident): \
                 a decoded media buffer faulted back from disk arrives late, which is audible.",
                web.swap_kb / 1024,
                web.rss_kb / 1024
            ));
        }
    }

    MemoryProfile {
        by_role: totals.into_values().collect(),
        yggterm_plane_committed_kb: plane_committed,
        private_bus_leak: leak,
        web_content_procs: web_procs,
        web_content_mean_committed_kb: if web_procs > 0 {
            web_committed / web_procs as u64
        } else {
            0
        },
        total_committed_kb: total_committed,
        warnings,
    }
}

/// Render the profile for a terminal.
pub fn render(profile: &MemoryProfile) -> String {
    let mut out = String::new();
    out.push_str("yggterm memory profile (COMMITTED = rss + swap; rss alone undercounts)\n\n");
    out.push_str(&format!(
        "  {:>9}  {:>9}  {:>9}  {:>5}  {}\n",
        "COMMIT", "RSS", "SWAP", "PROCS", "ROLE"
    ));
    let mut rows: Vec<&RoleTotal> = profile.by_role.iter().collect();
    rows.sort_by(|a, b| b.committed_kb.cmp(&a.committed_kb));
    for row in rows {
        out.push_str(&format!(
            "  {:>7} MB  {:>7} MB  {:>7} MB  {:>5}  {}\n",
            row.committed_kb / 1024,
            row.rss_kb / 1024,
            row.swap_kb / 1024,
            row.procs,
            row.label
        ));
    }
    out.push_str(&format!(
        "\n  yggterm's whole plane: {} MB committed\n",
        profile.yggterm_plane_committed_kb / 1024
    ));
    if profile.web_content_procs > 0 {
        out.push_str(&format!(
            "  web tabs: {} processes, mean {} MB each\n",
            profile.web_content_procs,
            profile.web_content_mean_committed_kb / 1024
        ));
    }
    if profile.private_bus_leak.buses > 0 {
        out.push_str(&format!(
            "  LEAKED: {} MB across {} procs on {} autolaunched buses\n",
            profile.private_bus_leak.committed_kb / 1024,
            profile.private_bus_leak.procs,
            profile.private_bus_leak.buses
        ));
    }
    for warning in &profile.warnings {
        out.push_str(&format!("\n  ⚠ {warning}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: i32, comm: &str, rss_kb: u64, swap_kb: u64, bus: Option<&str>) -> ProcSample {
        ProcSample {
            pid,
            ppid: 1,
            comm: comm.to_string(),
            rss_kb,
            swap_kb,
            dbus_session: bus.map(str::to_string),
            yggterm_marked: false,
        }
    }

    /// THE REGRESSION THIS MODULE EXISTS FOR. A fixture of the real 2026-07-30
    /// machine shape: yggterm's own processes are small, and the cost is in
    /// helper processes on autolaunched buses that no name, parent or cmdline
    /// ties back to us.
    ///
    /// The profile must attribute them to yggterm's plane anyway. A probe that
    /// reported "yggterm: 300 MB" here would be the same lie that hid 4.5 GB for
    /// three weeks.
    #[test]
    fn helpers_on_autolaunched_buses_are_counted_as_ours_however_orphaned() {
        let mut samples = vec![
            sample(100, "yggterm", 90_000, 40_000, Some("unix:path=/run/user/1000/bus")),
            sample(101, "WebKitWebProces", 120_000, 400_000, Some("unix:path=/run/user/1000/bus")),
        ];
        // Ten dead shadow launches, each leaving a bus and four helpers.
        for i in 0..10 {
            let bus = format!("unix:path=/tmp/dbus-{i:08},guid=deadbeef{i}");
            samples.push(sample(200 + i * 5, "dbus-daemon", 2_000, 10_000, Some(&bus)));
            samples.push(sample(201 + i * 5, "xdg-desktop-por", 8_000, 38_000, Some(&bus)));
            samples.push(sample(202 + i * 5, "ksecretd", 6_000, 36_000, Some(&bus)));
            samples.push(sample(203 + i * 5, "at-spi-bus-laun", 1_500, 1_500, Some(&bus)));
        }

        let report = profile(&samples);

        assert_eq!(report.private_bus_leak.buses, 10, "one bus per dead launch");
        assert_eq!(report.private_bus_leak.procs, 40, "four helpers per bus");
        // 10 * (12000 + 46000 + 42000 + 3000) kB
        assert_eq!(report.private_bus_leak.committed_kb, 1_030_000);

        // The leak must be inside yggterm's plane total, not filed under "other".
        assert!(
            report.yggterm_plane_committed_kb >= report.private_bus_leak.committed_kb,
            "the leak must be attributed to yggterm's plane: plane={} leak={}",
            report.yggterm_plane_committed_kb,
            report.private_bus_leak.committed_kb
        );
        // And it must dominate our own processes — the whole point of the report.
        let own = report
            .by_role
            .iter()
            .find(|r| r.role == Role::Yggterm)
            .expect("yggterm role present")
            .committed_kb;
        assert!(
            report.private_bus_leak.committed_kb > own * 5,
            "the fixture's leak dwarfs our own footprint, and the report must say so"
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("autolaunched")),
            "a multi-bus leak must be warned about by name: {:?}",
            report.warnings
        );
    }

    /// The user's own session bus is NOT a leak. Without this the probe would
    /// cry wolf on every healthy desktop and be ignored exactly when it matters.
    #[test]
    fn the_real_session_bus_is_never_reported_as_a_leak() {
        let real = "unix:path=/run/user/1000/bus";
        let samples = vec![
            sample(1, "dbus-daemon", 2_000, 0, Some(real)),
            sample(2, "xdg-desktop-por", 8_000, 0, Some(real)),
            sample(3, "ksecretd", 6_000, 0, Some(real)),
            sample(4, "at-spi-bus-laun", 1_500, 0, Some(real)),
        ];
        let report = profile(&samples);
        assert_eq!(report.private_bus_leak.buses, 0);
        assert_eq!(report.private_bus_leak.committed_kb, 0);
        assert!(
            report.warnings.is_empty(),
            "a healthy session must produce no warnings: {:?}",
            report.warnings
        );
        assert!(is_private_bus("unix:path=/tmp/dbus-AbCdEf,guid=1"));
        assert!(!is_private_bus(real));
    }

    /// `/proc/<pid>/comm` IS TRUNCATED TO 15 BYTES. Classifying on equality
    /// against the real names silently files every web process under `Other`,
    /// which would understate the per-tab cost — the number that decides whether
    /// "100 tabs" is affordable.
    #[test]
    fn classification_survives_the_kernels_fifteen_byte_comm_truncation() {
        assert_eq!(Role::classify("WebKitWebProces"), Role::WebContent);
        assert_eq!(Role::classify("WebKitWebProcess"), Role::WebContent);
        assert_eq!(Role::classify("WebKitNetworkPr"), Role::WebSupport);
        assert_eq!(Role::classify("xdg-desktop-por"), Role::SessionHelper);
        assert_eq!(Role::classify("at-spi-bus-laun"), Role::SessionHelper);
        assert_eq!(Role::classify("xdg-permission-"), Role::SessionHelper);
        assert_eq!(Role::classify("drkonqi-coredum"), Role::CrashHandler);
        assert_eq!(Role::classify("yggterm-headles"), Role::Yggterm);
        assert_eq!(Role::classify("sway"), Role::ShadowCompositor);
        assert_eq!(Role::classify("plasmashell"), Role::Other);
    }

    /// COMMITTED, not RSS. A web process at rss=123 MB / swap=406 MB is the real
    /// 2026-07-30 reading, and a report built on RSS would have called it 123 MB
    /// — a quarter of what has to fit in the machine.
    #[test]
    fn committed_counts_the_part_that_is_on_disk() {
        let samples = vec![sample(1, "WebKitWebProces", 123_000, 406_000, None)];
        let report = profile(&samples);
        assert_eq!(report.total_committed_kb, 529_000);
        assert_eq!(report.web_content_mean_committed_kb, 529_000);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("more on DISK than in RAM")),
            "a majority-swapped web fleet is the audio-glitch mechanism and must be \
             called out: {:?}",
            report.warnings
        );
    }

    /// An unreadable environ must not invent a bus. Guessing "private" there
    /// would blame the user's own desktop for the leak; guessing "real" would
    /// hide ours. `None` is neither, and stays out of the leak total.
    #[test]
    fn an_unreadable_environ_is_never_guessed_into_the_leak() {
        let samples = vec![
            sample(1, "xdg-desktop-por", 8_000, 38_000, None),
            sample(2, "ksecretd", 6_000, 36_000, None),
        ];
        let report = profile(&samples);
        assert_eq!(
            report.private_bus_leak.procs, 0,
            "a helper whose bus we could not read is not evidence of a leak"
        );
        assert!(report.warnings.is_empty());
        // It still shows up in the role table, so the memory is never unaccounted.
        assert_eq!(report.total_committed_kb, 88_000);
    }
}
