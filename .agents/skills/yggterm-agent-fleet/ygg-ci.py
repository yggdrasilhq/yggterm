#!/usr/bin/env python3
"""ygg-ci — the fleet's single build plane.

Like booter/monitor, this is a DETACHED watcher that owns the one daemon slot.

DEFECT THIS EXISTS FOR:
  N lanes × N cores × N worktrees each doing `cargo build && deploy` over the
  same 6 binary paths on 3 hosts. Two interleaving deploys write a fleet no
  tree ever held (deploy-fleet lease exists for this), per-worktree `target/`
  burns disk and collides, and a build in lane A replaces the daemon lane B is
  testing. Measured: a lane deploying from a stale checkout wrote 3.1.60 over
  3.1.61 fleet-wide, and the census named a version that was a mixture.

SHAPE:
  N lanes SUBSCRIBE a branch -> ONE watcher on dev wakes on a timer (not a
  burn), merges main + subs into an ephemeral integration worktree under
  ~/.yggterm/scratchpad/ci, builds ONCE, deploys via the project's own deploy
  script (yggterm: scripts/deploy-fleet.sh) which already proves identity and
  converges the fleet. Many agents test the same artefact.

  Host: dev is the auto host for all ci builds. The watcher only builds on dev;
  any agent on any fleet host subscribes by ssh-ing to dev.

  Project recipe: any gitcoding project defines how it builds. The ci.json in
  the ci state (and optionally .ygg-ci.json in the repo) carries build/deploy
  commands. When the service is present other agents can subscribe to it.

  Timer: watch sleeps interval (default 300s, tunable per project). A tick that
  finds nothing dirty does ~fetch + json stat and returns 0 — same shape as
  booter/monitor, no core burn.

Usage:
  ygg-ci.py subscribe --lane lane/foo --project yggterm [--repo ~/gh/yggterm]
  ygg-ci.py unsubscribe --lane lane/foo --project yggterm
  ygg-ci.py list [--project yggterm] [--json]
  ygg-ci.py status [--json]
  ygg-ci.py tick [--project yggterm] [--dry-run]
  ygg-ci.py watch [--interval 300] [--project yggterm]
  ygg-ci.py tune --project yggterm --interval 300 --build "cargo build --release" --deploy "scripts/deploy-fleet.sh" --repo ~/gh/yggterm [--host dev]
  ygg-ci.py config [--project yggterm] [--json]
  ygg-ci.py disarm [--hours 4|--forever] [--note why]
  ygg-ci.py arm
  ygg-ci.py hold [--until 2h|--forever|--clear] [--reason why]  # red main / toolchain hold
"""
import argparse
import json
import os
import re
import shlex
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
import ygg_appctl  # noqa: E402

# ─── state ───────────────────────────────────────────────────────────────────
HOME_PLANE_STATE = Path(ygg_appctl.relay_dir())
CI_STATE = HOME_PLANE_STATE / "ci"
SUBS = CI_STATE / "subs"
BUILDS = CI_STATE / "builds"
CONFIG_PATH = CI_STATE / "ci.json"
PIDFILE = CI_STATE / "ci.pid"
HEARTBEAT = CI_STATE / "ci.heartbeat"
LOGPATH = CI_STATE / "ci.log"
DISARMFILE = CI_STATE / "ci.disarmed"
HOLDFILE = CI_STATE / "ci.hold"

CI_HOST = "dev"  # auto host for all ci builds — see yggsteer
DEFAULT_INTERVAL = 300
DISARM_HOURS = 4.0

# in-process heartbeat bookkeeping (watcher only)
_LAST_LOG_WRITE_TS = 0.0
_WATCH_STARTED_TS = 0.0
_STDOUT_IS_LOG = None

def _ci_default_config():
    return {
        "projects": {
            "yggterm": {
                "repo": str(Path.home() / "gh/yggterm"),
                "remote": "origin",
                "main_branch": "main",
                "integration_branch": "integration",
                "build": "cargo build --release --bin yggterm --bin yggterm-headless --bin ynpm",
                "deploy": "scripts/deploy-fleet.sh",
                "host": CI_HOST,
                "interval": DEFAULT_INTERVAL,
            }
        }
    }

def _load_config():
    if CONFIG_PATH.exists():
        try:
            cfg = json.loads(CONFIG_PATH.read_text())
            # ensure shape
            if "projects" not in cfg:
                cfg = _ci_default_config()
            return cfg
        except Exception:
            pass
    cfg = _ci_default_config()
    # also honour repo-local .ygg-ci.json if present and richer
    # (repo file wins for build/deploy fields, not for host/interval fleet tuning)
    try:
        # try yggterm repo's .ygg-ci.json or ci.json
        for p in [Path.home() / "gh/yggterm/.ygg-ci.json", Path.home() / "gh/yggterm/.yggterm-ci.json"]:
            if p.exists():
                d = json.loads(p.read_text())
                if isinstance(d, dict) and "projects" in d:
                    # merge
                    for k, v in d["projects"].items():
                        cfg["projects"][k] = {**cfg["projects"].get(k, {}), **v}
    except Exception:
        pass
    return cfg

def _save_config(cfg):
    CI_STATE.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(cfg, indent=2))

def _project_cfg(project):
    cfg = _load_config()
    pcfg = cfg.get("projects", {}).get(project)
    if not pcfg:
        # lazy create generic entry: repo ~/gh/<project>
        pcfg = {
            "repo": str(Path.home() / f"gh/{project}"),
            "remote": "origin",
            "main_branch": "main",
            "integration_branch": "integration",
            "build": "cargo build --release" if project == "yggterm" else "make -j4 || cargo build --release || npm run build",
            "deploy": "scripts/deploy-fleet.sh" if project == "yggterm" else "",
            "host": CI_HOST,
            "interval": DEFAULT_INTERVAL,
        }
        cfg["projects"][project] = pcfg
        _save_config(cfg)
    # expand ~ in repo
    pcfg["repo"] = str(Path(pcfg["repo"]).expanduser())
    return pcfg

def this_host():
    return os.uname().nodename.split(".")[0]

def _stdout_is_the_log():
    try:
        s = os.fstat(sys.stdout.fileno())
        t = LOGPATH.stat()
        return (s.st_dev, s.st_ino) == (t.st_dev, t.st_ino)
    except Exception:
        return False

def log(m):
    global _LAST_LOG_WRITE_TS, _STDOUT_IS_LOG
    line = f"{time.strftime('%H:%M:%S')} ygg-ci {m}"
    print(line, flush=True)
    if _STDOUT_IS_LOG is None:
        _STDOUT_IS_LOG = _stdout_is_the_log()
    if _STDOUT_IS_LOG:
        _LAST_LOG_WRITE_TS = time.time()
        return
    try:
        CI_STATE.mkdir(parents=True, exist_ok=True)
        with open(LOGPATH, "a") as f:
            f.write(line + "\n")
        _LAST_LOG_WRITE_TS = time.time()
    except Exception:
        pass

def _sanitize_lane(lane):
    # filesystem-safe: lane/foo/bar -> lane--foo--bar
    s = re.sub(r"[^A-Za-z0-9._-]", "--", lane.strip())
    s = re.sub(r"-{2,}", "--", s)
    return s[:120] or "lane"

def _sub_path(project, lane):
    return SUBS / f"{project}--{_sanitize_lane(lane)}.json"

def _run(cmd, cwd=None, timeout=120, shell=False):
    try:
        if shell:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd, shell=True)
        else:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd)
        return r
    except Exception as e:
        return subprocess.CompletedProcess(cmd, 127, "", f"{type(e).__name__}: {e}")

def _repo_for(project, explicit=None):
    cfg = _project_cfg(project)
    p = Path(explicit or cfg["repo"]).expanduser()
    return p, cfg

def load_subs(project=None):
    if not SUBS.exists():
        return []
    out = []
    for p in sorted(SUBS.glob("*.json")):
        try:
            d = json.loads(p.read_text())
            if project and d.get("project") != project:
                continue
            out.append(d)
        except Exception:
            continue
    return out

def own_uuid():
    return (os.environ.get("YGGTERM_SESSION_ID") or "").rstrip("/").split("/")[-1]

# ─── heartbeat / pid / alive ─────────────────────────────────────────────────

def _write_heartbeat():
    CI_STATE.mkdir(parents=True, exist_ok=True)
    rec = {
        "ts": time.time(),
        "pid": os.getpid(),
        "host": this_host(),
        "last_log_write_ts": _LAST_LOG_WRITE_TS,
        "started_ts": _WATCH_STARTED_TS,
    }
    HEARTBEAT.write_text(json.dumps(rec))

def watcher_alive():
    if not PIDFILE.exists():
        return False
    try:
        pid = int(PIDFILE.read_text().strip().split()[0])
        os.kill(pid, 0)
        # heartbeat freshness: if stale > interval*2+60s, consider dead
        if HEARTBEAT.exists():
            hb = json.loads(HEARTBEAT.read_text())
            if time.time() - hb.get("ts", 0) > 700:
                return False
        return True
    except Exception:
        return False

def ensure_watcher(interval=DEFAULT_INTERVAL, project=None):
    if watcher_alive():
        return True
    # spawn detached watcher
    CI_STATE.mkdir(parents=True, exist_ok=True)
    logf = open(LOGPATH, "a")
    cmd = [sys.executable, str(HERE / "ygg-ci.py"), "watch", "--interval", str(interval)]
    if project:
        cmd += ["--project", project]
    # detach: setsid, redirect
    try:
        subprocess.Popen(cmd, stdout=logf, stderr=subprocess.STDOUT, preexec_fn=os.setsid, stdin=subprocess.DEVNULL, cwd=str(HERE))
        # give it a moment to write pidfile
        for _ in range(10):
            time.sleep(0.2)
            if watcher_alive():
                log(f"watcher spawned (interval {interval}s)")
                return True
        log("watcher spawn requested — pidfile not yet visible")
        return False
    except Exception as e:
        log(f"spawn failed: {e}")
        return False

# ─── disarm / hold ───────────────────────────────────────────────────────────

def disarm_state():
    try:
        d = json.loads(DISARMFILE.read_text())
    except Exception:
        return None
    until = d.get("until") or 0
    if until and time.time() >= until:
        DISARMFILE.unlink(missing_ok=True)
        log(f"⭐ disarm EXPIRED ({d.get('note') or 'no reason given'}) — ci is ARMED again")
        return None
    return d

def hold_state():
    try:
        d = json.loads(HOLDFILE.read_text())
    except Exception:
        return None
    if d.get("indefinite"):
        return d
    until = d.get("until") or 0
    if time.time() >= until:
        HOLDFILE.unlink(missing_ok=True)
        return None
    return d

def hold_remaining(h):
    if h.get("indefinite"):
        return "INDEFINITE"
    left = (h.get("until") or 0) - time.time()
    return f"{left/60:.0f}m left" if left < 5400 else f"{left/3600:.1f}h left"

# ─── git helpers ─────────────────────────────────────────────────────────────

def git_fetch(repo_path, remote="origin"):
    return _run(["git", "fetch", "--prune", remote], cwd=str(repo_path), timeout=120)

def git_rev(repo_path, rev):
    r = _run(["git", "rev-parse", rev], cwd=str(repo_path), timeout=30)
    if r.returncode == 0:
        return r.stdout.strip()
    return None

def git_branch_exists(repo_path, remote_branch):
    # remote_branch like origin/lane/foo
    r = _run(["git", "rev-parse", "--verify", remote_branch], cwd=str(repo_path), timeout=30)
    return r.returncode == 0

def git_is_ancestor(repo_path, a, b):
    r = _run(["git", "merge-base", "--is-ancestor", a, b], cwd=str(repo_path), timeout=30)
    return r.returncode == 0

# ─── subscribe / unsubscribe ─────────────────────────────────────────────────

def cmd_subscribe(a):
    project = a.project or "yggterm"
    lane = (a.lane or "").strip()
    if not lane:
        print("subscribe: need --lane lane/<topic>", file=sys.stderr)
        return 2
    repo_path, pcfg = _repo_for(project, a.repo)
    # CI_HOST enforcement: warn if not on dev
    if this_host() != CI_HOST and this_host() != pcfg.get("host", CI_HOST):
        log(f"⚠ subscribe on {this_host()} but ci host is {pcfg.get('host', CI_HOST)} — subscription stored locally, watcher may not see it. Prefer: ssh {pcfg.get('host', CI_HOST)} ygg-ci.py subscribe --lane {lane} --project {project}")
    SUBS.mkdir(parents=True, exist_ok=True)
    # resolve tip if branch exists remotely (best effort)
    tip = None
    if repo_path.exists() and (repo_path / ".git").exists() or (repo_path / ".git").is_file():
        # file .git for worktrees
        git_fetch(repo_path, pcfg.get("remote", "origin"))
        remote_ref = f"{pcfg.get('remote','origin')}/{lane}"
        tip = git_rev(repo_path, remote_ref)
    rec = {
        "lane": lane,
        "project": project,
        "repo": str(repo_path),
        "tip_at_enlist": tip,
        "enlisted_at": int(time.time()),
        "by": own_uuid() or a.by or "shell",
        "want": a.want or "next",
    }
    p = _sub_path(project, lane)
    p.write_text(json.dumps(rec, indent=2))
    log(f"subscribed {project}:{lane} tip={tip[:12] if tip else 'unknown'} want={rec['want']}")
    # arm watcher
    ensure_watcher(interval=int(pcfg.get("interval", DEFAULT_INTERVAL)), project=project if project != "yggterm" else None)
    # readback
    if not p.exists():
        log("⛔ subscribe did not persist")
        return 1
    return 0

def cmd_unsubscribe(a):
    project = a.project or "yggterm"
    lane = (a.lane or "").strip()
    if not lane:
        print("unsubscribe: need --lane", file=sys.stderr)
        return 2
    p = _sub_path(project, lane)
    if not p.exists():
        # try any matching project prefix
        matches = list(SUBS.glob(f"*--{_sanitize_lane(lane)}.json"))
        if len(matches) == 1:
            p = matches[0]
        elif len(matches) > 1:
            print(f"ambiguous lane {lane} across projects: {', '.join(x.name for x in matches)} — pass --project", file=sys.stderr)
            return 2
        else:
            log(f"not subscribed: {project}:{lane}")
            return 0
    p.unlink(missing_ok=True)
    log(f"unsubscribed {project}:{lane}")
    return 0

def cmd_list(a):
    subs = load_subs(project=a.project)
    if a.json:
        print(json.dumps({"subs": subs}, indent=2))
        return 0
    if not subs:
        print("no ci subscriptions" + (f" for {a.project}" if a.project else ""))
        return 0
    for s in subs:
        age_m = (time.time() - s.get("enlisted_at", time.time())) / 60
        print(f"{s.get('project')}:{s.get('lane')}  tip={ (s.get('tip_at_enlist') or '?')[:12]}  by={s.get('by','')[:8]}  {age_m:.0f}m ago  want={s.get('want')}")
    return 0

def cmd_config(a):
    if a.project:
        pcfg = _project_cfg(a.project)
        if a.json:
            print(json.dumps({a.project: pcfg}, indent=2))
        else:
            print(f"project {a.project}:")
            for k, v in pcfg.items():
                print(f"  {k}: {v}")
        return 0
    cfg = _load_config()
    if a.json:
        print(json.dumps(cfg, indent=2))
        return 0
    for proj, pcfg in cfg.get("projects", {}).items():
        print(f"[{proj}] repo={pcfg.get('repo')} host={pcfg.get('host')} interval={pcfg.get('interval')} build={pcfg.get('build')[:60]}")
    return 0

def cmd_tune(a):
    project = a.project or "yggterm"
    cfg = _load_config()
    pcfg = cfg.get("projects", {}).get(project) or _project_cfg(project)
    # ensure in cfg
    cfg.setdefault("projects", {})[project] = pcfg
    changed = []
    if a.repo:
        pcfg["repo"] = str(Path(a.repo).expanduser())
        changed.append(f"repo={pcfg['repo']}")
    if a.build is not None:
        pcfg["build"] = a.build
        changed.append(f"build={a.build[:50]}")
    if a.deploy is not None:
        pcfg["deploy"] = a.deploy
        changed.append(f"deploy={a.deploy[:50]}")
    if a.interval is not None:
        pcfg["interval"] = int(a.interval)
        changed.append(f"interval={pcfg['interval']}")
    if a.host:
        pcfg["host"] = a.host
        changed.append(f"host={a.host}")
    if a.remote:
        pcfg["remote"] = a.remote
        changed.append(f"remote={a.remote}")
    if a.gates is not None:
        pcfg["gates"] = a.gates
        changed.append(f"gates={a.gates}")
    if a.push is not None:
        pcfg["push"] = (a.push == "true")
        changed.append(f"push={pcfg['push']}")
    if a.push_remote:
        pcfg["push_remote"] = a.push_remote
        changed.append(f"push_remote={a.push_remote}")
    _save_config(cfg)
    log(f"tuned {project}: {', '.join(changed) or 'no change'}")
    # verify
    cfg2 = _load_config()
    if cfg2.get("projects", {}).get(project) != pcfg:
        log("⛔ tune did not read back")
        return 1
    return 0

# ─── status ──────────────────────────────────────────────────────────────────

def cmd_status(a):
    subs = load_subs(project=a.project)
    alive = watcher_alive()
    dis = disarm_state()
    hold = hold_state()
    hb = None
    if HEARTBEAT.exists():
        try:
            hb = json.loads(HEARTBEAT.read_text())
        except Exception:
            hb = None
    # last build
    last = None
    if BUILDS.exists():
        candidates = sorted(BUILDS.glob("*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
        for p in candidates[:5]:
            if a.project and a.project not in p.name:
                continue
            try:
                last = json.loads(p.read_text())
                break
            except Exception:
                continue
    info = {
        "host": this_host(),
        "ci_host": CI_HOST,
        "watcher_alive": alive,
        "heartbeat": hb,
        "subscribed": len(subs),
        "disarmed": dis,
        "hold": hold,
        "last_build": last,
        "pidfile": str(PIDFILE) if PIDFILE.exists() else None,
        "log": str(LOGPATH),
    }
    if a.json:
        print(json.dumps(info, indent=2))
        return 0
    print(f"ygg-ci status host={this_host()} ci_host={CI_HOST}")
    print(f"  watcher: {'alive' if alive else 'not running'}" + (f" pid {hb.get('pid')}" if hb else ""))
    if hb:
        age = time.time() - hb.get("ts", 0)
        print(f"  heartbeat: {age:.0f}s ago host={hb.get('host')} last_log={time.time()-hb.get('last_log_write_ts',0):.0f}s ago")
    print(f"  subs: {len(subs)}" + (f" (project {a.project})" if a.project else ""))
    if dis:
        print(f"  ⛔ DISARMED for {hold_remaining(dis)} note={dis.get('note')}")
    if hold:
        print(f"  ⏸ HOLD {hold_remaining(hold)} reason={hold.get('reason') or hold.get('note')}")
    if last:
        print(f"  last build: {last.get('id')} project={last.get('project')} status={last.get('status')} sha={last.get('sha','')[:12]} lanes={len(last.get('lanes',[]))}")
        if last.get("conflicts"):
            print(f"    conflicts: {last['conflicts']}")
    print(f"  log: {LOGPATH}")
    return 0

# ─── disarm / arm / hold ─────────────────────────────────────────────────────

def cmd_disarm(a):
    hours = None if a.forever else float(a.hours or DISARM_HOURS)
    rec = {
        "since": time.time(),
        "until": 0 if hours is None else time.time() + hours * 3600,
        "hours": hours,
        "note": a.note,
        "by": own_uuid() or "shell",
        "host": this_host(),
    }
    DISARMFILE.parent.mkdir(parents=True, exist_ok=True)
    DISARMFILE.write_text(json.dumps(rec, indent=1))
    back = disarm_state()
    span = "until re-armed by hand" if hours is None else f"for {hours:g}h"
    log(f"⛔ ci DISARMED {span} on {this_host()} — nobody will be built. Reason: {a.note or 'none'}")
    return 0 if back else 1

def cmd_arm(a):
    d = disarm_state()
    if not d:
        log(f"ci already armed on {this_host()}")
        return 0
    DISARMFILE.unlink(missing_ok=True)
    log(f"⭐ ci ARMED on {this_host()} — was disarmed ({d.get('note') or 'no reason'})")
    return 0 if not disarm_state() else 1

def _parse_until(spec, now):
    spec = spec.strip()
    m = re.fullmatch(r"(\d+(?:\.\d+)?)([dhm])", spec, re.I)
    if m:
        n, unit = float(m.group(1)), m.group(2).lower()
        return now + n * {"d":86400,"h":3600,"m":60}[unit]
    for fmt in ("%Y-%m-%dT%H:%M","%Y-%m-%d %H:%M","%Y-%m-%dT%H:%M:%S"):
        try:
            return time.mktime(time.strptime(spec, fmt))
        except ValueError:
            pass
    raise ValueError(f"cannot read --until {spec!r}: use 5d / 36h / 90m or 2026-08-19T09:00")

def cmd_hold(a):
    now = time.time()
    cur = hold_state()
    if a.clear:
        if not cur:
            log("no hold in force")
            return 0
        HOLDFILE.unlink(missing_ok=True)
        log("⭐ hold CLEARED — builds resume next tick")
        return 0
    if a.forever:
        rec = {"since": (cur or {}).get("since", now), "until": now+10*365*86400, "indefinite": True, "reason": a.reason or a.note or "hold indefinite", "by": own_uuid() or this_host()}
        HOLDFILE.parent.mkdir(parents=True, exist_ok=True)
        HOLDFILE.write_text(json.dumps(rec, indent=1))
        log(f"⏸ HOLD ARMED — INDEFINITE reason: {rec['reason']}")
        return 0
    if not a.until:
        if not cur:
            log("no hold in force — ci is free to build")
            return 0
        print(f"⏸ hold {hold_remaining(cur)} reason={cur.get('reason')}")
        return 0
    until = _parse_until(a.until, now)
    if until <= now:
        log(f"⛔ refusing: {a.until} is in the past")
        return 2
    rec = {"since": (cur or {}).get("since", now), "until": until, "reason": a.reason or a.note or "hold by hand", "by": own_uuid() or this_host()}
    HOLDFILE.parent.mkdir(parents=True, exist_ok=True)
    HOLDFILE.write_text(json.dumps(rec, indent=1))
    log(f"⏸ HOLD ARMED until {time.strftime('%Y-%m-%d %H:%M', time.localtime(until))} reason: {rec['reason']}")
    return 0

# ─── tick — one integration build per project ────────────────────────────────

def _scratch_dir(project, ts):
    # ⛔ OBSOLETE IN V2: the CI integrates IN the main checkout — no worktrees
    # of any project (owner directive 2026-09-03). Kept only so old build
    # records still resolve; the v2 tick never calls this.
    base = Path.home() / ".yggterm/scratchpad/ci" / project
    base.mkdir(parents=True, exist_ok=True)
    return base / f"integ-{ts}"

# ─── v2: talking events, quarantine, build-in-main ──────────────────────────

EVENTS = CI_STATE / "events.jsonl"
QUARANTINE = CI_STATE / "quarantine.json"
BOARD_THROTTLE = CI_STATE / "board-throttle.json"
BOARD = "infra/ci"

# Event kinds that are always worth a board post (agents mine failures);
# success kinds are throttled to one digest per hour per project.
BOARD_FAILURE_KINDS = {
    "merge_refused", "build_failed", "gate_failed", "lane_quarantined",
    "ci_blocked_dirty_main", "ci_refused_diverged", "push_failed",
    "deploy_failed", "ci_refused_not_main",
}

def _emit_event(project, kind, dry=False, **fields):
    """THE TALKING PLANE: every state transition lands here.

    Appends to events.jsonl (machine plane — read with `ygg-ci events`) and,
    for kinds an agent must notice, posts to the msgGraph board so a row that
    never polls still sees the CI's state (the msgGraph door is always open;
    failures are exactly what another campaign's decisions depend on).
    """
    event_id = "ci-" + uuid.uuid4().hex[:12]
    rec = {
        "event_id": event_id,
        "at": int(time.time()),
        "project": project,
        "kind": kind,
        "host": this_host(),
    }
    rec.update(fields)
    try:
        CI_STATE.mkdir(parents=True, exist_ok=True)
        with EVENTS.open("a") as fh:
            fh.write(json.dumps(rec) + "\n")
    except Exception as e:
        log(f"⚠ event write failed: {e}")
    if dry:
        return event_id
    if kind in BOARD_FAILURE_KINDS:
        _board_post(project, kind, rec)
    return event_id

def _board_post(project, kind, rec):
    try:
        thr = {}
        if BOARD_THROTTLE.exists():
            thr = json.loads(BOARD_THROTTLE.read_text())
        now = time.time()
        # failure dedupe: identical kind+project re-posts at most hourly so a
        # persistently red lane does not flood the board.
        key = f"{project}:{kind}:{rec.get('lane') or rec.get('sha') or ''}"
        if now - thr.get(key, 0) < 3600:
            return
        thr[key] = now
        # hourly digest cap on success kinds (built_and_pushed)
        if kind == "built_and_pushed":
            skey = f"{project}:built_and_pushed"
            if now - thr.get(skey, 0) < 3600:
                return
            thr[skey] = now
        BOARD_THROTTLE.write_text(json.dumps(thr))
        body = json.dumps({k: v for k, v in rec.items() if k not in ("event_id", "at", "host")},
                          default=str)[:900]
        _run(["msgboard", "post", BOARD, "--kind", "note", "--harness", "opencode",
              "--from-row", "ygg-ci", "--ttl-days", "14",
              "--body", f"[{kind}] {body}"], timeout=30)
    except Exception as e:
        log(f"⚠ board post failed: {e}")

def _quarantine_load():
    try:
        return json.loads(QUARANTINE.read_text())
    except Exception:
        return {}

def _quarantine_save(q):
    CI_STATE.mkdir(parents=True, exist_ok=True)
    QUARANTINE.write_text(json.dumps(q))

def _hygiene_gate(project, pcfg, repo, main_branch):
    """THE MAIN-CHECKOUT IS THE BUILD FLOOR — it must be integration-ready.

    Blocks on anything that a merge/build would trip over (staged, modified,
    unmerged, deleted files) and on a checkout that is not the main branch.
    Untracked files (??) are tolerated: scratch and logs live in trees git
    ignores, and an untracked file cannot conflict with a merge.
    """
    st = _run(["git", "status", "--porcelain"], cwd=str(repo), timeout=30)
    if st.returncode != 0:
        _emit_event(project, "ci_blocked_dirty_main", detail=f"git status failed: {st.stderr[:300]}")
        return {"blocked": True, "why": "git status failed"}
    hard = [l for l in st.stdout.splitlines() if l.strip() and not l.startswith("??")]
    if hard:
        _emit_event(project, "ci_blocked_dirty_main", dirty=hard[:12])
        return {"blocked": True,
                "why": f"main checkout has {len(hard)} non-untracked change(s): {hard[0][:80]}"}
    br = _run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=str(repo), timeout=30)
    branch = (br.stdout or "").strip()
    if branch != main_branch:
        _emit_event(project, "ci_refused_not_main", branch=branch)
        return {"blocked": True, "why": f"checked out branch is {branch!r}, not {main_branch!r}"}
    return {"blocked": False}

def _do_tick_project(project, dry=False):
    pcfg = _project_cfg(project)
    repo = Path(pcfg["repo"]).expanduser()
    upstream = pcfg.get("push_remote", pcfg.get("remote", "origin"))
    main_branch = pcfg.get("main_branch", "main")
    build_cmd = (pcfg.get("build", "") or "").strip()
    deploy_cmd = (pcfg.get("deploy", "") or "").strip()
    gates = [g for g in (pcfg.get("gates", "") or "").split() if g]
    do_push = pcfg.get("push", True)

    # host gate (warning only — the fleet may move the ci host deliberately)
    if this_host() != pcfg.get("host", CI_HOST):
        log(f"⚠ tick {project} on {this_host()} but ci host is {pcfg.get('host', CI_HOST)} — building anyway (tune --host)")

    dis = disarm_state()
    if dis:
        log(f"⏸ tick {project}: DISARMED — skipping")
        return {"status": "disarmed"}
    hold = hold_state()
    if hold:
        log(f"⏸ tick {project}: HELD — skipping ({hold.get('reason')})")
        return {"status": "held", "hold": hold}
    subs = load_subs(project=project)
    if not subs:
        log(f"tick {project}: no subscriptions — nothing to do")
        return {"status": "no-subs"}
    if not repo.exists() or not ((repo / ".git").exists() or (repo / ".git").is_file()):
        _emit_event(project, "ci_blocked_dirty_main", dry=dry, detail=f"repo {repo} missing or not git")
        return {"status": "no-repo"}

    # ⛔ THE MAIN-CHECKOUT HYGIENE GATE. The CI integrates IN main — a dirty
    # floor (a crashed merge, staged files, a foreign branch checked out)
    # would corrupt the integration. Loud event; an agent clears it.
    hy = _hygiene_gate(project, pcfg, repo, main_branch)
    if hy.get("blocked"):
        log(f"⛔ tick {project}: BLOCKED — {hy['why']}")
        return {"status": "blocked", "why": hy["why"]}

    git_fetch(repo, upstream)
    upstream_main = git_rev(repo, f"{upstream}/{main_branch}")
    local_main = git_rev(repo, "HEAD")
    if not upstream_main:
        _emit_event(project, "push_failed", dry=dry, detail=f"cannot resolve {upstream}/{main_branch}")
        return {"status": "no-main"}

    # ALIGN: local behind → fast-forward; diverged → refuse (a human call).
    if upstream_main != local_main:
        if git_is_ancestor(repo, local_main, upstream_main):
            r = _run(["git", "merge", "--ff-only", f"{upstream}/{main_branch}"], cwd=str(repo), timeout=120)
            if r.returncode != 0:
                _emit_event(project, "ci_refused_diverged", dry=dry, detail=r.stderr[:400])
                return {"status": "ff-failed", "stderr": r.stderr[:400]}
            local_main = git_rev(repo, "HEAD")
            log(f"  fast-forwarded local {main_branch} to {upstream_main[:12]}")
        elif not git_is_ancestor(repo, upstream_main, local_main):
            _emit_event(project, "ci_refused_diverged", local=local_main, upstream=upstream_main)
            log(f"⛔ tick {project}: local {main_branch} DIVERGED from upstream — refusing (local {local_main[:12]} vs upstream {upstream_main[:12]})")
            return {"status": "refused-diverged"}

    dirty, last = _dirty_subs(project, pcfg, subs)
    if not dirty and last and upstream_main == local_main:
        log(f"tick {project}: {len(subs)} subs but none dirty since {last.get('id')} — skipping")
        return {"status": "clean", "last": last.get("id")}
    if dry:
        for s in subs:
            mark = "dirty" if s in dirty else "clean"
            tip = git_rev(repo, f"{upstream}/{s['lane']}") or s.get("tip_at_enlist") or "?"
            log(f"  DRY {s['lane']} [{mark}] tip={tip[:12]}")
        return {"status": "dry", "subs": len(subs), "dirty": len(dirty)}

    pre_tick = local_main
    merged, conflicts = [], []
    quar = _quarantine_load().get(project, {})
    quar_changed = False
    for s in subs:
        lane = s["lane"]
        remote_ref = f"{pcfg.get('remote','origin')}/{lane}"
        tip = git_rev(repo, remote_ref)
        if not tip:
            conflicts.append({"lane": lane, "reason": "no-remote-branch"})
            continue
        if git_is_ancestor(repo, tip, local_main):
            log(f"  skip {lane}: already in main ({tip[:12]})")
            merged.append({"lane": lane, "tip": tip, "already_in_main": True})
            continue
        if quar.get(lane) == tip:
            log(f"  ⏳ skip {lane}: quarantined (this tip already failed a build) — new tip re-arms it")
            conflicts.append({"lane": lane, "tip": tip, "reason": "quarantined"})
            continue
        r = _run(["git", "merge", "--no-ff", "--no-edit", remote_ref], cwd=str(repo), timeout=120)
        if r.returncode == 0:
            log(f"  merged {lane} {tip[:12]}")
            merged.append({"lane": lane, "tip": tip})
            if lane in quar:
                quar.pop(lane, None); quar_changed = True
        else:
            _run(["git", "merge", "--abort"], cwd=str(repo), timeout=30)
            log(f"  ⛔ conflict merging {lane} — excluded: {r.stderr[:400]}")
            conflicts.append({"lane": lane, "tip": tip, "reason": "conflict", "stderr": r.stderr[:600]})
            _emit_event(project, "merge_refused", lane=lane, tip=tip, stderr=r.stderr[:500])
    if quar_changed:
        q = _quarantine_load(); q[project] = quar; _quarantine_save(q)

    viable = [m for m in merged if not m.get("already_in_main")]
    integrated_local = local_main != pre_tick  # local was ahead: push pending
    if not viable and not integrated_local and not (conflicts and last is None):
        log(f"tick {project}: nothing new to integrate (merged={len(merged)} conflicts={len(conflicts)})")
        return {"status": "clean", "merged": len(merged), "conflicts": len(conflicts)}

    # ── BUILD, IN MAIN. This is the owner's ordering: the build gates the push. ──
    integ_sha = git_rev(repo, "HEAD")
    build_ok = True
    build_log = ""
    if build_cmd:
        log(f"  building {project} IN {repo} ({main_branch}@{integ_sha[:12]}): {build_cmd}")
        r = _run(build_cmd, cwd=str(repo), timeout=3600, shell=True)
        build_log = (r.stdout or "")[-4000:] + (r.stderr or "")[-4000:]
        build_ok = (r.returncode == 0)
        log("  build ok" if build_ok else f"  ⛔ build FAILED: {build_log[-1200:]}")
    else:
        log("  no build command — treating as ok")

    gate_ok = build_ok
    for g in gates:
        if not build_ok:
            break
        log(f"  gate: {g}")
        r = _run(g, cwd=str(repo), timeout=900, shell=True)
        if r.returncode != 0:
            gate_ok = False
            build_log = (r.stdout or "")[-2000:] + (r.stderr or "")[-2000:]
            log(f"  ⛔ gate FAILED: {g}: {build_log[-800:]}")

    pushed = False
    push_err = ""
    if build_ok and gate_ok:
        if do_push:
            log(f"  pushing {main_branch} → {upstream} ({integ_sha[:12]})")
            r = _run(["git", "push", upstream, main_branch], cwd=str(repo), timeout=600)
            pushed = (r.returncode == 0)
            push_err = (r.stderr or "")[-1200:]
            if not pushed:
                log(f"  ⛔ push FAILED: {push_err}")
        else:
            pushed = None  # push disabled for this repo
    else:
        # ⛔ BUILD/GATE FAILED: main goes back to exactly what upstream (and the
        # fleet) had. The failing lanes are quarantined at this tip so the next
        # tick builds the REST instead of looping red; a new tip re-arms them.
        r = _run(["git", "reset", "--hard", pre_tick], cwd=str(repo), timeout=120)
        log(f"  ⛔ integration failed — main reset to {pre_tick[:12]} ({r.returncode})")
        q = _quarantine_load(); qp = q.setdefault(project, {})
        failed_lanes = [m["lane"] for m in viable]
        for lane in failed_lanes:
            tip = next((m["tip"] for m in viable if m["lane"] == lane), None)
            if tip: qp[lane] = tip
        q[project] = qp; _quarantine_save(q)
        kind = "build_failed" if not build_ok else "gate_failed"
        _emit_event(project, kind, lanes=failed_lanes, reset_to=pre_tick,
                    log_tail=build_log[-900:])
        for m in merged:
            if m.get("already_in_main"):
                p = _sub_path(project, m["lane"])
                if p.exists(): p.unlink(missing_ok=True)
        rec = {
            "id": f"{project}--{ts_stamp()}--{integ_sha[:12] if integ_sha else 'no-sha'}--failed",
            "project": project, "at": int(time.time()), "host": this_host(),
            "main": pre_tick, "sha": integ_sha, "lanes": merged, "conflicts": conflicts,
            "build": build_cmd, "build_ok": build_ok, "status": "build-failed",
            "reset_to": pre_tick, "quarantined": failed_lanes,
        }
        BUILDS.mkdir(parents=True, exist_ok=True)
        (BUILDS / f"{rec['id']}.json").write_text(json.dumps(rec, indent=2))
        return rec

    # ── DEPLOY — only after the push landed upstream (hosts must never run
    # commits that upstream does not have; that split is what the downgrade
    # guard spent 3.2.4x fighting). ──
    deploy_ok = None
    if pushed is not False:
        if deploy_cmd:
            log(f"  deploying {project}: {deploy_cmd}")
            r = _run(deploy_cmd, cwd=str(repo), timeout=3600, shell=True)
            deploy_ok = (r.returncode == 0)
            if not deploy_ok:
                log(f"  ⛔ deploy FAILED: {(r.stderr or r.stdout or '')[-1200:]}")
                _emit_event(project, "deploy_failed", sha=integ_sha,
                            log_tail=((r.stderr or "") + (r.stdout or ""))[-600:])
        if pushed:
            _emit_event(project, "built_and_pushed", sha=integ_sha, upstream=upstream,
                        lanes=[m["lane"] for m in viable], deploy_ok=deploy_ok)
    status = "built" if build_ok else "build-failed"
    if pushed is True: status = "pushed"
    elif pushed is False: status = "push-failed"
    if deploy_ok is True: status = "deployed"
    elif deploy_ok is False: status = "deploy-failed"
    for m in merged:
        if m.get("already_in_main"):
            p = _sub_path(project, m["lane"])
            if p.exists(): p.unlink(missing_ok=True)
            log(f"  auto-unsubscribed {m['lane']} (already in main)")
    rec = {
        "id": f"{project}--{ts_stamp()}--{integ_sha[:12] if integ_sha else 'no-sha'}",
        "project": project, "at": int(time.time()), "host": this_host(),
        "main": pre_tick, "sha": integ_sha, "lanes": merged, "conflicts": conflicts,
        "subs": len(subs), "build": build_cmd, "build_ok": build_ok,
        "gates": gates, "deploy": deploy_cmd, "deploy_ok": deploy_ok,
        "pushed": pushed, "upstream": upstream, "status": status,
    }
    BUILDS.mkdir(parents=True, exist_ok=True)
    (BUILDS / f"{rec['id']}.json").write_text(json.dumps(rec, indent=2))
    log(f"tick {project} -> {status} sha={integ_sha[:12]} merged={len(merged)} conflicts={len(conflicts)} pushed={pushed}")
    return rec

def ts_stamp():
    return time.strftime("%Y%m%d-%H%M%S")

def _last_build(project):
    if not BUILDS.exists():
        return None
    cands = sorted(BUILDS.glob(f"{project}--*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    for p in cands[:1]:
        try:
            return json.loads(p.read_text())
        except Exception:
            continue
    return None

def _dirty_subs(project, pcfg, subs):
    """which subs have moved since last build, or are new. Returns dirty list."""
    repo = Path(pcfg["repo"]).expanduser()
    last = _last_build(project)
    last_map = {}
    if last and last.get("lanes"):
        for l in last["lanes"]:
            last_map[l["lane"]] = l.get("tip")
    dirty = []
    if repo.exists():
        git_fetch(repo, pcfg.get("remote","origin"))
    for s in subs:
        lane = s["lane"]
        remote_ref = f"{pcfg.get('remote','origin')}/{lane}"
        cur_tip = git_rev(repo, remote_ref) if repo.exists() else s.get("tip_at_enlist")
        if cur_tip is None and s.get("tip_at_enlist") is None:
            dirty.append(s)
            continue
        if lane not in last_map:
            dirty.append(s)
        elif cur_tip and last_map[lane] != cur_tip:
            dirty.append(s)
    return dirty, last

def cmd_tick(a):
    # optionally host gate
    if not a.project:
        # tick all projects that have subs
        projects = set(s["project"] for s in load_subs())
        if not projects:
            projects = {"yggterm"}
        results = {}
        for proj in sorted(projects):
            r = _do_tick_project(proj, dry=bool(a.dry_run))
            results[proj] = r
            # after each project, heartbeat
            try:
                _write_heartbeat()
            except Exception:
                pass
        if a.json:
            print(json.dumps(results, indent=2))
        bad = {"build-failed","push-failed","deploy-failed","no-main","no-repo","blocked","refused-diverged"}
        return 0 if all(v.get("status") not in bad for v in results.values()) else 1
    else:
        r = _do_tick_project(a.project, dry=bool(a.dry_run))
        if a.json:
            print(json.dumps(r, indent=2))
        bad = {"build-failed","push-failed","deploy-failed","no-main","no-repo","blocked","refused-diverged"}
        return 0 if r.get("status") not in bad else 1

def cmd_events(a):
    """Read the CI's talking plane: every state transition, newest last."""
    if not EVENTS.exists():
        print("no events yet — the watcher emits on its first transition")
        return 0
    since = None
    if a.since:
        import re as _re
        m = _re.match(r"^(\d+)([smh])$", a.since.strip())
        secs = {"s": 1, "m": 60, "h": 3600}[m.group(2)] * int(m.group(1)) if m else 1800
        since = time.time() - secs
    out = []
    for line in EVENTS.read_text().splitlines():
        try:
            e = json.loads(line)
        except Exception:
            continue
        if a.project and e.get("project") != a.project:
            continue
        if a.kind and e.get("kind") != a.kind:
            continue
        if since and e.get("at", 0) < since:
            continue
        out.append(e)
    if a.json:
        print(json.dumps(out[-a.tail:], indent=2))
        return 0
    for e in out[-a.tail:]:
        t = time.strftime("%H:%M:%S", time.localtime(e.get("at", 0)))
        body = {k: v for k, v in e.items() if k not in ("event_id", "at", "project", "host")}
        print(f"{t} [{e.get('project')}] {e.get('kind')}: {json.dumps(body, default=str)[:220]}")
    print(f"— {len(out)} event(s) — ids in events.jsonl; failures also posted to msgboard {BOARD}")
    return 0

def cmd_why(a):
    """Plain-language state + the next action. The LLM-facing status."""
    project = a.project or "yggterm"
    pcfg = _project_cfg(project)
    repo = Path(pcfg["repo"]).expanduser()
    alive = watcher_alive()
    print(f"project {project}: repo={repo} ci_host={pcfg.get('host')} watcher={'ALIVE' if alive else 'DOWN (ensure_watcher spawns on next subscribe/tick)'}")
    dis = disarm_state(); hold = hold_state()
    if dis: print(f"  DISARMED: {dis}")
    if hold: print(f"  HELD: {hold}")
    if not repo.exists():
        print(f"  next: fix the repo path (tune --repo)")
        return 2
    st = _run(["git", "status", "--porcelain"], cwd=str(repo), timeout=30)
    hard = [l for l in (st.stdout or "").splitlines() if l.strip() and not l.startswith("??")]
    if hard:
        print(f"  ⛔ main checkout DIRTY ({len(hard)}): {hard[0][:100]}")
        print("     next: resolve or stash in the main checkout — the CI refuses to integrate on a dirty floor")
        return 2
    br = (_run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=str(repo), timeout=30).stdout or "").strip()
    print(f"  main checkout: branch={br} clean={not hard}")
    quar = _quarantine_load().get(project, {})
    if quar:
        print(f"  quarantined lanes (tip failed a build; new tip re-arms): {quar}")
    last = _last_build(project)
    if last:
        print(f"  last build: {last.get('id')} status={last.get('status')} pushed={last.get('pushed')}")
        if last.get("status") in ("build-failed",):
            print("     next: the failing lanes are quarantined; fix their build or land a corrected tip")
    if EVENTS.exists():
        recent = [l for l in EVENTS.read_text().splitlines()[-40:]]
        fails = [l for l in recent if '"kind":' in l and any(k in l for k in BOARD_FAILURE_KINDS)]
        if fails:
            lastf = json.loads(fails[-1])
            print(f"  last failure: {time.strftime('%H:%M', time.localtime(lastf.get('at',0)))} {lastf.get('kind')} {json.dumps({k:v for k,v in lastf.items() if k in ('lane','sha','detail')}, default=str)[:160]}")
    print("  verbs: events (history) · tick --dry-run (rehearse) · tune --gates/--push (config)")
    return 0

def cmd_watch(a):
    project = a.project  # None = all
    interval = int(a.interval or DEFAULT_INTERVAL)
    # respect project tune interval if single project
    if project:
        pcfg = _project_cfg(project)
        interval = int(a.interval or pcfg.get("interval", DEFAULT_INTERVAL))
    global _WATCH_STARTED_TS
    _WATCH_STARTED_TS = time.time()
    CI_STATE.mkdir(parents=True, exist_ok=True)
    # pidfile
    PIDFILE.write_text(f"{os.getpid()} {time.time()}\n")
    # handle TERM
    def _term(signum, frame):
        log(f"watch received {signum} — exiting, removing pidfile")
        try:
            PIDFILE.unlink(missing_ok=True)
        except Exception:
            pass
        sys.exit(0)
    signal.signal(signal.SIGTERM, _term)
    signal.signal(signal.SIGINT, _term)
    log(f"⭐ ci watcher started on {this_host()} interval={interval}s project={project or 'all'} pid={os.getpid()}")
    try:
        while True:
            _write_heartbeat()
            try:
                cmd_tick(argparse.Namespace(project=project, dry_run=False, json=False))
            except Exception as e:
                log(f"tick error: {e}")
            _write_heartbeat()
            # sleep interval (not burning)
            for _ in range(interval):
                time.sleep(1)
                # allow interval retune without restart: re-read config
                if project:
                    try:
                        pcfg = _project_cfg(project)
                        new_interval = int(pcfg.get("interval", interval))
                        if new_interval != interval:
                            log(f"interval retuned {interval}s -> {new_interval}s (project {project})")
                            interval = new_interval
                            break
                    except Exception:
                        pass
    finally:
        try:
            PIDFILE.unlink(missing_ok=True)
        except Exception:
            pass

# ─── main ────────────────────────────────────────────────────────────────────

def main():
    p = argparse.ArgumentParser(prog="ygg-ci.py", description="fleet single-build plane — subscribe lanes, one build, one deploy")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("subscribe", help="enroll a lane branch in the next build")
    s.add_argument("--lane", required=True, help="branch name, e.g. lane/foo/bar")
    s.add_argument("--project", default="yggterm", help="project name (default yggterm)")
    s.add_argument("--repo", help="repo path override")
    s.add_argument("--want", default="next", choices=["next","always"], help="want next build or always")
    s.add_argument("--by", help="who (uuid)")

    s = sub.add_parser("unsubscribe", help="remove a lane from the build")
    s.add_argument("--lane", required=True)
    s.add_argument("--project", default="yggterm")

    s = sub.add_parser("list", help="list subscriptions")
    s.add_argument("--project", help="filter project")
    s.add_argument("--json", action="store_true")

    s = sub.add_parser("config", help="show project config")
    s.add_argument("--project", help="project")
    s.add_argument("--json", action="store_true")

    s = sub.add_parser("tune", help="tune how a project builds")
    s.add_argument("--project", default="yggterm")
    s.add_argument("--repo", help="repo path")
    s.add_argument("--build", help="build command (shell)")
    s.add_argument("--deploy", help="deploy command (shell, empty to disable)")
    s.add_argument("--interval", type=int, help="watch interval seconds")
    s.add_argument("--host", help="ci host (default dev)")
    s.add_argument("--remote", help="git remote name")
    s.add_argument("--gates", help="space-separated gate scripts run after the build, before the push (e.g. 'scripts/check-privacy.sh')")
    s.add_argument("--push", choices=["true", "false"], help="push main to the upstream after a green build (default true)")
    s.add_argument("--push-remote", help="remote to push the integration to (default: the fetch remote)")

    s = sub.add_parser("status", help="is watcher alive, subs, last build")
    s.add_argument("--json", action="store_true")
    s.add_argument("--project", help="filter project for subs/last build")

    s = sub.add_parser("events", help="the CI's talking plane — every state transition (merge refused, build done, pushed…)")
    s.add_argument("--project", help="filter project")
    s.add_argument("--kind", help="filter event kind")
    s.add_argument("--since", help="30m / 2h / 1d window (default 30m)")
    s.add_argument("--tail", type=int, default=40, help="last N events to print (default 40)")
    s.add_argument("--json", action="store_true")

    s = sub.add_parser("why", help="plain-language current state + the next action (agent-facing)")
    s.add_argument("--project", help="project")

    s = sub.add_parser("tick", help="one integration pass over all subs")
    s.add_argument("--project", help="single project or all if omitted")
    s.add_argument("--dry-run", action="store_true")
    s.add_argument("--json", action="store_true")

    s = sub.add_parser("watch", help="the loop — let subscribe spawn it")
    s.add_argument("--interval", type=int, default=None)
    s.add_argument("--project", help="watch single project")

    s = sub.add_parser("disarm", help="stand ci down without dismantling it")
    s.add_argument("--hours", type=float, help="hours to stay down (default 4)")
    s.add_argument("--forever", action="store_true")
    s.add_argument("--note", help="why")

    s = sub.add_parser("arm", help="re-arm after disarm")

    s = sub.add_parser("hold", help="fleet-wide build hold (red main)")
    s.add_argument("--until", help="2h / 30m / 2026-08-30T10:00 or --forever")
    s.add_argument("--forever", action="store_true")
    s.add_argument("--clear", action="store_true")
    s.add_argument("--reason", help="why")
    s.add_argument("--note", help="alias for --reason")

    a = p.parse_args()
    # route
    if a.cmd == "subscribe":
        sys.exit(cmd_subscribe(a))
    elif a.cmd == "unsubscribe":
        sys.exit(cmd_unsubscribe(a))
    elif a.cmd == "list":
        sys.exit(cmd_list(a))
    elif a.cmd == "config":
        sys.exit(cmd_config(a))
    elif a.cmd == "tune":
        sys.exit(cmd_tune(a))
    elif a.cmd == "status":
        sys.exit(cmd_status(a))
    elif a.cmd == "tick":
        sys.exit(cmd_tick(a))
    elif a.cmd == "events":
        sys.exit(cmd_events(a))
    elif a.cmd == "why":
        sys.exit(cmd_why(a))
    elif a.cmd == "watch":
        sys.exit(cmd_watch(a))
    elif a.cmd == "disarm":
        sys.exit(cmd_disarm(a))
    elif a.cmd == "arm":
        sys.exit(cmd_arm(a))
    elif a.cmd == "hold":
        sys.exit(cmd_hold(a))
    else:
        p.print_help()
        sys.exit(2)

if __name__ == "__main__":
    main()
