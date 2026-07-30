//! Never let GLib autolaunch a private D-Bus session bus.
//!
//! # The leak this exists to stop
//!
//! On 2026-07-30 a 16 GB machine was carrying **4,574 MB across 243 orphaned
//! `xdg-desktop-portal` / `ksecretd` / `at-spi-bus-launcher` /
//! `xdg-permission-store` / `dbus-daemon` processes, on 43 private session
//! buses**, the oldest three weeks old — while yggterm's own processes came to
//! 222 MB. The user's reading was that yggterm was the most memory-hungry app
//! they had ever seen. In effect they were right; the processes just did not
//! carry our name.
//!
//! The mechanism: when a GTK/WebKit process starts with **no
//! `DBUS_SESSION_BUS_ADDRESS` to inherit**, GLib autolaunches its own session bus
//! at `/tmp/dbus-XXXXXXXX`, and that bus then activates the whole helper set.
//! Nothing ever reaps any of it. **One launch leaks ~130 MB permanently.** Our
//! launches that hit this: agent shadow views started over ssh, headless probes,
//! cron runs — anything that does not come from a desktop session.
//!
//! # The fix, and why it is one call at the top of `main`
//!
//! Resolve the bus **once, in-process, before GTK initialises**, and every child
//! we later spawn inherits the answer. Doing it at the spawn sites instead would
//! mean getting it right at every one of them forever, and the sites that leaked
//! were precisely the ones nobody remembered.
//!
//! Prefer the real session bus. If there is genuinely none, set a deliberately
//! invalid address: GDBus then fails to connect and reports it, which costs a
//! headless probe some portal features it was never going to use, instead of
//! leaking a bus and four daemons for the rest of the machine's uptime. **A loud
//! missing feature beats a silent permanent leak.**

use std::path::{Path, PathBuf};

/// The address we set when no real session bus exists. Invalid ON PURPOSE, and
/// self-describing so it explains itself in a GDBus warning: an unusable address
/// makes GDBus report a failure, while an ABSENT one makes it autolaunch.
pub const NO_AUTOLAUNCH_ADDRESS: &str = "unix:path=/nonexistent/yggterm-refuses-dbus-autolaunch";

/// What [`resolve`] decided, so the caller can trace it and a test can assert it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionBusDecision {
    /// A bus address was already in the environment. Left exactly as found —
    /// including a `/tmp/dbus-` one, because by then something else already owns
    /// it and stealing it would break that owner.
    Inherited(String),
    /// No address inherited, but the session's real bus socket exists. Adopted.
    AdoptedRuntimeBus(String),
    /// No address and no real bus. Autolaunch refused.
    RefusedAutolaunch,
}

impl SessionBusDecision {
    /// The address to place in the environment, or `None` to leave it untouched.
    pub fn address_to_set(&self) -> Option<&str> {
        match self {
            SessionBusDecision::Inherited(_) => None,
            SessionBusDecision::AdoptedRuntimeBus(address) => Some(address),
            SessionBusDecision::RefusedAutolaunch => Some(NO_AUTOLAUNCH_ADDRESS),
        }
    }

    pub fn reason(&self) -> &'static str {
        match self {
            SessionBusDecision::Inherited(_) => "inherited a session bus address",
            SessionBusDecision::AdoptedRuntimeBus(_) => {
                "adopted the session's real bus so GLib cannot autolaunch a private one"
            }
            SessionBusDecision::RefusedAutolaunch => {
                "no session bus exists; refused GLib's autolaunch so this launch \
                 cannot leak a bus and its helper daemons"
            }
        }
    }
}

/// The session bus socket a login session exposes.
pub fn runtime_bus_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("bus")
}

/// Decide what `DBUS_SESSION_BUS_ADDRESS` should be. Pure: the caller supplies
/// the inherited value, `XDG_RUNTIME_DIR`, and whether the bus socket exists.
///
/// An inherited value always wins, **even a private `/tmp/dbus-` one**: by the
/// time we see it some other process is already using that bus, and replacing it
/// mid-flight would cut that process off from services it has already talked to.
/// This function prevents new leaks; it does not adopt existing ones.
pub fn resolve(
    inherited: Option<&str>,
    runtime_dir: Option<&Path>,
    bus_socket_exists: impl Fn(&Path) -> bool,
) -> SessionBusDecision {
    if let Some(inherited) = inherited {
        let trimmed = inherited.trim();
        if !trimmed.is_empty() {
            return SessionBusDecision::Inherited(trimmed.to_string());
        }
    }
    if let Some(runtime_dir) = runtime_dir {
        let socket = runtime_bus_path(runtime_dir);
        if bus_socket_exists(&socket) {
            return SessionBusDecision::AdoptedRuntimeBus(format!(
                "unix:path={}",
                socket.display()
            ));
        }
    }
    SessionBusDecision::RefusedAutolaunch
}

/// Read the environment, decide, and apply.
///
/// ⚠ **Call this at the very top of `main`, before GTK/GLib initialisation and
/// before any thread is spawned.** `set_var` is unsound once other threads exist,
/// and GLib caches the bus address on first use — a late call changes nothing.
///
/// Returns the decision so the caller can trace it.
#[cfg(unix)]
pub fn adopt_or_refuse_session_bus() -> SessionBusDecision {
    let inherited = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok().map(PathBuf::from);
    let decision = resolve(
        inherited.as_deref(),
        runtime_dir.as_deref(),
        |path| path.exists(),
    );
    if let Some(address) = decision.address_to_set() {
        // SAFETY: documented contract of this function — called at the top of
        // `main`, before any thread exists.
        unsafe {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address);
        }
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE LEAK CASE. A process started over ssh has no bus address, and before
    /// this it got a private one autolaunched by GLib plus four helper daemons
    /// that outlived it by weeks. With a real session bus on the host, adopt it.
    #[test]
    fn a_launch_with_no_inherited_bus_adopts_the_real_one() {
        let decision = resolve(None, Some(Path::new("/run/user/1000")), |_| true);
        assert_eq!(
            decision,
            SessionBusDecision::AdoptedRuntimeBus("unix:path=/run/user/1000/bus".to_string())
        );
        assert_eq!(
            decision.address_to_set(),
            Some("unix:path=/run/user/1000/bus"),
            "the address must actually be placed in the environment, or GLib \
             autolaunches exactly as before"
        );
    }

    /// An empty string is not an address. Treating it as inherited would leave it
    /// in place, and GLib treats an empty value as unset — so it would autolaunch
    /// and the leak would survive the fix.
    #[test]
    fn an_empty_inherited_address_is_not_treated_as_inherited() {
        for blank in ["", "   "] {
            let decision = resolve(Some(blank), Some(Path::new("/run/user/1000")), |_| true);
            assert_eq!(
                decision,
                SessionBusDecision::AdoptedRuntimeBus("unix:path=/run/user/1000/bus".to_string()),
                "a blank inherited value must not block adoption: {blank:?}"
            );
        }
    }

    /// With no bus anywhere, REFUSE rather than let GLib autolaunch. The cost is
    /// portal features a headless probe was not going to use; the alternative is
    /// a bus plus four daemons leaked for the machine's uptime.
    #[test]
    fn with_no_session_bus_at_all_autolaunch_is_refused_not_allowed() {
        for runtime in [Some(Path::new("/run/user/1000")), None] {
            let decision = resolve(None, runtime, |_| false);
            assert_eq!(decision, SessionBusDecision::RefusedAutolaunch);
            let address = decision
                .address_to_set()
                .expect("refusal must still SET an address — an absent one is what autolaunches");
            assert_eq!(address, NO_AUTOLAUNCH_ADDRESS);
            assert!(
                !address.is_empty(),
                "an empty address reads as unset to GLib and autolaunches"
            );
        }
    }

    /// An inherited address is never replaced — not even a private one. By the
    /// time we see it, another process owns that bus and has already talked to
    /// services on it; swapping it underneath would break that process, and this
    /// function's job is preventing NEW leaks, not adopting existing ones.
    #[test]
    fn an_inherited_address_is_left_alone_even_when_it_is_a_private_bus() {
        let private = "unix:path=/tmp/dbus-AbCdEf,guid=1";
        let decision = resolve(Some(private), Some(Path::new("/run/user/1000")), |_| true);
        assert_eq!(
            decision,
            SessionBusDecision::Inherited(private.to_string())
        );
        assert_eq!(
            decision.address_to_set(),
            None,
            "an inherited bus must not be rewritten out from under its owner"
        );

        let real = "unix:path=/run/user/1000/bus";
        assert_eq!(
            resolve(Some(real), None, |_| false).address_to_set(),
            None,
            "the normal desktop case must be a no-op"
        );
    }

    /// The bus socket is `$XDG_RUNTIME_DIR/bus`. Getting this path wrong makes
    /// every launch fall through to the refusal branch, which would silently cost
    /// the whole fleet its portal integration.
    #[test]
    fn the_runtime_bus_path_is_the_documented_one() {
        assert_eq!(
            runtime_bus_path(Path::new("/run/user/1000")),
            PathBuf::from("/run/user/1000/bus")
        );
    }

    /// ANCHOR: every entry point that can touch GTK must make this call, and make
    /// it before anything else can spawn a thread or initialise GLib. A resolver
    /// nobody calls is a leak with a unit test, and the sites that leaked were the
    /// ones nobody remembered — so the wiring is locked, not trusted.
    ///
    /// The shell script is anchored too: it starts `sway` itself, before the
    /// binary's own refusal can apply.
    #[test]
    fn every_entry_point_refuses_autolaunch_before_it_can_happen() {
        const GUI: &str = include_str!("../../../apps/yggterm/src/main.rs");
        const HEADLESS: &str = include_str!("../../../apps/yggterm/src/bin/yggterm-headless.rs");
        const SHADOW: &str = include_str!("../../../scripts/shadow-client.sh");

        for (name, source) in [("yggterm", GUI), ("yggterm-headless", HEADLESS)] {
            let body = source
                .split("fn main() -> Result<()> {")
                .nth(1)
                .unwrap_or_else(|| panic!("{name} has no main()"));
            let call = body
                .find("session_bus::adopt_or_refuse_session_bus()")
                .unwrap_or_else(|| {
                    panic!(
                        "{name} does not refuse D-Bus autolaunch: a GTK touch with no \
                         inherited address leaks a session bus and its helper daemons \
                         permanently"
                    )
                });
            // It must be the FIRST thing: GLib caches the address on first use and
            // `set_var` is unsound once a thread exists.
            let first_stmt = body
                .find(';')
                .expect("main() has statements");
            assert!(
                call < first_stmt + 2,
                "{name} resolves the session bus too late — it must be the first \
                 statement in main(), before any thread or GLib initialisation"
            );
        }

        assert!(
            SHADOW.contains("yggterm-refuses-dbus-autolaunch"),
            "shadow-client.sh starts sway before the binary can refuse, so it must \
             resolve the bus itself — this launcher is where 13 run dirs' worth of \
             orphaned helper sets came from"
        );
        assert!(
            SHADOW.contains(r#"if [ -S "$XDG_RUNTIME_DIR/bus" ]"#),
            "the shadow launcher must prefer the REAL session bus when there is one"
        );
    }
}
