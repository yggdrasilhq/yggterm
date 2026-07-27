//! The view supervisor: N views, and the process→view map the API does not give.

use std::time::{Duration, Instant};

use crate::view::View;
use crate::{Engine, Error, Result};

/// Handle to a view owned by a [`Supervisor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(usize);

impl ViewId {
    pub fn index(self) -> usize {
        self.0
    }
}

/// A process serving web content, as found by walking OUR descendants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebProcess {
    pub pid: u32,
    /// `comm` from `/proc/<pid>/stat` — **truncated to 15 characters** by the
    /// kernel, so `WPENetworkProcess` reads as `WPENetworkProce`.
    pub comm: String,
    pub ppid: u32,
}

struct Slot {
    view: View,
    /// The URI this view was opened on. A crashed view has no document left to
    /// reload, so recovery re-navigates rather than calling `reload()`.
    uri: String,
    /// The web process pid attributed to this view, if the open/restart diff
    /// could attribute one. See [`Supervisor::open`].
    web_process: Option<u32>,
}

/// Owns N headless views and supervises their web processes.
///
/// **Detection and restart are separate, explicit calls.** Nothing here
/// restarts a view on its own: a crash loop that silently re-spawns is far
/// worse than one a caller can see, and the caller is the only thing that knows
/// whether a reload is safe for the work in flight.
pub struct Supervisor<'engine> {
    engine: &'engine Engine,
    slots: Vec<Slot>,
}

impl<'engine> Supervisor<'engine> {
    pub fn new(engine: &'engine Engine) -> Self {
        Supervisor {
            engine,
            slots: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn view(&self, id: ViewId) -> Result<&View> {
        self.slots
            .get(id.0)
            .map(|slot| &slot.view)
            .ok_or(Error::NoSuchView(id.0))
    }

    pub fn view_mut(&mut self, id: ViewId) -> Result<&mut View> {
        self.slots
            .get_mut(id.0)
            .map(|slot| &mut slot.view)
            .ok_or(Error::NoSuchView(id.0))
    }

    pub fn ids(&self) -> impl Iterator<Item = ViewId> + '_ {
        (0..self.slots.len()).map(ViewId)
    }

    /// Open a view on `uri`, waiting until it has painted.
    ///
    /// **This is where the process→view map is built, because neither libwpe
    /// nor WebKit reports it.** The web processes that appear between the
    /// snapshot before the view is created and the snapshot after it has
    /// painted are attributed to this view. That is a best-effort diff and it
    /// is honest about it: [`Supervisor::web_process_of`] returns `Option`, and
    /// a concurrent open elsewhere could confuse the attribution. It is
    /// nonetheless the only mapping available — the alternative is none.
    pub fn open(&mut self, uri: &str, width: u32, height: u32, timeout: Duration) -> Result<ViewId> {
        let before: Vec<u32> = web_processes().into_iter().map(|p| p.pid).collect();

        let mut view = self.engine.view(width, height)?;
        view.load_uri(uri)?;
        self.slots.push(Slot {
            view,
            uri: uri.to_string(),
            web_process: None,
        });
        let id = ViewId(self.slots.len() - 1);

        let painted = self.pump_until(timeout, |sup| {
            sup.view(id).is_ok_and(|v| v.painted_current_document())
        });
        if !painted {
            return Err(Error::NeverPainted {
                uri: uri.to_string(),
                frames: self.view(id).map(|v| v.frames_exported()).unwrap_or(0),
                blank: self.view(id).map(|v| v.blank_frames_skipped()).unwrap_or(0),
            });
        }

        self.settle();
        let after = web_processes();
        let fresh: Vec<u32> = after
            .iter()
            .map(|p| p.pid)
            .filter(|pid| !before.contains(pid))
            .collect();
        self.slots[id.0].web_process = fresh.first().copied();
        Ok(id)
    }

    /// The web process attributed to this view, if the open diff caught one.
    pub fn web_process_of(&self, id: ViewId) -> Option<u32> {
        self.slots.get(id.0).and_then(|slot| slot.web_process)
    }

    /// Every web-content process below us.
    pub fn web_processes(&self) -> Vec<WebProcess> {
        web_processes()
    }

    /// Views whose web process has died, per WebKit's own signal.
    pub fn terminated(&self) -> Vec<ViewId> {
        self.ids()
            .filter(|id| {
                self.view(*id)
                    .is_ok_and(|view| view.web_process_terminated())
            })
            .collect()
    }

    /// Restart a view whose web process died.
    ///
    /// EXPLICIT by design — see the type doc. Spike C proved a reload restores
    /// the view completely (it paints again and answers input again) on a fresh
    /// pid, so this waits for the repaint and re-attributes the process.
    pub fn restart(&mut self, id: ViewId, timeout: Duration) -> Result<()> {
        let before: Vec<u32> = web_processes().into_iter().map(|p| p.pid).collect();
        let uri = self
            .slots
            .get(id.0)
            .map(|slot| slot.uri.clone())
            .ok_or(Error::NoSuchView(id.0))?;
        {
            let view = self.view_mut(id)?;
            view.forget_frame();
            // ⚠ NOT `reload()`. A view whose web process was KILLED has no
            // document left to reload: the reload completes against nothing and
            // the view settles on a blank WHITE page, reporting a successful
            // restart of a surface that is still empty. Re-navigating to the
            // view's own URI recovers it properly.
            view.clear_termination();
            view.load_uri(&uri)?;
        }
        // Not "has a frame" — has a frame painted AFTER the load finished. A
        // crashed view repaints WHITE on the way back, and returning on that
        // frame would report a restored surface that is still blank.
        let painted = self.pump_until(timeout, |sup| {
            sup.view(id).is_ok_and(|v| v.painted_current_document())
        });
        if !painted {
            return Err(Error::RestartFailed(id.0));
        }
        self.settle();
        let fresh: Vec<u32> = web_processes()
            .into_iter()
            .map(|p| p.pid)
            .filter(|pid| !before.contains(pid))
            .collect();
        self.slots[id.0].web_process = fresh.first().copied();
        Ok(())
    }

    /// Kill a view's web process — a supervision primitive for recovery drills
    /// and for shedding a wedged surface.
    pub fn kill_web_process_of(&self, id: ViewId) -> Result<u32> {
        let pid = self.web_process_of(id).ok_or(Error::NoWebProcess(id.0))?;
        // SAFETY: the pid came from our own descendant walk.
        unsafe { crate::ffi::kill(pid as i32, 9) };
        Ok(pid)
    }

    /// Pump briefly after a load reports finished.
    ///
    /// A document can paint more than once as it completes — recovering a killed
    /// view paints WHITE and then the real page — so returning on the first
    /// frame that belongs to the new document can still hand back an
    /// intermediate one. This window lets the settled paint replace it. Short,
    /// bounded and deliberate: not a retry loop, just the tail of a load.
    fn settle(&self) {
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            self.engine.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pump the main loop until `done` or the timeout expires.
    pub fn pump_until(&self, timeout: Duration, mut done: impl FnMut(&Self) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.engine.pump();
            if done(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done(self)
    }

    /// Pump until this view's next painted frame satisfies `accept`.
    ///
    /// Forgets the current frame first, so a stale frame from before the input
    /// under test can never satisfy the wait.
    pub fn await_frame(
        &mut self,
        id: ViewId,
        timeout: Duration,
        accept: impl Fn(&crate::Frame) -> bool,
    ) -> Result<()> {
        self.view_mut(id)?.forget_frame();
        let ok = self.pump_until(timeout, |sup| {
            sup.view(id)
                .ok()
                .and_then(|v| v.last_frame())
                .is_some_and(&accept)
        });
        if ok {
            Ok(())
        } else {
            Err(Error::FrameNeverMatched(id.0))
        }
    }
}

/// Every descendant of this process that serves web content.
///
/// ⚠ **A web process is NOT a direct child.** WebKit launches each one inside
/// bubblewrap, so the tree is `app -> bwrap -> WPEWebProcess`, and `comm` is
/// truncated to 15 characters. A direct-children scan finds only `bwrap` and
/// reports zero web processes — which is exactly what spike C did on its first
/// attempt. Walk descendants; match on a prefix.
pub fn web_processes() -> Vec<WebProcess> {
    descendants()
        .into_iter()
        .filter(|p| p.comm.starts_with("WPEWebProcess"))
        .collect()
}

/// Every descendant process, at any depth.
pub fn descendants() -> Vec<WebProcess> {
    let mut all: Vec<WebProcess> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // Parse from the LAST ')': field 2 is the executable name in parens and
        // may itself contain spaces and parens.
        let Some((before, after)) = stat.rsplit_once(')') else {
            continue;
        };
        let comm = before
            .split_once('(')
            .map(|(_, c)| c.to_string())
            .unwrap_or_default();
        let mut fields = after.split_whitespace();
        let _state = fields.next();
        let ppid: u32 = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        all.push(WebProcess { pid, comm, ppid });
    }

    let me = std::process::id();
    let mut seen = vec![me];
    let mut out: Vec<WebProcess> = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for candidate in &all {
            if seen.contains(&candidate.ppid) && !seen.contains(&candidate.pid) {
                seen.push(candidate.pid);
                out.push(candidate.clone());
                changed = true;
            }
        }
    }
    out.sort_by_key(|p| p.pid);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descendant walk must find a grandchild, because a web process IS
    /// one. A direct-children implementation passes every "does it find my
    /// child" test and still reports zero web processes in production.
    #[test]
    fn the_walk_reaches_grandchildren_not_just_children() {
        // bash -c 'sleep 5' gives us bash (child) -> sleep (grandchild).
        let mut child = std::process::Command::new("bash")
            .args(["-c", "sleep 5"])
            .spawn()
            .expect("spawn bash");
        std::thread::sleep(Duration::from_millis(400));

        let found = descendants();
        let child_pid = child.id();
        assert!(
            found.iter().any(|p| p.pid == child_pid),
            "the direct child was not found at all: {found:?}",
        );
        assert!(
            found.iter().any(|p| p.comm.starts_with("sleep")),
            "the GRANDCHILD was not found — a direct-children walk would pass every \
             other test and still report zero web processes in production: {found:?}",
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_process_with_no_children_has_no_descendants_of_interest() {
        // Nothing named like a web process should appear when none is running.
        assert!(
            web_processes().is_empty(),
            "no engine is running in this test, so there can be no web processes",
        );
    }
}
