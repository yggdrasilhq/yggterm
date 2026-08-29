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
    # disk-backed per AGENTS scratch law: ~/.yggterm/scratchpad
    base = Path.home() / ".yggterm/scratchpad/ci" / project
    base.mkdir(parents=True, exist_ok=True)
    return base / f"integ-{ts}"

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
    # refresh remote tips
    if repo.exists():
        git_fetch(repo, pcfg.get("remote","origin"))
    for s in subs:
        lane = s["lane"]
        remote_ref = f"{pcfg.get('remote','origin')}/{lane}"
        cur_tip = git_rev(repo, remote_ref) if repo.exists() else s.get("tip_at_enlist")
        # if branch not yet pushed, treat subscribing itself as dirty once
        if cur_tip is None and s.get("tip_at_enlist") is None:
            dirty.append(s)
            continue
        if lane not in last_map:
            dirty.append(s)
        elif cur_tip and last_map[lane] != cur_tip:
            dirty.append(s)
    # also dirty if new subs exist that were not in last build even if tip same
    return dirty, last

def _do_tick_project(project, dry=False):
    pcfg = _project_cfg(project)
    repo = Path(pcfg["repo"]).expanduser()
    # host gate
    if this_host() != pcfg.get("host", CI_HOST):
        # still allow tick but log — fleet may have moved
        log(f"⚠ tick for {project} on {this_host()} but ci host is {pcfg.get('host', CI_HOST)} — building anyway (override via tune --host)")
    # guards
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
    if not repo.exists():
        log(f"⛔ tick {project}: repo {repo} missing — cannot build")
        return {"status": "no-repo"}
    # worktree check
    if not (repo / ".git").exists() and not (repo / ".git").is_file():
        log(f"⛔ tick {project}: {repo} is not a git repo")
        return {"status": "no-git"}
    # dirty check
    dirty, last = _dirty_subs(project, pcfg, subs)
    if not dirty and last:
        # also check if last build had conflicts that now cleared? treat as not dirty then
        log(f"tick {project}: {len(subs)} subs but none dirty since last build {last.get('id')} — skipping")
        return {"status": "clean", "last": last.get("id")}
    log(f"tick {project}: {len(subs)} subs, {len(dirty)} dirty — aggregating")
    if dry:
        for s in subs:
            mark = "dirty" if s in dirty else "clean"
            remote_ref = f"{pcfg.get('remote','origin')}/{s['lane']}"
            tip = git_rev(repo, remote_ref) or s.get("tip_at_enlist") or "?"
            log(f"  DRY {s['lane']} [{mark}] tip={tip[:12]}")
        return {"status": "dry", "subs": len(subs), "dirty": len(dirty)}
    # ensure main fetch
    fr = git_fetch(repo, pcfg.get("remote","origin"))
    # resolve main sha
    remote_main = f"{pcfg.get('remote','origin')}/{pcfg.get('main_branch','main')}"
    main_sha = git_rev(repo, remote_main)
    if not main_sha:
        log(f"⛔ tick {project}: cannot resolve {remote_main}")
        return {"status": "no-main"}
    ts = time.strftime("%Y%m%d-%H%M%S")
    work = _scratch_dir(project, ts)
    # worktree add --detach <work> <main_sha>
    r = _run(["git", "worktree", "add", "--detach", str(work), main_sha], cwd=str(repo), timeout=60)
    if r.returncode != 0:
        # fallback: plain checkout to scratch clone? try git worktree add without detach
        log(f"⛔ worktree add failed: {r.stderr[:600]}")
        # try shallow clone approach: git clone --no-checkout replica?
        return {"status": "worktree-fail", "stderr": r.stderr[:600]}
    try:
        # merge each lane
        merged = []
        conflicts = []
        for s in subs:
            lane = s["lane"]
            remote_ref = f"{pcfg.get('remote','origin')}/{lane}"
            tip = git_rev(repo, remote_ref)
            # also show work repo's view — fetch into work?
            # work is separate worktree sharing same object store, so remote refs already visible
            if not tip:
                log(f"  ⚠ skip {lane}: no remote tip ({remote_ref} not found — not yet pushed?)")
                conflicts.append({"lane": lane, "reason": "no-remote-branch"})
                continue
            # check if already in main (already landed)
            if git_is_ancestor(repo, tip, main_sha):
                log(f"  skip {lane}: already in main ({tip[:12]}) — will auto-unsubscribe on success")
                # keep merged? no, already in main so nothing to merge
                merged.append({"lane": lane, "tip": tip, "already_in_main": True})
                continue
            # attempt merge
            r = _run(["git", "merge", "--no-ff", "--no-edit", remote_ref], cwd=str(work), timeout=120)
            if r.returncode == 0:
                log(f"  merged {lane} {tip[:12]}")
                merged.append({"lane": lane, "tip": tip})
            else:
                # conflict
                _run(["git", "merge", "--abort"], cwd=str(work), timeout=30)
                log(f"  ⛔ conflict merging {lane} — excluded: {r.stderr[:500]}")
                conflicts.append({"lane": lane, "tip": tip, "reason": "conflict", "stderr": r.stderr[:800]})
        # nothing to merge and all subs are missing remote — skip build, keep worktree clean
        viable = [m for m in merged if not m.get("already_in_main")]
        if not viable and conflicts:
            # all subs missing or conflicted and none merged — no build
            integ_sha = git_rev(work, "HEAD")
            log(f"  no viable lanes to build (all missing/empty) — skipping build/deploy")
            rec = {
                "id": f"{project}--{ts}--{integ_sha[:12] if integ_sha else 'no-sha'}--skipped",
                "project": project,
                "at": int(time.time()),
                "host": this_host(),
                "main": main_sha,
                "sha": integ_sha,
                "lanes": merged,
                "conflicts": conflicts,
                "subs": len(subs),
                "build": pcfg.get("build",""),
                "build_ok": None,
                "deploy": pcfg.get("deploy",""),
                "deploy_ok": None,
                "status": "skipped-no-viable-lanes",
            }
            BUILDS.mkdir(parents=True, exist_ok=True)
            (BUILDS / f"{rec['id']}.json").write_text(json.dumps(rec, indent=2))
            log(f"tick {project} -> skipped sha={integ_sha[:12] if integ_sha else '?'} work={work}")
            _run(["git", "worktree", "remove", "--force", str(work)], cwd=str(repo), timeout=30)
            try:
                import shutil
                if work.exists():
                    shutil.rmtree(work, ignore_errors=True)
            except Exception:
                pass
            return rec
        # built sha
        integ_sha = git_rev(work, "HEAD")
        # build step
        build_cmd = pcfg.get("build", "")
        build_ok = True
        build_log = ""
        if build_cmd:
            log(f"  building {project} with: {build_cmd}")
            r = _run(build_cmd, cwd=str(work), timeout=1800, shell=True)
            build_log = (r.stdout or "")[-4000:] + (r.stderr or "")[-4000:]
            if r.returncode != 0:
                log(f"  ⛔ build FAILED ({r.returncode}): {build_log[-1500:]}")
                build_ok = False
            else:
                log(f"  build ok")
        else:
            log("  no build command — skipping build")
        # deploy step only if build ok and no fatal conflicts? we deploy even with partial merge, but not on build fail
        deploy_ok = None
        deploy_log = ""
        if build_ok:
            deploy_cmd = pcfg.get("deploy", "")
            if deploy_cmd and deploy_cmd.strip():
                # resolve deploy invocation: if relative, run from work
                # yggterm's deploy expects FROM to point at artefacts
                # default is scripts/deploy-fleet.sh which will build's FROM default target/release
                # Ensure cargo artefacts are at work/target/release
                # For worktree build, cargo target is at work/target
                deploy_cwd = str(work)
                # allow deploy to be bare script name: run via bash
                full = deploy_cmd
                # yggterm special: deploy-fleet expects --from <dir> else target/release
                # Pass explicit --from if not present
                if "deploy-fleet" in deploy_cmd and "--from" not in deploy_cmd:
                    full = f"{deploy_cmd} --from {work}/target/release"
                elif "deploy-dev" in deploy_cmd and "--from" not in deploy_cmd:
                    full = f"{deploy_cmd} --from {work}/target/release"
                log(f"  deploying {project} with: {full}")
                r = _run(full, cwd=deploy_cwd, timeout=1800, shell=True)
                deploy_log = (r.stdout or "")[-4000:] + (r.stderr or "")[-4000:]
                deploy_ok = (r.returncode == 0)
                if deploy_ok:
                    log(f"  deploy ok")
                else:
                    log(f"  ⛔ deploy FAILED ({r.returncode}): {deploy_log[-1500:]}")
            else:
                log("  no deploy command — stopping after build")
                deploy_ok = None
        status = "built" if build_ok else "build-failed"
        if deploy_ok is True:
            status = "deployed"
        elif deploy_ok is False:
            status = "deploy-failed"
        rec = {
            "id": f"{project}--{ts}--{integ_sha[:12] if integ_sha else 'no-sha'}",
            "project": project,
            "at": int(time.time()),
            "host": this_host(),
            "main": main_sha,
            "sha": integ_sha,
            "lanes": merged,
            "conflicts": conflicts,
            "subs": len(subs),
            "build": build_cmd,
            "build_ok": build_ok,
            "deploy": pcfg.get("deploy",""),
            "deploy_ok": deploy_ok,
            "status": status,
        }
        BUILDS.mkdir(parents=True, exist_ok=True)
        (BUILDS / f"{rec['id']}.json").write_text(json.dumps(rec, indent=2))
        # scratched worktree is kept for a bit for diagnosis, but worktree remove after?
        # Keep for 1h, then prune via scratch reaper; for now just log location
        log(f"tick {project} -> {status} sha={integ_sha[:12] if integ_sha else '?'} merged={len(merged)} conflicts={len(conflicts)} work={work}")
        # auto-unsubscribe lanes that are already in main and were merged successfully?
        # Only unsubscribe those flagged already_in_main, to keep integration clean
        for m in merged:
            if m.get("already_in_main"):
                p = _sub_path(project, m["lane"])
                if p.exists():
                    p.unlink(missing_ok=True)
                    log(f"  auto-unsubscribed {m['lane']} (already in main)")
        # also reap scratch worktree after success? keep work dir for now, but remove git worktree link
        _run(["git", "worktree", "remove", "--force", str(work)], cwd=str(repo), timeout=60)
        # if work dir still exists (worktree remove leaves it if untracked files), keep target for cache? remove to save disk
        try:
            import shutil
            if work.exists():
                # keep log but drop target to save space? keep for now and let scratch reaper handle
                pass
        except Exception:
            pass
        return rec
    finally:
        # ensure worktree removed on failure path too
        try:
            _run(["git", "worktree", "remove", "--force", str(work)], cwd=str(repo), timeout=30)
        except Exception:
            pass

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
        return 0 if all(v.get("status") not in ("build-failed","deploy-failed","worktree-fail","no-main","no-repo") for v in results.values()) else 1
    else:
        r = _do_tick_project(a.project, dry=bool(a.dry_run))
        if a.json:
            print(json.dumps(r, indent=2))
        return 0 if r.get("status") not in ("build-failed","deploy-failed","worktree-fail","no-main","no-repo") else 1

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

    s = sub.add_parser("status", help="is watcher alive, subs, last build")
    s.add_argument("--json", action="store_true")
    s.add_argument("--project", help="filter project for subs/last build")

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
