"""ygg-swarm-executor@ — the queue transport adapter (sol turn-5 split).

The unit's ExecStart hands the INSTALLED serve command after `--`; this
adapter is the versioned gate in front of it:

  - reads the ACTIVATED config (active.json), never the workspace;
  - containment-only (no usable config)  -> exit 75, systemd backs off;
  - this host is not fleet.dispatch_host -> exit 75 (replicas never serve);
  - otherwise exec the installed transport verbatim.

No model, no network judgment, no config parsing beyond the kernel loader.
"""
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import swarmconfig as sc  # noqa: E402


def main():
    res = sc.load()
    mode = res["mode"]
    if mode == "containment_only":
        faults = "; ".join(f.get("detail", f["code"]) for f in res.get("faults", [])[:3])
        print(f"executor-adapter: containment-only ({faults}) — refusing to serve; "
              f"maintenance keeps supervising", file=sys.stderr)
        return 75  # EX_TEMPFAIL: systemd Restart=on-failure backs off and retries
    cfg = res.get("config") or {}
    dispatch = (cfg.get("fleet") or {}).get("dispatch_host")
    host = os.uname().nodename.split(".")[0]
    if dispatch and host != dispatch:
        print(f"executor-adapter: dispatch_host={dispatch}, this host={host} — "
              f"not the dispatch authority", file=sys.stderr)
        return 75
    if "--" not in sys.argv:
        print("executor-adapter: no transport command after --", file=sys.stderr)
        return 2
    argv = sys.argv[sys.argv.index("--") + 1:]
    if not argv:
        print("executor-adapter: empty transport command", file=sys.stderr)
        return 2
    try:
        sc.ack("executor-adapter")   # consumer acknowledgement (distinct state)
    except Exception:
        pass
    print(f"executor-adapter: config {res['revision']['content_hash'][:12]} "
          f"rev {res['revision']['revision']} — exec {argv[0]}", file=sys.stderr)
    os.execvp(argv[0], argv)


if __name__ == "__main__":
    sys.exit(main())
