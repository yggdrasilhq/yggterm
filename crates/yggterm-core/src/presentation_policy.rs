//! THE SANCTIONED PRESENTATION DEFAULTS, per platform. One table, one owner.
//!
//! # Why this file exists
//!
//! Not because the logic was missing — it was not. `apps/yggterm/src/main.rs`
//! already decides the backend, the terminal renderer and the GL/DMABuf plan,
//! each as a pure function with tests. This file exists because that decision
//! was **unreadable as a whole**: three functions in a binary's `main.rs`,
//! Linux-only, with no single place anyone could look up "what is this product
//! supposed to run as, on this platform".
//!
//! The consequence was paid repeatedly, by the user, in hours:
//!
//! > *"I have seen you countless times, I changed this flag that flag. No."*
//! > *"I have seen agents testing on dev on Xvfb and then suddenly deciding to
//! > restart my yggterm in XWayland and hours lost on bug finding (this has
//! > happened multiple times)."*
//!
//! So the table below is the answer, as DATA, for every platform — and
//! [`crate::presentation_policy`]'s tests hold the live Linux decision
//! functions to the Linux row, so the two cannot drift. It is not a second
//! encoding: it is the encoding, and `main.rs` is checked against it.
//!
//! # THE LAW (read this before touching any variable named here)
//!
//! 1. **A sanctioned default is not a suggestion.** An agent may not set,
//!    export, or "just try" any variable in [`PRESENTATION_VARS`] against the
//!    user's running GUI. Ever. To test an arm, use the sandbox
//!    (`scripts/underglass-sandbox.sh`, `scripts/web-tear-probe.sh`), which
//!    builds a throwaway GUI with its own env and its own daemon.
//! 2. **The user's desktop session decides the backend, not the agent.** On a
//!    Wayland session yggterm runs **Wayland-native**. Forcing X11 gives
//!    XWayland, which changes compositing, input latency and the terminal
//!    renderer all at once, and every subsequent measurement on that GUI is
//!    then about a machine the user does not run.
//! 3. **`/proc/<pid>/environ` CANNOT tell you what is in force.** Every one of
//!    these is applied with `set_var` after exec, so the process environment
//!    shows the LAUNCH env and nothing later. Read the
//!    `gui/startup/linux_desktop_backend_policy` trace event, which is the
//!    decision reporting itself. This has misled at least two investigations.
//! 4. **An override must be LOUD.** Every deviation from this table is recorded
//!    with the variable, the value and where it came from, so "why is my GUI
//!    behaving strangely" is one lookup rather than an afternoon.
//!
//! # Why the defaults are what they are
//!
//! Each row carries its reason inline. The short version: WebKitGTK presents
//! through the compositor, so the questions are always the same three — which
//! display backend, whether GL is real or software, and whether frames reach
//! the screen zero-copy (DMABuf) or through a shared-memory copy. Getting the
//! third wrong is invisible in a screenshot and shows up only as heat and
//! judder, which is exactly the class of bug that keeps costing days.

/// Every environment variable that decides how this product presents.
///
/// The list is the point: it is what an agent must not touch, what the
/// override report enumerates, and what a reviewer greps for. Anything that
/// changes presentation and is NOT here is a bug in this list.
pub const PRESENTATION_VARS: &[&str] = &[
    // Display backend.
    "GDK_BACKEND",
    "WINIT_UNIX_BACKEND",
    // GL reality.
    "LIBGL_ALWAYS_SOFTWARE",
    "GALLIUM_DRIVER",
    "YGGTERM_FORCE_SOFTWARE_GL",
    "YGGTERM_ENABLE_WEBKIT_COMPOSITING",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
    // Frame delivery.
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "YGGTERM_WEB_SURFACE_UNDER_GLASS",
    // Media decode.
    "GST_PLUGIN_FEATURE_RANK",
    // Terminal renderer.
    "YGGTERM_ENABLE_XTERM_CANVAS",
];

/// The session a platform row applies to. Windows/macOS/mobile have exactly one
/// each; Linux has two, and conflating them is the XWayland mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationTarget {
    LinuxWayland,
    LinuxX11,
    /// Headless CI / Xvfb / a sandbox GUI. **Never the user's machine.**
    LinuxHeadless,
    Windows,
    MacOs,
    Android,
    Ios,
}

impl PresentationTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxWayland => "linux-wayland",
            Self::LinuxX11 => "linux-x11",
            Self::LinuxHeadless => "linux-headless",
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }
}

/// One sanctioned setting: the variable, the value, and WHY.
///
/// `value: None` means "this variable must be ABSENT" — which is a real
/// setting, not the lack of one. `LIBGL_ALWAYS_SOFTWARE` unset is what makes GL
/// real; inheriting it from a probe harness is how a GUI silently ends up
/// rasterising on the CPU at 4x to 22x the frame cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanctionedVar {
    pub name: &'static str,
    pub value: Option<&'static str>,
    pub why: &'static str,
}

/// THE TABLE. One row per target.
///
/// A platform with no row yet returns an empty slice and says so, which is
/// honest — Windows and macOS do not build yet (docs/pending-bugs.md §3.0.0),
/// and mobile is further out. An empty row means "undecided", never "defaults
/// are fine".
pub fn sanctioned(target: PresentationTarget) -> &'static [SanctionedVar] {
    match target {
        PresentationTarget::LinuxWayland => &[
            SanctionedVar {
                name: "GDK_BACKEND",
                value: Some("wayland"),
                why: "A Wayland session runs WAYLAND-NATIVE. This must be set explicitly and \
                      EARLY, because the vendored dioxus app.rs forces `x11` whenever it finds \
                      GDK_BACKEND unset — so leaving it to the default is how yggterm lands in \
                      XWayland without anyone choosing it.",
            },
            SanctionedVar {
                name: "WINIT_UNIX_BACKEND",
                value: Some("wayland"),
                why: "The window layer's half of the same decision; split from GDK it can \
                      disagree with the toolkit.",
            },
            SanctionedVar {
                name: "LIBGL_ALWAYS_SOFTWARE",
                value: None,
                why: "ABSENT. Present, every frame rasterises on the CPU. It is inherited from \
                      probe harnesses and CI images, which is why absence is stated as a \
                      requirement rather than assumed.",
            },
            SanctionedVar {
                name: "GALLIUM_DRIVER",
                value: None,
                why: "ABSENT, for the same reason — `llvmpipe` here is software GL wearing the \
                      GPU's name.",
            },
            SanctionedVar {
                name: "WEBKIT_DISABLE_COMPOSITING_MODE",
                value: None,
                why: "ABSENT. Setting it BREAKS THE WEB SURFACE OUTRIGHT on this stack \
                      (docs/pending-bugs.md). It is not a performance knob and never was.",
            },
            SanctionedVar {
                name: "WEBKIT_DISABLE_DMABUF_RENDERER",
                value: None,
                why: "ABSENT — this is the zero-copy path frames and VIDEO travel on. Disabling \
                      it forces shared-memory presentation, which is invisible in a screenshot \
                      and shows up only as heat and judder. The vendored dioxus app.rs sets it \
                      on a Wayland session whenever under-glass is NOT armed, so under-glass \
                      and this are ONE decision, not two.",
            },
            SanctionedVar {
                name: "YGGTERM_WEB_SURFACE_UNDER_GLASS",
                value: Some("1"),
                why: "Armed. It is the current default (5b0280a) AND it is what keeps the \
                      vendored DMABuf-disable above from firing. Turning it off silently costs \
                      the zero-copy path.",
            },
            SanctionedVar {
                name: "GST_PLUGIN_FEATURE_RANK",
                value: Some("vah264dec:MAX,vah265dec:MAX,vavp9dec:MAX,vaav1dec:MAX"),
                why: "HARDWARE VIDEO DECODE. WebKitGTK decodes through GStreamer, which loads \
                      both the VA (hardware) and libav (software) decoders and picks by RANK. \
                      Measured on jojo 2026-08-01 with a YouTube video playing: libgstva.so and \
                      libgstlibav.so were BOTH mapped into the video WebProcess while it burned \
                      58-61% of one core — the signature of software decode winning a pipeline \
                      that had hardware available. Ranking the VA decoders up is the standard \
                      remedy and belongs in the defaults rather than in an agent's head.",
            },
            SanctionedVar {
                name: "YGGTERM_ENABLE_XTERM_CANVAS",
                value: Some("1"),
                why: "xterm.js's WebGL renderer, which can only present with WebKitGTK \
                      accelerated compositing on — so it is downstream of the GL row above, \
                      not an independent choice.",
            },
        ],
        PresentationTarget::LinuxX11 => &[
            SanctionedVar {
                name: "GDK_BACKEND",
                value: Some("x11"),
                why: "A real X11 session. Legitimate — the mistake is X11 on a WAYLAND session, \
                      not X11 on an X11 one.",
            },
            SanctionedVar {
                name: "YGGTERM_ENABLE_XTERM_CANVAS",
                value: Some("0"),
                why: "The WebGL terminal renderer is disabled on X11 — this is the existing \
                      `xterm_canvas_disabled_for_x11` policy, kept.",
            },
            SanctionedVar {
                name: "WEBKIT_DISABLE_COMPOSITING_MODE",
                value: None,
                why: "ABSENT here too; it breaks the surface on any backend.",
            },
        ],
        PresentationTarget::LinuxHeadless => &[
            SanctionedVar {
                name: "GDK_BACKEND",
                value: Some("x11"),
                why: "Xvfb/headless sway IS an X11 server, so x11 is correct HERE. ⚠ This row \
                      is the one that gets carried onto a user's machine by mistake: an agent \
                      tests under Xvfb, learns `GDK_BACKEND=x11`, and then restarts the user's \
                      real GUI with it. That row is LinuxWayland's, not this one.",
            },
            SanctionedVar {
                name: "YGGTERM_ENABLE_XTERM_CANVAS",
                value: Some("0"),
                why: "No GPU worth using under Xvfb; the canvas renderer buys nothing.",
            },
        ],
        // Deliberately empty until the platform builds. See docs/pending-bugs.md
        // §3.0.0 — claiming a default for a platform nobody has run is how a
        // wrong default gets shipped and then defended.
        PresentationTarget::Windows
        | PresentationTarget::MacOs
        | PresentationTarget::Android
        | PresentationTarget::Ios => &[],
    }
}

/// A deviation from the sanctioned table, found in a live environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deviation {
    pub name: &'static str,
    pub expected: Option<&'static str>,
    pub actual: Option<String>,
    pub why: &'static str,
}

/// Compare a live environment against the table.
///
/// Pure: `lookup` supplies the world, so this is testable without touching a
/// process environment and can be run against a REPORTED env (the startup
/// trace) rather than only the current process — which matters, because the
/// current process's `environ` is not the truth (see THE LAW §3).
pub fn deviations(
    target: PresentationTarget,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<Deviation> {
    sanctioned(target)
        .iter()
        .filter_map(|var| {
            let actual = lookup(var.name);
            let matches = match (var.value, actual.as_deref()) {
                (None, None) => true,
                (Some(expected), Some(found)) => expected == found,
                _ => false,
            };
            (!matches).then(|| Deviation {
                name: var.name,
                expected: var.value,
                actual: actual.clone(),
                why: var.why,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    /// The live arming measured on jojo, 2026-08-01, from the startup trace.
    fn jojo_live() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GDK_BACKEND", "wayland"),
            ("WINIT_UNIX_BACKEND", "wayland"),
            ("YGGTERM_WEB_SURFACE_UNDER_GLASS", "1"),
            ("YGGTERM_ENABLE_XTERM_CANVAS", "1"),
            (
                "GST_PLUGIN_FEATURE_RANK",
                "vah264dec:MAX,vah265dec:MAX,vavp9dec:MAX,vaav1dec:MAX",
            ),
        ]
    }

    #[test]
    fn the_live_host_matches_the_wayland_row_once_the_decoder_rank_is_set() {
        // This is the whole point of the table: the machine the user runs is a
        // row in it, not a configuration somebody remembers.
        let deviations = deviations(PresentationTarget::LinuxWayland, &env_of(&jojo_live()));
        assert!(
            deviations.is_empty(),
            "the sanctioned Wayland row should describe the live host exactly: {deviations:#?}"
        );
    }

    #[test]
    fn a_missing_decoder_rank_is_reported_as_a_deviation() {
        // The judder bug, as a test. Before the rank default existed, this is
        // exactly what the live host looked like.
        let mut env = jojo_live();
        env.retain(|(k, _)| *k != "GST_PLUGIN_FEATURE_RANK");
        let found = deviations(PresentationTarget::LinuxWayland, &env_of(&env));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "GST_PLUGIN_FEATURE_RANK");
        assert!(found[0].why.contains("58-61%"), "the reason must carry the measurement");
    }

    #[test]
    fn forcing_x11_on_a_wayland_session_is_a_deviation_and_says_why() {
        // THE mistake this file exists to stop.
        let mut env = jojo_live();
        env.retain(|(k, _)| *k != "GDK_BACKEND");
        env.push(("GDK_BACKEND", "x11"));
        let found = deviations(PresentationTarget::LinuxWayland, &env_of(&env));
        let backend = found
            .iter()
            .find(|d| d.name == "GDK_BACKEND")
            .expect("forcing x11 on a wayland session must be reported");
        assert_eq!(backend.actual.as_deref(), Some("x11"));
        assert!(
            backend.why.contains("XWayland"),
            "the deviation has to NAME the consequence, or it reads as a preference"
        );
    }

    #[test]
    fn an_absent_var_is_a_real_setting_and_its_presence_is_a_deviation() {
        // LIBGL_ALWAYS_SOFTWARE inherited from a probe harness is the 4x-22x
        // frame-cost bug. "Unset" has to be assertable.
        let mut env = jojo_live();
        env.push(("LIBGL_ALWAYS_SOFTWARE", "1"));
        let found = deviations(PresentationTarget::LinuxWayland, &env_of(&env));
        let gl = found
            .iter()
            .find(|d| d.name == "LIBGL_ALWAYS_SOFTWARE")
            .expect("an inherited software-GL force must be reported");
        assert_eq!(gl.expected, None);
        assert_eq!(gl.actual.as_deref(), Some("1"));
    }

    #[test]
    fn disabling_dmabuf_is_reported_because_it_is_invisible_in_a_screenshot() {
        let mut env = jojo_live();
        env.push(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
        let found = deviations(PresentationTarget::LinuxWayland, &env_of(&env));
        assert!(found.iter().any(|d| d.name == "WEBKIT_DISABLE_DMABUF_RENDERER"));
    }

    #[test]
    fn the_headless_row_is_x11_and_says_it_must_not_travel() {
        // The recorded failure: an agent tests under Xvfb, learns x11, and
        // restarts the user's real GUI with it.
        let headless = sanctioned(PresentationTarget::LinuxHeadless);
        let backend = headless
            .iter()
            .find(|v| v.name == "GDK_BACKEND")
            .expect("the headless row pins a backend");
        assert_eq!(backend.value, Some("x11"));
        assert!(
            backend.why.contains("user's"),
            "the headless row MUST warn that it does not travel to a user's machine"
        );
    }

    #[test]
    fn an_unbuilt_platform_is_empty_rather_than_guessed() {
        for target in [
            PresentationTarget::Windows,
            PresentationTarget::MacOs,
            PresentationTarget::Android,
            PresentationTarget::Ios,
        ] {
            assert!(
                sanctioned(target).is_empty(),
                "{} must stay undecided until it builds — a guessed default gets shipped and \
                 then defended",
                target.as_str()
            );
        }
    }

    #[test]
    fn every_sanctioned_var_is_listed_in_the_grep_list() {
        // PRESENTATION_VARS is what an agent is told not to touch and what the
        // override report enumerates. A row naming a variable absent from it
        // would be unenforceable.
        for target in [
            PresentationTarget::LinuxWayland,
            PresentationTarget::LinuxX11,
            PresentationTarget::LinuxHeadless,
        ] {
            for var in sanctioned(target) {
                assert!(
                    PRESENTATION_VARS.contains(&var.name),
                    "{} is sanctioned for {} but missing from PRESENTATION_VARS",
                    var.name,
                    target.as_str()
                );
            }
        }
    }

    #[test]
    fn every_row_explains_itself() {
        // A default with no reason is a default the next agent will "clean up".
        for target in [
            PresentationTarget::LinuxWayland,
            PresentationTarget::LinuxX11,
            PresentationTarget::LinuxHeadless,
        ] {
            for var in sanctioned(target) {
                assert!(
                    var.why.len() > 40,
                    "{} on {} needs a real reason, not a label",
                    var.name,
                    target.as_str()
                );
            }
        }
    }
}
