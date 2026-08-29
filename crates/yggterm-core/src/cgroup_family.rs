//! The GUI family's own cgroup shape: children `gui` / `web` / `helpers`
//! under the scope the memory-scope arm already creates, with the `memory`
//! controller enabled on the scope and a committed bound on the `web` child.
//!
//! **Why this exists.** WebKit's own memory policy reads `VmRSS`/`statm` only,
//! so on a host that swaps, the kernel evicts cold pages, RSS falls, and the
//! threshold never fires while the committed footprint climbs — measured
//! 649 → 1 362 MB committed against a flat 586–714 MB RSS band
//! (`docs/pending-bugs.md` 6.7). A cgroup v2 bound does not have that blind
//! spot: `memory.high` + `memory.swap.max` together bound the two halves of
//! what the machine actually committed.
//!
//! **The kernel constraints this module is shaped around — all measured live,
//! on the GUI host and on dev, throwaway scopes, zero residue
//! (`~/.yggterm/scratchpad/cgroup-family-probe4.sh`):**
//!
//! 1. A cgroup with controllers enabled in `cgroup.subtree_control` may
//!    contain NO internal processes (`EBUSY` otherwise). ⇒ The scope must be
//!    emptied into children BEFORE `+memory` is enabled: the GUI migrates
//!    itself into `gui` first, and everything born later inherits `gui` and is
//!    migrated to `web` / `helpers` by the family sweep.
//! 2. Controller files (`memory.high`, `memory.swap.max`, `memory.current`,
//!    …) do not exist in a child until the controller is enabled in the
//!    PARENT's `subtree_control`. ⇒ Bound writes come after the enable,
//!    never before.
//! 3. Created children and their files are owned by the creating user inside
//!    the private `systemd-run --user --scope` unit — every operation here is
//!    unprivileged. On the plain login `session-<id>.scope` this whole shape
//!    is impossible unprivileged (the directory is root-owned and the scope
//!    holds unrelated processes), which is why the caller arms it only inside
//!    the private scope.
//!
//! **What is NOT testable in CI:** the kernel semantics above (EBUSY ordering,
//! page-rounded readbacks). The file-level contract IS: every operation is a
//! plain write/read on a path, so the tests below run against synthetic trees
//! and lock the shapes, and the live probe carries the order.

use std::io;
use std::path::Path;

/// The family children, in arm order. `gui` first: the GUI migrates itself
/// there before the controller can be enabled, which is what makes the enable
/// legal.
pub const FAMILY_CHILDREN: [&str; 3] = ["gui", "web", "helpers"];

/// The child the heavy web processes are migrated into.
pub const WEB_CHILD: &str = "web";

/// Everything the family arm needs to say on the trace, so the startup event
/// answers "was the family shape armed, and if not, why not" without a second
/// event name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FamilyArmReport {
    /// Whether the shape ended armed: children exist, `+memory` enabled, the
    /// `web` bound set, and the GUI inside `gui`.
    pub armed: bool,
    /// The children that exist after the attempt.
    pub children: Vec<String>,
    /// The `web` child's `memory.high` in bytes as the kernel confirmed it
    /// (page-rounded), when it was set.
    pub web_high_bytes: Option<u64>,
    /// The `web` child's `memory.swap.max` in bytes, as confirmed.
    pub web_swap_max_bytes: Option<u64>,
    /// Why the shape is not armed, when it is not. One line, trace-ready.
    pub error: Option<String>,
}

/// Which family child a process belongs in, from its `comm`.
///
/// Prefix matching only — `/proc/<pid>/comm` is truncated to 15 bytes by the
/// kernel (`WebKitWebProcess` arrives as `WebKitWebProces`), the same trap
/// `memory_profile::Role::classify` documents. `None` means "leave where it
/// is": the GUI itself placed itself at arm time, and plain forks (ssh
/// transports, agent CLIs) stay in `gui` — accounted by the family, bounded
/// only by the scope, until their own committed series earns them a child.
pub fn family_child_for_comm(comm: &str) -> Option<&'static str> {
    const HELPERS: [&str; 7] = [
        "bwrap",
        "glycin",
        "xdg-desktop-por",
        "ksecretd",
        "at-spi-bus-laun",
        "at-spi2-registr",
        "xdg-permission-",
    ];
    if comm.starts_with("WebKitWebProc")
        || comm.starts_with("WebKitNetwork")
        || comm.starts_with("WebKitGPU")
    {
        return Some(WEB_CHILD);
    }
    if HELPERS.iter().any(|h| comm.starts_with(h)) {
        return Some("helpers");
    }
    None
}

/// The `0::` unified-hierarchy line of `/proc/<pid>/cgroup`, as a filesystem
/// path under the cgroup2 mount.
pub fn own_cgroup_path(pid: i32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let line = text.lines().find(|line| line.starts_with("0::"))?;
    let path = line.splitn(3, ':').nth(2)?;
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Basename of a cgroup path — the scope or child a pid currently sits in.
pub fn cgroup_basename(cgroup_path: &str) -> &str {
    cgroup_path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(cgroup_path)
}

/// Where a pid currently is, as a basename (`gui`, `web`, a scope name, …).
pub fn current_child_basename(pid: i32) -> Option<String> {
    own_cgroup_path(pid).map(|path| cgroup_basename(&path).to_string())
}

/// Create the family children under `scope_root` (a filesystem path).
///
/// Existing children are fine — an idempotent re-arm over an already-armed
/// family (adoption restarts) must not fail on them.
pub fn create_children<P: AsRef<Path>>(scope_root: P) -> io::Result<Vec<String>> {
    let mut created = Vec::new();
    for name in FAMILY_CHILDREN {
        let dir = scope_root.as_ref().join(name);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        created.push(name.to_string());
    }
    Ok(created)
}

/// Migrate `pid` into `child` (a filesystem path): write the pid to
/// `cgroup.procs`. The write is the whole migration — measured clean and
/// atomic on live processes, including mid-session. Appended, not truncated:
/// the kernel treats the file as a command port (position and truncation are
/// meaningless to it), and append keeps a synthetic tree honest as a log of
/// the commands issued.
pub fn migrate_pid<P: AsRef<Path>>(child: P, pid: i32) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(child.as_ref().join("cgroup.procs"))?;
    io::Write::write_all(&mut file, format!("{pid}\n").as_bytes())
}

/// Enable the `memory` controller on `scope_root`'s subtree.
///
/// `EBUSY` means the scope still holds an internal process — the constraint
/// that decides the whole arm order. Surfaced as
/// [`CgroupFamilyError::InternalProcesses`] so the trace can say that and not
/// a generic io error.
pub fn enable_memory_controller<P: AsRef<Path>>(scope_root: P) -> Result<(), CgroupFamilyError> {
    write_control(scope_root.as_ref().join("cgroup.subtree_control"), "+memory\n")
}

/// Disable it again (rollback paths).
pub fn disable_memory_controller<P: AsRef<Path>>(scope_root: P) -> Result<(), CgroupFamilyError> {
    write_control(scope_root.as_ref().join("cgroup.subtree_control"), "-memory\n")
}

fn write_control(path: impl AsRef<Path>, value: &str) -> Result<(), CgroupFamilyError> {
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    if let Err(error) = io::Write::write_all(&mut file, value.as_bytes()) {
        return if error.kind() == io::ErrorKind::ResourceBusy {
            Err(CgroupFamilyError::InternalProcesses)
        } else {
            Err(CgroupFamilyError::Io(error))
        };
    }
    Ok(())
}

/// Set the `web` child's committed bound. Both files exist only after
/// [`enable_memory_controller`] — the kernel creates controller files in a
/// child when the controller joins the PARENT's subtree — and the kernel
/// page-rounds what it stores, so the caller reads back through
/// [`read_child_bound`] rather than trusting the input.
pub fn set_web_bounds<P: AsRef<Path>>(
    web_child: P,
    high_bytes: u64,
    swap_max_bytes: u64,
) -> io::Result<()> {
    std::fs::write(
        web_child.as_ref().join("memory.high"),
        format!("{high_bytes}\n"),
    )?;
    std::fs::write(
        web_child.as_ref().join("memory.swap.max"),
        format!("{swap_max_bytes}\n"),
    )
}

/// Read back a bound the kernel actually stored. `max`, empty and unreadable
/// all mean NO bound — blind is not bounded, the same rule
/// `cgroup_memory_high_is_a_bound` applies to the scope.
pub fn read_child_bound<P: AsRef<Path>>(child: P, file: &str) -> Option<u64> {
    let text = std::fs::read_to_string(child.as_ref().join(file)).ok()?;
    let value = text.trim();
    if value.is_empty() || value == "max" {
        return None;
    }
    value.parse::<u64>().ok().filter(|bytes| *bytes > 0)
}

/// The derived `web`-child bound: the same single-web-process share the
/// engine's own policy already grants (`MemTotal/8`) as the resident `high`,
/// with at most half of it as swap — so the committed ceiling for the whole
/// web plane lands below the family scope's (`2 × MemTotal/8` resident
/// + `MemTotal/8` swap, see the scope bound's derivation in the GUI entry)
/// and the child's bound engages BEFORE the scope's.
///
/// Measured context, not a fitted constant: the worst single web process on
/// the GUI host reached ≈ 2.0 GiB committed at the 95 %-RAM horizon and the
/// family ≈ 3.5 GiB, so on the 14.8 GB host this bound (1 856 MB resident
/// + 928 MB swap) sits above today's steady state and below the scope
/// ceiling — it bounds the unbounded, it does not throttle the working app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebChildBound {
    pub high_bytes: u64,
    pub swap_max_bytes: u64,
}

/// The small-machine floor, the same one the WebKit policy reserves for the
/// smallest supported hosts.
pub const MIN_WEB_CHILD_HIGH_MB: u64 = 768;

pub fn web_child_bound(mem_total_kb: Option<u64>) -> WebChildBound {
    let web_share_mb = match mem_total_kb {
        Some(kb) if kb > 0 => (kb / 1024) / 8,
        _ => 1024,
    };
    let high_mb = web_share_mb.max(MIN_WEB_CHILD_HIGH_MB);
    WebChildBound {
        high_bytes: high_mb * 1024 * 1024,
        // Half the resident share, floored at the same ratio the scope uses.
        swap_max_bytes: (high_mb / 2).max(MIN_WEB_CHILD_HIGH_MB / 2) * 1024 * 1024,
    }
}

/// Everything that can go wrong while arming, with the one kernel constraint
/// that deserves its own name.
#[derive(Debug, thiserror::Error)]
pub enum CgroupFamilyError {
    /// `+memory` refused: the scope still holds an internal process. The arm
    /// order (children first, self-migration second, enable last) exists to
    /// make this impossible; seeing it means a family member re-entered the
    /// scope root between the migration and the enable.
    #[error("the scope still holds an internal process (EBUSY from subtree_control)")]
    InternalProcesses,
    #[error("cgroup control write failed: {0}")]
    Io(#[from] io::Error),
}

/// Every pid currently a member of `scope_root`, from its `cgroup.procs`.
/// An absent file means no members — a real cgroup always carries the file,
/// and a synthetic tree (the tests') may not.
pub fn scope_root_pids<P: AsRef<Path>>(scope_root: P) -> io::Result<Vec<i32>> {
    let text = match std::fs::read_to_string(scope_root.as_ref().join("cgroup.procs")) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(text
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .collect())
}

/// Arm the whole shape in the measured order, from `scope_root`, draining the
/// scope root into `gui`. Idempotent: an already-armed family re-arms cleanly
/// (existing children tolerated, the enable is a no-op write, bounds are
/// rewritten).
///
/// ⛔ **THE ROOT IS DRAINED, NOT MERELY SELF-MIGRATED** — and that is not a
/// style choice, it is the measured failure: on the GUI host (3.2.7, first
/// live arm) the launch chain's own waiter (`~/.local/bin/yggterm`, `ppid=1`,
/// alive for the scope's whole life) sat at the scope root beside the arming
/// process, and the self-only migration left it there — `+memory` answered
/// EBUSY and the family armed with NO bound on the web child. The constraint
/// is "the scope holds no internal processes", so the arm empties the root
/// completely: every member moves into `gui` (the arming process among them),
/// and only then can the controller come on.
///
/// Every failure leaves the family as it was and reports why — an unbounded
/// GUI that says so is the same contract the scope arm ships.
pub fn arm_family<P: AsRef<Path>>(scope_root: P, self_pid: i32) -> FamilyArmReport {
    let scope_root = scope_root.as_ref();
    let mut report = FamilyArmReport::default();
    let fail = |report: FamilyArmReport, error: String| FamilyArmReport {
        armed: false,
        error: Some(error),
        ..report
    };

    report.children = match create_children(scope_root) {
        Ok(children) => children,
        Err(error) => return fail(report, format!("children could not be created ({error})")),
    };

    // Drain the scope root into `gui` — every member, the arming process
    // included (its own move is the same write; into the same child, it is a
    // no-op if already there). A member that exits mid-drain is gone, not an
    // error; a pid that cannot be read back simply does not move.
    let gui_child = scope_root.join("gui");
    match scope_root_pids(scope_root) {
        Ok(pids) => {
            for pid in pids {
                let _ = migrate_pid(&gui_child, pid);
            }
        }
        Err(error) => {
            return fail(report, format!("the scope root could not be read ({error})"))
        }
    }
    if let Err(error) = migrate_pid(&gui_child, self_pid) {
        return fail(report, format!("the GUI could not enter the gui child ({error})"));
    }
    if let Err(error) = enable_memory_controller(scope_root) {
        return fail(report, format!("+memory was refused ({error})"));
    }
    let bound = web_child_bound(read_mem_total_kb());
    if let Err(error) =
        set_web_bounds(scope_root.join(WEB_CHILD), bound.high_bytes, bound.swap_max_bytes)
    {
        return fail(report, format!("the web child bound was refused ({error})"));
    }
    report.web_high_bytes = read_child_bound(scope_root.join(WEB_CHILD), "memory.high");
    report.web_swap_max_bytes = read_child_bound(scope_root.join(WEB_CHILD), "memory.swap.max");
    if report.web_high_bytes.is_none() || report.web_swap_max_bytes.is_none() {
        return fail(
            report,
            "the web child bound did not take (readback empty or max)".to_string(),
        );
    }
    report.armed = true;
    report
}

/// `MemTotal` in kB, the one input every bound here derives from. `None` when
/// `/proc/meminfo` would not answer, which every derivation must survive with
/// a floor rather than a zero. THE one owner of that question — the GUI's
/// scope and WebKit derivations read it from here.
pub fn read_mem_total_kb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo.lines().find_map(|line| {
        let rest = line.strip_prefix("MemTotal:")?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

/// Whether THIS process armed (or adopted) the family shape. A process-global
/// flag, not shell state: the render probe must not dirty a signal to learn
/// something the startup already knew, and the flag costs one atomic read per
/// tick.
static FAMILY_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_family_armed(armed: bool) {
    FAMILY_ARMED.store(armed, std::sync::atomic::Ordering::Release);
}

pub fn family_armed() -> bool {
    FAMILY_ARMED.load(std::sync::atomic::Ordering::Acquire)
}

/// The sweep tick. Ten seconds: a WebKit child is born on a surface mount and
/// the lag between its birth and its bound is what the scope-level ceiling
/// covers anyway, so faster buys nothing and slower loiters in an unbounded
/// child during exactly the traffic that grows it.
const FAMILY_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// Spawn the family sweep as a plain thread of the arming process.
///
/// ⛔ **WHY NOT THE RENDER PROBE'S LOOP — it was tried, and it was measured
/// dead on the first live arm (3.2.8):** the probe's whole body sits behind
/// the profiling toggle, and the memory bound is not a profiling feature.
/// Moving the walk before the gate would have coupled the bound to the
/// probe's lifecycle, its sampling, and a toggle nobody thinks about when
/// asking "why is my web plane unbounded". The sweep gets its own thread: it
/// outlives nothing, it reads nothing but `/proc` and cgroupfs, and it dies
/// with the process.
///
/// `home` is the perf home the `family_migration` events land in.
pub fn spawn_family_sweep(home: std::path::PathBuf) {
    let _ = std::thread::Builder::new()
        .name("cgroup-family-sweep".to_string())
        .spawn(move || {
            let self_pid = std::process::id() as i32;
            loop {
                std::thread::sleep(FAMILY_SWEEP_INTERVAL);
                // The stats-only walk: pid + ppid + comm per family member —
                // the ONE owner of the tree walk (`render_probe`) is reused,
                // not grown a second copy.
                for (pid, comm) in crate::render_probe::observe_process_tree_stats(self_pid)
                    .into_iter()
                    .map(|stat| (stat.pid, stat.comm))
                {
                    let Some(child) = family_child_for_comm(&comm) else {
                        continue;
                    };
                    if current_child_basename(pid).as_deref() == Some(child) {
                        continue;
                    }
                    if migrate_misplaced_one(pid, &comm) {
                        let _ = crate::trace::append_trace_event(
                            &home,
                            "gui",
                            "memory",
                            "family_migration",
                            serde_json::json!({
                                "pid": pid,
                                "comm": comm,
                                "to": child,
                            }),
                        );
                    }
                }
            }
        });
}

/// Migrate ONE misplaced member into the child its comm names. The threaded
/// sweep's per-member body: silent on a member that exited mid-sweep (the
/// common race, not an error).
fn migrate_misplaced_one(pid: i32, comm: &str) -> bool {
    if !family_armed() {
        return false;
    }
    let Some(child) = family_child_for_comm(comm) else {
        return false;
    };
    if current_child_basename(pid).as_deref() == Some(child) {
        return false;
    }
    // The children are SIBLINGS of this process's own child: after the arm
    // the GUI sits in `<scope>/gui`, so the scope root is one level up. A
    // process that is not in `gui` is not running an armed family (a stale
    // flag across an exec boundary) and must not write into a guessed path.
    // ⛔ `own_cgroup_path` returns the path INSIDE the hierarchy — the
    // filesystem path needs the mount prefix, or the write lands on a
    // relative path against the CWD and answers ENOENT (measured live, 3.2.9:
    // the sweep ticked, classified, and every write went nowhere).
    let Some(own) = own_cgroup_path(std::process::id() as i32) else {
        return false;
    };
    let Some(target) = sweep_target_path(&own, child) else {
        return false;
    };
    migrate_pid(target, pid).is_ok()
}

/// The filesystem path a sweep member migrates to, from the arming process's
/// own cgroup path and the child it belongs in.
///
/// ⛔ THE MOUNT PREFIX IS THE TEST. `own_cgroup_path` returns the path INSIDE
/// the cgroup hierarchy (`/user.slice/…/yggterm-gui-<pid>.scope/gui`); the
/// filesystem path needs `/sys/fs/cgroup` glued in front, and the first live
/// sweep shipped without it — every write answered ENOENT against a relative
/// path while the thread ticked, classified, and "succeeded" at nothing. A
/// silent no-op is the worst failure a bound can have, so the shape of the
/// path is locked, not trusted.
fn sweep_target_path(own_cgroup: &str, child: &str) -> Option<String> {
    let (scope_root, base) = own_cgroup.rsplit_once('/')?;
    (base == "gui").then(|| format!("/sys/fs/cgroup{scope_root}/{child}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic cgroup tree: the file-level contract of every operation in
    /// this module is a plain filesystem write, so the tests run against a
    /// temp tree and lock the shapes. The KERNEL semantics (EBUSY on an
    /// internal process, page-rounded readbacks, controller files appearing
    /// only after the enable) are measured live on the GUI host and on dev by
    /// `~/.yggterm/scratchpad/cgroup-family-probe4.sh`; the pending-bugs 6.7
    /// entry carries the falsifying observation so the probe cannot rot
    /// silently.
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        pub struct TempDir {
            path: PathBuf,
        }

        impl TempDir {
            pub fn new(label: &str) -> std::io::Result<TempDir> {
                let n = COUNTER.fetch_add(1, Ordering::Relaxed);
                let path =
                    std::env::temp_dir().join(format!("ygg-{label}-{}-{n}", std::process::id()));
                std::fs::create_dir_all(&path)?;
                Ok(TempDir { path })
            }
            pub fn path(&self) -> &Path {
                &self.path
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    fn synthetic_scope() -> (tempdir::TempDir, std::path::PathBuf) {
        let dir = tempdir::TempDir::new("cgroup-family").expect("tempdir");
        let root = dir.path().join("yggterm-gui-123.scope");
        std::fs::create_dir_all(&root).expect("scope dir");
        // The files the kernel provides at the scope level.
        std::fs::write(root.join("cgroup.subtree_control"), "").expect("subtree_control");
        (dir, root)
    }

    #[test]
    fn the_arm_order_is_children_then_self_migration_then_enable_then_bounds() {
        let (_dir, root) = synthetic_scope();
        // On the synthetic tree every step is a plain file write, which is
        // exactly the file-level contract being locked; the kernel semantics
        // behind the order are the live probe's to carry.
        let report = arm_family(&root, 4242);
        assert!(
            report.armed,
            "on a synthetic tree every step is a plain write, so the arm must \
             complete; it said: {:?}",
            report.error
        );
        assert_eq!(
            report.children,
            vec!["gui".to_string(), "web".to_string(), "helpers".to_string()],
            "all three children, in arm order: {report:?}"
        );
        assert!(
            report.web_high_bytes.is_some() && report.web_swap_max_bytes.is_some(),
            "the bounds were confirmed by readback: {report:?}"
        );
        // The order left its mark: the subtree was enabled and the bounds were
        // written, not merely attempted.
        assert_eq!(
            std::fs::read_to_string(root.join("cgroup.subtree_control")).expect("subtree"),
            "+memory\n",
            "the enable is on the record"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("gui/cgroup.procs")).expect("gui procs"),
            "4242\n",
            "the GUI entered the gui child before the enable, which is what makes \
             the enable legal"
        );
    }

    #[test]
    fn the_scope_root_is_drained_not_merely_self_migrated() {
        // THE 3.2.7 FIRST-LIVE-ARM DEFECT: the launch chain's waiter sat at
        // the scope root beside the arming process; a self-only migration
        // left it there and `+memory` answered EBUSY — the family armed with
        // no bound at all. The arm empties the ROOT: every member moves into
        // `gui` before the enable, whatever it is and however it got there.
        let (_dir, root) = synthetic_scope();
        std::fs::write(root.join("cgroup.procs"), "111\n4242\n999\n").expect("root members");
        let report = arm_family(&root, 4242);
        assert!(report.armed, "the drain makes the enable legal: {report:?}");
        // The kernel empties the root as the SIDE EFFECT of the migrations;
        // the file-level contract here is that a migration command was issued
        // for EVERY root member before the enable.
        let gui_members =
            std::fs::read_to_string(root.join("gui/cgroup.procs")).expect("gui procs");
        for pid in ["111", "4242", "999"] {
            assert!(
                gui_members.split_whitespace().any(|m| m == pid),
                "scope-root member {pid} must have a migration command issued \
                 into gui before the enable; gui held: {gui_members:?}"
            );
        }
    }

    #[test]
    fn a_failed_enable_names_the_internal_process_constraint_not_the_syscall() {
        let error = CgroupFamilyError::InternalProcesses;
        let text = error.to_string();
        assert!(
            text.contains("internal process"),
            "the trace must be able to say WHY, in the domain's words: {text}"
        );
    }

    #[test]
    fn the_web_child_bound_engages_below_the_family_scope_on_every_host() {
        // The scope rule this must sit under lives in the GUI entry
        // (`memory_scope_policy`): high = 2 × web share, swap = 1 × web share.
        // The child takes 1 × share resident + ½ × share swap, so its
        // committed ceiling is strictly below the scope's on every host size
        // and the child bound always fires first.
        for total_gb in [2u64, 8, 16, 32, 64] {
            let kb = total_gb * 1024 * 1024;
            let bound = web_child_bound(Some(kb));
            let web_share_mb = (kb / 1024) / 8;
            let scope_high_mb = (web_share_mb * 2).max(1536);
            let scope_swap_mb = web_share_mb.max(768);
            assert!(
                bound.high_bytes > 0 && bound.swap_max_bytes > 0,
                "{total_gb} GB host: a bound of nothing bounds nothing: {bound:?}"
            );
            let child_committed = bound.high_bytes + bound.swap_max_bytes;
            let scope_committed = (scope_high_mb + scope_swap_mb) * 1024 * 1024;
            assert!(
                child_committed < scope_committed,
                "{total_gb} GB host: the web child's committed ceiling ({child_committed}) \
                 must engage below the family scope's ({scope_committed}), or the child \
                 bound can never fire first"
            );
            assert!(
                bound.high_bytes >= MIN_WEB_CHILD_HIGH_MB * 1024 * 1024,
                "{total_gb} GB host: the small-machine floor holds: {bound:?}"
            );
        }
        let unknown = web_child_bound(None);
        assert!(
            unknown.high_bytes >= MIN_WEB_CHILD_HIGH_MB * 1024 * 1024,
            "an unreadable meminfo takes the floor, never zero: {unknown:?}"
        );
    }

    #[test]
    fn family_classification_prefixes_survive_the_15_byte_comm_truncation() {
        assert_eq!(family_child_for_comm("WebKitWebProcess"), Some("web"));
        assert_eq!(
            family_child_for_comm("WebKitWebProces"),
            Some("web"),
            "the truncated comm a real kernel reports"
        );
        assert_eq!(family_child_for_comm("WebKitNetworkProcess"), Some("web"));
        assert_eq!(family_child_for_comm("WebKitGPU"), Some("web"));
        assert_eq!(family_child_for_comm("bwrap"), Some("helpers"));
        assert_eq!(family_child_for_comm("glycin-svg"), Some("helpers"));
        assert_eq!(family_child_for_comm("yggterm"), None, "the GUI placed itself at arm time");
        assert_eq!(
            family_child_for_comm("ssh"),
            None,
            "plain transports stay in gui, accounted, unbounded"
        );
        assert_eq!(
            family_child_for_comm("claude"),
            None,
            "agent CLIs stay in gui until their own committed series earns a child"
        );
    }

    #[test]
    fn read_child_bound_treats_max_empty_and_unreadable_as_no_bound() {
        let (_dir, root) = synthetic_scope();
        std::fs::create_dir_all(root.join("web")).expect("web");
        std::fs::write(root.join("web/memory.high"), "max").expect("max");
        assert_eq!(
            read_child_bound(root.join("web"), "memory.high"),
            None,
            "max is the kernel's word for NO bound"
        );
        std::fs::write(root.join("web/memory.high"), "3776000000\n").expect("value");
        assert_eq!(
            read_child_bound(root.join("web"), "memory.high"),
            Some(3_776_000_000),
            "a real number is a real bound"
        );
        assert_eq!(
            read_child_bound(root.join("web"), "memory.peak"),
            None,
            "an absent file is not a zero"
        );
    }

    #[test]
    fn create_children_is_idempotent_over_an_already_armed_family() {
        let (_dir, root) = synthetic_scope();
        let first = create_children(&root).expect("first arm");
        let second = create_children(&root).expect("re-arm");
        assert_eq!(first, second, "a re-arm over an armed family must not fail");
    }

    #[test]
    fn the_sweep_is_a_noop_until_the_family_is_armed() {
        // CI runs unscoped: the flag is off, so the sweep must touch nothing —
        // not read /proc for its own cgroup, not write a single cgroup file.
        set_family_armed(false);
        assert!(!super::migrate_misplaced_one(
            std::process::id() as i32,
            "WebKitWebProcess"
        ));
        // And the flag round-trips, because the GUI arm and the sweep thread
        // are two call sites that must never disagree about which state they
        // are in.
        set_family_armed(true);
        assert!(family_armed());
        set_family_armed(false);
    }

    #[test]
    fn classification_leaves_the_gui_itself_unmoved() {
        // The GUI migrated itself into `gui` at arm time; the sweep re-reading
        // it must not fight that placement (a self-migration war would spin).
        assert_eq!(family_child_for_comm("yggterm"), None);
        assert_eq!(family_child_for_comm("yggterm-headles"), None, "truncated to 15 bytes too");
    }

    #[test]
    fn the_sweep_target_carries_the_mount_prefix_and_only_fires_inside_gui() {
        // THE 3.2.9 FIRST-SWEEP DEFECT: the migration path was built WITHOUT
        // the /sys/fs/cgroup mount prefix, so every write answered ENOENT
        // against a path relative to the process CWD while the thread ticked,
        // classified correctly, and moved nothing — a silent no-op, the worst
        // failure a bound can have. The prefix is now the lock.
        let own = "/user.slice/user-1000.slice/user@1000.service/app.slice/yggterm-gui-123.scope/gui";
        assert_eq!(
            sweep_target_path(own, "web"),
            Some(format!(
                "/sys/fs/cgroup{own}",
                own = own.strip_suffix("/gui").expect("the gui child")
            ) + "/web"),
            "the target is the sibling web child under the SAME scope, on the \
             mounted filesystem"
        );
        assert_eq!(
            sweep_target_path(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/yggterm-gui-123.scope",
                "web"
            ),
            None,
            "a process NOT inside the gui child is not running an armed family: \
             the sweep must refuse to guess a scope root"
        );
    }
}
