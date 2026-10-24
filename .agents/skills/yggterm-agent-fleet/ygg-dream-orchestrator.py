#!/usr/bin/env python3
"""ygg-dream-orchestrator — the yggterm dream system's demand side.

One script, three passes (all bounded, all single-flight):

  status      human/json view: today counts, backlog, plane gates
  topup       supply->demand: stage briefs (ygg-memory dream --prepare) and
              enqueue them on the executor queue, bounded per pass
  supervise   fleet-level care for dreams that died at API drops / quota walls:
              classify queue failures against the quota calendars, reroute to
              planes with quota, hold until a reset window, revoke
              false-completions, write the wake ledger

Config (generated on first run, hand-moldable; bootstrap never overwrites):
  ~/.yggterm/config/dream/floors.json   namespaces, floors, routes, targets,
                                        peak windows, quota calendars
  ~/.yggterm/dream/state.json           per-day counts (the spawner's ledger)
  ~/.yggterm/dream/wake-ledger.jsonl    one line per supervision pass

Arming law (onboarding): autonomous dispatch runs on EXACTLY ONE host —
~/.yggterm/config/dream/enabled must exist; without it every mutating pass
exits 0 silently (units install fleet-wide, arming happens once).

Owner law baked in: banked resets are NEVER consumed by any automated path;
quota exhaustion means hold/reroute, never a reset.
"""
import fcntl, json, random, shlex, subprocess, sys, time, uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

HOME = Path.home()
CFG_DIR = HOME / ".yggterm/config/dream"
CFG = CFG_DIR / "floors.json"
ENABLED = CFG_DIR / "enabled"
STATE_DIR = HOME / ".yggterm/dream"
STATE = STATE_DIR / "state.json"
LEDGER = STATE_DIR / "wake-ledger.jsonl"
LOCK = Path("/tmp/ygg-dream-orchestrator.lock")
IST = timezone(timedelta(hours=5, minutes=30))

DEFAULT_CONFIG = {
    "_doc": "dream orchestrator config — mold freely; a tuned file is never overwritten",
    "queue": {"host": "dev", "repo": "~/gh/zcode-tui", "cli": "~/.bun/bin/bun run src/bin/queue.ts"},
    "supply": {"prepare_timeout_s": 240, "pass_max_enqueues": 2, "pass_wall_budget_s": 420},
    "targets": {"day": 60, "moderate": 20, "backlog_cap": 14},
    "routes": ["zai-coding-plan", "autoclaw", "codex", "opencode-free"],
    "quota_calendars": {
        "_doc": "per-plane gates; the orchestrator holds or reroutes, never resets accounts",
        "zai-coding-plan": {"peak_ist": "Mon-Fri 11:30-15:30"},
        "codex": {"_doc": "hourly rolling caps: the queue's cooldown ladder rides the refusal; supervision reroutes"},
        "opencode-free": {"_doc": "free tier — queue owns cooldowns; supervision only audits"},
        "autoclaw": {}
    },
    "namespaces": [
        {"ns": "-home-pi-data-medgraph", "per_day": 8, "moderate_share": 0.5, "budget": 90000},
        {"ns": "-home-pi-gh-yggterm", "per_day": 8, "moderate_share": 0.5, "budget": 90000},
        {"ns": "_global", "per_day": 6, "moderate_share": 0.5, "budget": 90000}
    ],
    "kinds": ["synthesis", "recon", "verify", "sweep"],
    "false_completion": {"audit_per_pass": 3}
}

CONTRACT = """# DREAM ({ns}, {kind}) — consolidate what the journals learned
You are the dreamer for `{ns}`. Read the staged brief and its manifest, compose
RESPONSE + CRITIC in the pending dir per the dream contract, apply what is
safe, publish what must survive. UNATTENDED: no questions, verify before done.
Brief: {brief}
Manifest: {manifest}
Finish with: DREAM {ns} consolidated=<yes|no> findings=<n>
"""


def load_cfg():
    if not CFG.exists():
        CFG_DIR.mkdir(parents=True, exist_ok=True)
        CFG.write_text(json.dumps(DEFAULT_CONFIG, indent=2) + "\n")
        print(f"orchestrator: generated default config {CFG} — mold it, then arm: touch {ENABLED}")
    return json.loads(CFG.read_text())


def run(cmd, timeout=120):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def queue_cli(cfg):
    q = cfg["queue"]
    return ["ssh", q["host"], f"cd {q['repo']} && {q['cli']}"]


def qjson(cfg, args, timeout=60):
    rc, out, err = run(queue_cli(cfg) + args + ["--json"], timeout)
    try:
        return json.loads((out or "").strip())
    except Exception:
        return []


def parse_peak(spec, now):
    """'Mon-Fri 11:30-15:30' -> active?"""
    try:
        days, win = spec.split(" ")
        d0, d1 = days.split("-")
        in_days = days == "Daily" or int(d0) <= now.weekday() <= int(d1)
        start, end = win.split("-")
        sh, sm = map(int, start.split(":"))
        eh, em = map(int, end.split(":"))
        cur = now.hour * 60 + now.minute
        return in_days and (sh * 60 + sm) <= cur < (eh * 60 + em), (eh * 60 + em) - cur
    except Exception:
        return False, 0


def plane_gates(cfg, now=None):
    now = now or datetime.now(IST)
    gates = {}
    for route in cfg["routes"]:
        cal = cfg["quota_calendars"].get(route, {})
        spec = cal.get("peak_ist")
        if spec:
            active, mins_left = parse_peak(spec, now)
            gates[route] = {"ok": not active, "why": "peak window" if active else "",
                            "retry_after_min": mins_left if active else 0}
        else:
            gates[route] = {"ok": True, "why": "", "retry_after_min": 0}
    return gates


def acquire_lock():
    lf = open(LOCK, "w")
    try:
        fcntl.flock(lf, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        print("orchestrator: another pass holds the lock — skipping")
        sys.exit(0)
    return lf


def armed():
    if ENABLED.exists():
        return True
    print(f"orchestrator: not armed ({ENABLED} absent) — no-op (single-host arming law)")
    return False


def load_state():
    try:
        return json.loads(STATE.read_text())
    except Exception:
        return {}


def save_state(st):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps(st, indent=1))


# ---------------------------------------------------------------- topup ----

def stage_brief(cfg, ns, budget):
    q = cfg["queue"]
    ygg = "~/.local/bin/ygg-memory.py"
    to = cfg["supply"]["prepare_timeout_s"]
    rc, out, err = run(["ssh", q["host"],
                        f"timeout {to} python3 {ygg} dream --prepare --ns={shlex.quote(ns)} --force --budget {budget} --json"], to + 30)
    root = f"~/.yggterm/memory/dream/{ns}/pending"
    chk, chk_out, _ = run(["ssh", q["host"],
                           f"test -f {root}/BRIEF.md && echo BRIEF-YES || echo BRIEF-NO; "
                           f"test -f {root}/MANIFEST.json && echo MAN-YES || echo MAN-NO"], 20)
    if "BRIEF-YES" not in chk_out:
        return None
    return {"brief": f"{root}/BRIEF.md", "manifest": f"{root}/MANIFEST.json" if "MAN-YES" in chk_out else ""}


def enqueue(cfg, route, ns, kind, moderate, prompt):
    pf = Path("/tmp") / f"ygg-dream-{uuid.uuid4().hex[:8]}.md"
    pf.write_text(prompt)
    q = cfg["queue"]
    title = f"DREAM[{kind}{'/mod' if moderate else ''}]: {ns} consolidation"
    # ssh JOINS argv into one remote shell string — every variable arg must be
    # shlex-quoted or a title with spaces shatters (measured 2026-10-24).
    repo = q["repo"]
    if repo.startswith("~/"):
        repo = "$HOME/" + repo[2:]  # remote shell expands; shlex.quote would freeze the tilde
    remote = (f"cd {shlex.quote(repo)} && {q['cli']} enqueue"
              f" --route {shlex.quote(route)} --prompt-file {shlex.quote(str(pf))}"
              f" --workspace {shlex.quote(str(HOME / 'gh/zcode-tui'))}"
              f" --title {shlex.quote(title)}")
    up_rc, _, up_err = run(["scp", "-q", str(pf), f"{q['host']}:{pf}"], 30)
    rc, out, err = run(["ssh", q["host"], remote], 90)
    run(["ssh", q["host"], f"rm -f {shlex.quote(str(pf))}"], 20)
    pf.unlink(missing_ok=True)
    if up_rc != 0:
        return False, f"upload failed: {up_err.strip()[:120]}"
    tail = (out + err).strip().splitlines()
    if rc != 0:
        return False, f"rc={rc} " + " | ".join(tail[-3:])
    return True, tail[-1] if tail else "queued"


def cmd_topup(cfg):
    st = load_state()
    today = datetime.now(IST).strftime("%F")
    if st.get("day") != today:
        st = {"day": today, "counts": {}, "moderate": 0, "dry": {}}
    rows = qjson(cfg, ["list"])
    backlog = len([r for r in rows if r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    gates = plane_gates(cfg)
    livable = [r for r in cfg["routes"] if gates[r]["ok"]]
    if not livable:
        print("orchestrator: all planes gated — holding supply (" +
              ", ".join(f"{r}:{gates[r]['why']}" for r in cfg["routes"]) + ")")
        return
    enq, t0 = 0, time.time()
    while (sum(st["counts"].values()) + backlog < cfg["targets"]["day"]
           and backlog < cfg["targets"]["backlog_cap"]
           and enq < cfg["supply"]["pass_max_enqueues"]
           and time.time() - t0 < cfg["supply"]["pass_wall_budget_s"]):
        pick = min((n for n in cfg["namespaces"] if st["counts"].get(n["ns"], 0) < n["per_day"]),
                   key=lambda n: st["counts"].get(n["ns"], 0), default=None)
        if not pick:
            break
        staged = stage_brief(cfg, pick["ns"], pick["budget"])
        if not staged:
            st["dry"][pick["ns"]] = st["dry"].get(pick["ns"], 0) + 1
            save_state(st)
            continue
        n_done = st["counts"].get(pick["ns"], 0)
        route = livable[n_done % len(livable)]
        kind = cfg["kinds"][n_done % len(cfg["kinds"])]
        moderate = kind in ("verify", "sweep")
        ok, line = enqueue(cfg, route, pick["ns"], kind, moderate,
                           CONTRACT.format(ns=pick["ns"], kind=kind, **staged))
        if ok:
            enq += 1
            st["counts"][pick["ns"]] = n_done + 1
            st["moderate"] += 1 if moderate else 0
            save_state(st)
            print(f"orchestrator: dispatched {pick['ns']} on {route}")
        else:
            print(f"orchestrator: enqueue failed: {line}", file=sys.stderr)
            break
        rows = qjson(cfg, ["list"])
        backlog = len([r for r in rows if r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    print(f"orchestrator: topup staged={sum(st['counts'].values())} moderate={st['moderate']} backlog={backlog}")


# ----------------------------------------------------------- supervise ----

QUOTA_SIGNS = ("quota", "usage limit", "rate limit", "429", "hourly", "cap reached", "free tier")
AUTH_SIGNS = ("auth", "unauthorized", "401", "refresh token", "login")


def cmd_supervise(cfg):
    ledger = {"t": int(time.time()), "audited": 0, "revoked": [], "rerouted": [], "held": [], "notes": []}
    rows = qjson(cfg, ["list"])
    gates = plane_gates(cfg)
    day_ago_ms = now_ms - 24 * 3600 * 1000
    seen_notes = set()
    for r in rows:
        low = (r.get("last_error") or "").lower()
        if r.get("status") != "failed":
            continue
        if (r.get("updated_at") or 0) < day_ago_ms:
            continue  # history, not news
        if r["id"] in seen_notes:
            continue
        seen_notes.add(r["id"])
        if any(s in low for s in QUOTA_SIGNS):
            alt = next((p for p in cfg["routes"] if p != r["route"] and gates[p]["ok"]), None)
            if alt:
                rc, _, _ = run(queue_cli(cfg) + ["reroute", r["id"], "--to", alt, "--json"], 60)
                if rc == 0:
                    ledger["rerouted"].append({"id": r["id"], "to": alt, "why": (r.get("last_error") or "")[:80]})
                    print(f"supervise: rerouted {r['id'][:16]} {r['route']} -> {alt} (quota death)")
            else:
                run(queue_cli(cfg) + ["hold", r["id"], "--until-min", "60", "--json"], 60)
                ledger["held"].append({"id": r["id"], "min": 60, "why": "all planes gated"})
                print(f"supervise: held {r['id'][:16]} 60m (all planes gated)")
        elif any(s in low for s in AUTH_SIGNS):
            ledger["notes"].append({"id": r["id"], "why": f"auth death needs ygg-auth: {(r.get('last_error') or '')[:80]}"})
    # false-completion audit: completed runs whose event stream shows an
    # error turn with zero tokens riding rc=0 (the G1 class)
    done = [r for r in rows if r.get("status") == "completed"]
    random.shuffle(done)
    q = cfg["queue"]
    for r in done[: cfg["false_completion"]["audit_per_pass"]]:
        ledger["audited"] += 1
        rc, log, _ = run(["ssh", q["host"],
                          f"tail -c 4000 ~/.local/state/zcode-tui/queue/runs/{r['id']}*.json.log 2>/dev/null || true"], 30)
        flat = log.replace(" ", "")
        if '"stopReason":"error"' in flat and '"totalTokens":0' in flat:
            run(queue_cli(cfg) + ["revoke", r["id"], "--reason", "false-completion: zero-token error turn", "--json"], 60)
            ledger["revoked"].append({"id": r["id"]})
            print(f"supervise: REVOKED false completion {r['id'][:16]}")
    LEDGER.parent.mkdir(parents=True, exist_ok=True)
    with LEDGER.open("a") as f:
        f.write(json.dumps(ledger) + "\n")
    print(f"orchestrator: supervise audited={ledger['audited']} revoked={len(ledger['revoked'])} "
          f"rerouted={len(ledger['rerouted'])} held={len(ledger['held'])}")


def cmd_status(cfg, as_json=False):
    st = load_state()
    gates = plane_gates(cfg)
    rows = qjson(cfg, ["list"])
    backlog = len([r for r in rows if r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    running = len([r for r in rows if r.get("status") == "running"])
    out = {
        "armed": ENABLED.exists(),
        "today": {"staged": sum(st.get("counts", {}).values()), "moderate": st.get("moderate", 0),
                  "targets": cfg["targets"]},
        "planes": gates,
        "queue": {"backlog": backlog, "running": running},
        "dry": st.get("dry", {}),
    }
    if as_json:
        print(json.dumps(out, indent=1))
        return
    t = cfg["targets"]
    print(f"dream orchestrator — armed={out['armed']} staged={out['today']['staged']}/{t['day']} "
          f"moderate={out['today']['moderate']}/{t['moderate']} backlog={backlog} running={running}")
    for r in cfg["routes"]:
        g = gates[r]
        state = "OK" if g["ok"] else f"GATED {g['why']} ({g['retry_after_min']}m)"
        print(f"  {r:16} {state}")


def main():
    verb = sys.argv[1] if len(sys.argv) > 1 else "status"
    cfg = load_cfg()
    if verb == "status":
        cmd_status(cfg, "--json" in sys.argv)
    elif verb == "topup":
        acquire_lock()
        cmd_topup(cfg)
    elif verb == "supervise":
        acquire_lock()
        cmd_supervise(cfg)
    else:
        print("usage: ygg-dream-orchestrator.py <status|topup|supervise> [--json]")
        sys.exit(2)


if __name__ == "__main__":
    main()
