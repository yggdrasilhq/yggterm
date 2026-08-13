#!/usr/bin/env python3
"""Continuous resource recorder — the black box for "why is this machine hot?".

⛔ WHY THIS IS AN EXTERNAL OBSERVER AND NOT MORE APP TELEMETRY.
`crates/yggterm-core/src/render_probe.rs` already samples every role every 60 s
and it faithfully recorded a 22-hour CPU regression that nobody noticed, because
nothing read the file and nothing alarmed on it. Building a second in-app
sampler would repeat that mistake. This process lives OUTSIDE the app, so it
keeps recording across GUI restarts, daemon swaps and crashes — which is
precisely when the interesting samples happen — and it writes to a queryable
store rather than an append-only log nobody opens.

WHAT IT RECORDS, and why each column is here rather than an easier one:

- **CPU as delta jiffies between consecutive samples, split user/kernel.**
  Never `ps %CPU`, which is an average over the process's whole LIFETIME and has
  misled this campaign more than once. The user/kernel split halves the search on
  sight: kernel-dominant means syscalls, user-dominant means compute.
- **`rss_kb` AND `swap_kb`.** RSS alone undercounts badly once the machine
  swaps; the number that has to fit in the machine is the sum.
- **`threads` and `fds`.** The idle-cost regression on this fleet is a
  population of *cheap* objects — the heap stops growing while CPU keeps
  climbing — so a count is the signal and bytes are not.
- **`webkit_datastore_threads` / `receive_queue_threads`.** The one confirmed
  instance of that shape: 11 and 13 on an aged GUI, 1 and 2 on a fresh one. This
  is the growth counter the campaign is actually hunting, recorded by name so a
  successor does not have to rediscover it.
- **Temperatures.** The owner's report is "the fan is running", and that is the
  symptom the whole mandate exists to answer. A CPU number that never touches
  heat cannot be checked against what he actually experiences.

RETENTION: a byte budget, not a time budget (default 10 GB). Time-based pruning
throws away the long baseline that makes a regression visible in the first
place; when the cap is hit this deletes the OLDEST samples and reclaims with an
incremental vacuum.

USAGE
  resource-recorder.py run [--interval 10] [--db PATH] [--max-gb 10]
  resource-recorder.py status [--db PATH]

Run it detached; it is a plain process with no service dependency:
  setsid nohup scripts/resource-recorder.py run >/dev/null 2>&1 &
"""

import argparse
import os
import re
import sqlite3
import sys
import time
from pathlib import Path

DEFAULT_DB = Path.home() / ".yggterm" / "resource-metrics.db"
# Everything whose cost this campaign is accountable for, plus the desktop
# neighbours that share the CPU with it — a burner outside the list would
# otherwise read as "unexplained machine load".
TRACKED = re.compile(
    r"yggterm|WebKit|ychrome|helium|kwin|plasma|easyeffects|pulse|pipewire|tailscaled|Xwayland",
    re.I,
)
SCHEMA = """
CREATE TABLE IF NOT EXISTS samples (
  ts_ms INTEGER NOT NULL,
  pid INTEGER NOT NULL,
  comm TEXT NOT NULL,
  role TEXT NOT NULL,
  age_s INTEGER,
  cpu_user_pct REAL,      -- % of ONE core over the interval, not a lifetime average
  cpu_kernel_pct REAL,
  rss_kb INTEGER,
  swap_kb INTEGER,
  threads INTEGER,
  fds INTEGER,
  webkit_datastore_threads INTEGER,
  receive_queue_threads INTEGER
);
CREATE INDEX IF NOT EXISTS samples_ts ON samples(ts_ms);
CREATE INDEX IF NOT EXISTS samples_role_ts ON samples(role, ts_ms);

CREATE TABLE IF NOT EXISTS system (
  ts_ms INTEGER PRIMARY KEY,
  load1 REAL,
  mem_used_mb INTEGER,
  mem_total_mb INTEGER,
  swap_used_mb INTEGER,
  temp_max_c REAL,
  temp_alarm INTEGER
);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT);
"""


def classify(comm, cmdline):
    """Role, not just comm. `comm` truncates at 15 chars and several distinct
    processes collapse onto the same string there, which is how a daemon and a
    GUI end up looking like one population.

    ⛔ AND THE TRUNCATION MUST BE MATCHED EXPLICITLY, not just tolerated.
    `/proc/<pid>/cmdline` reads back EMPTY whenever the process is mid-exec or
    being reaped, and under load that happens often enough to matter. When it
    does, the only evidence left is `comm` — which the kernel has already cut to
    "WebKitWebProces" (no trailing 's'). Matching the full spelling then falls
    through to the generic branch and invents a SECOND role for the same
    process. Caught live: `web_content` at 20.5% sitting beside a phantom
    `webkitwebproces` at 14.3%, i.e. a third of the web process's cost filed
    under a name nobody would think to add up.
    """
    c = (cmdline or "") + " " + comm
    if "WebKitWebProces" in c:  # truncated spelling is a prefix of the full one
        return "web_content"
    if "WebKitNetworkPr" in c:
        return "web_network"
    if "yggterm-headless" in c:
        return "daemon"
    if re.search(r"/yggterm($|\s)", c) or comm == "yggterm":
        return "gui"
    if "ychrome" in c:
        return "ychrome"
    return comm.lower()


def read_stat(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            data = f.read()
        # comm can contain spaces and parens; everything after the LAST ')' is
        # positional. Splitting on whitespace from the left gets this wrong for
        # any process whose name has a space in it.
        rparen = data.rindex(")")
        fields = data[rparen + 2 :].split()
        return int(fields[11]), int(fields[12]), int(fields[19])  # utime, stime, starttime
    except Exception:
        return None


def read_mem(pid):
    rss = swap = 0
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
                elif line.startswith("VmSwap:"):
                    swap = int(line.split()[1])
    except Exception:
        pass
    return rss, swap


def thread_census(pid):
    """Total threads plus the two named populations that grow per EVENT — the
    shape this campaign established is the leak (an idle app grows neither)."""
    total = ds = rq = 0
    try:
        tasks = os.listdir(f"/proc/{pid}/task")
    except Exception:
        return 0, 0, 0
    total = len(tasks)
    for t in tasks:
        try:
            with open(f"/proc/{pid}/task/{t}/comm") as f:
                name = f.read().strip()
        except Exception:
            continue
        # The thread name is truncated to 15 chars by the kernel, so
        # "WebsiteDataStore" arrives as "ebsiteDataStore" — matching the full
        # spelling silently counts zero forever.
        if "ebsiteDataStore" in name:
            ds += 1
        elif "ReceiveQueue" in name:
            rq += 1
    return total, ds, rq


def fd_count(pid):
    try:
        return len(os.listdir(f"/proc/{pid}/fd"))
    except Exception:
        return 0


def cmdline(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode("utf8", "replace")[:400]
    except Exception:
        return ""


def temps():
    hi, alarm = 0.0, 0
    base = Path("/sys/class/hwmon")
    if not base.exists():
        return hi, alarm
    for hw in base.glob("hwmon*"):
        for inp in hw.glob("temp*_input"):
            try:
                v = int(inp.read_text().strip()) / 1000.0
            except Exception:
                continue
            hi = max(hi, v)
            crit = inp.with_name(inp.name.replace("_input", "_max"))
            try:
                if crit.exists() and v >= int(crit.read_text().strip()) / 1000.0:
                    alarm = 1
            except Exception:
                pass
    return hi, alarm


def sysinfo():
    load1 = float(open("/proc/loadavg").read().split()[0])
    mem = {}
    for line in open("/proc/meminfo"):
        k, v = line.split(":", 1)
        mem[k] = int(v.split()[0])
    used = (mem["MemTotal"] - mem["MemAvailable"]) // 1024
    swap_used = (mem.get("SwapTotal", 0) - mem.get("SwapFree", 0)) // 1024
    t, alarm = temps()
    return load1, used, mem["MemTotal"] // 1024, swap_used, t, alarm


def open_db(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(str(path))
    db.executescript(SCHEMA)
    # WAL so a reader (a successor asking "what is burning?") never blocks the
    # recorder, and the recorder never blocks the reader.
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=NORMAL")
    db.execute("PRAGMA auto_vacuum=INCREMENTAL")
    db.commit()
    return db


def prune(db, path, max_bytes):
    """Byte-budget retention. Deleting rows alone does not shrink the file, so
    the incremental vacuum is not optional here — without it the cap is
    enforced against a number that never falls."""
    try:
        size = path.stat().st_size
    except FileNotFoundError:
        return
    if size < max_bytes:
        return
    row = db.execute("SELECT MIN(ts_ms), MAX(ts_ms) FROM samples").fetchone()
    if not row or row[0] is None:
        return
    cut = row[0] + (row[1] - row[0]) // 5  # drop the oldest fifth
    db.execute("DELETE FROM samples WHERE ts_ms < ?", (cut,))
    db.execute("DELETE FROM system WHERE ts_ms < ?", (cut,))
    db.commit()
    db.execute("PRAGMA incremental_vacuum")
    db.commit()


def run(args):
    db = open_db(args.db)
    db.execute(
        "INSERT OR REPLACE INTO meta(k,v) VALUES('recorder_started_ms',?)",
        (str(int(time.time() * 1000)),),
    )
    db.commit()
    prev = {}
    clk = os.sysconf("SC_CLK_TCK")
    while True:
        t0 = time.time()
        # ONE timestamp per round, taken before the walk. Stamping each process
        # as it is read makes every row's ts unique, so "group by round" — the
        # natural way to ask what the machine was doing at an instant — silently
        # returns one row per process instead.
        now_ms = int(t0 * 1000)
        rows = []
        for entry in os.scandir("/proc"):
            if not entry.name.isdigit():
                continue
            pid = int(entry.name)
            cl = cmdline(pid)
            try:
                comm = open(f"/proc/{pid}/comm").read().strip()
            except Exception:
                continue
            if not TRACKED.search(cl + " " + comm):
                continue
            st = read_stat(pid)
            if not st:
                continue
            utime, stime, starttime = st
            key = (pid, starttime)  # pid alone is reused; starttime disambiguates
            if key in prev:
                pu, ps_, pt = prev[key]
                dt = (now_ms - pt) / 1000.0
                if dt > 0:
                    upct = (utime - pu) / clk / dt * 100
                    kpct = (stime - ps_) / clk / dt * 100
                    rss, swap = read_mem(pid)
                    threads, ds, rq = thread_census(pid)
                    try:
                        age = int(t0 - (os.stat(f"/proc/{pid}").st_ctime))
                    except Exception:
                        age = None
                    rows.append(
                        (now_ms, pid, comm, classify(comm, cl), age, upct, kpct,
                         rss, swap, threads, fd_count(pid), ds, rq)
                    )
            prev[key] = (utime, stime, now_ms)
        if rows:
            db.executemany(
                "INSERT INTO samples VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", rows
            )
        l1, mu, mt, su, tc, al = sysinfo()
        db.execute(
            "INSERT OR REPLACE INTO system VALUES (?,?,?,?,?,?,?)",
            (int(time.time() * 1000), l1, mu, mt, su, tc, al),
        )
        db.commit()
        prune(db, args.db, int(args.max_gb * 1024**3))
        time.sleep(max(1.0, args.interval - (time.time() - t0)))


def status(args):
    if not args.db.exists():
        print(f"no db at {args.db}")
        return 1
    db = open_db(args.db)
    size_mb = args.db.stat().st_size / 1024**2
    n, lo, hi = db.execute("SELECT COUNT(*), MIN(ts_ms), MAX(ts_ms) FROM samples").fetchone()
    print(f"db={args.db}  {size_mb:.1f} MB  samples={n}")
    if not n:
        return 0
    print(f"span={(hi-lo)/3600000:.1f} h  last={time.strftime('%H:%M:%S', time.localtime(hi/1000))}")
    print("\nlast 5 min, by role — % of ONE core (mean), and the growth counters:")
    for r in db.execute(
        """SELECT role, ROUND(AVG(cpu_user_pct),1), ROUND(AVG(cpu_kernel_pct),1),
                  ROUND(AVG(cpu_user_pct+cpu_kernel_pct),1),
                  ROUND(AVG(rss_kb)/1024.0), MAX(threads),
                  MAX(webkit_datastore_threads), MAX(receive_queue_threads)
           FROM samples WHERE ts_ms > ? GROUP BY role
           ORDER BY 4 DESC LIMIT 10""",
        (hi - 300000,),
    ):
        print(f"  {r[0]:14s} total={r[3]:6}%  user={r[1]:6}  kern={r[2]:6}  "
              f"rss={r[4] or 0:6.0f}MB  thr={r[5]}  wds={r[6]} rq={r[7]}")
    s = db.execute("SELECT load1, mem_used_mb, swap_used_mb, temp_max_c, temp_alarm "
                   "FROM system ORDER BY ts_ms DESC LIMIT 1").fetchone()
    if s:
        print(f"\nsystem: load={s[0]}  mem={s[1]}MB  swap={s[2]}MB  "
              f"temp_max={s[3]}C{'  ⚠ALARM' if s[4] else ''}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["run", "status"])
    ap.add_argument("--db", type=Path, default=DEFAULT_DB)
    ap.add_argument("--interval", type=float, default=10.0)
    ap.add_argument("--max-gb", type=float, default=10.0)
    a = ap.parse_args()
    return run(a) if a.cmd == "run" else status(a)


if __name__ == "__main__":
    sys.exit(main() or 0)
