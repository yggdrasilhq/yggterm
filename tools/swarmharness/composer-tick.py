#!/usr/bin/env python3
"""composer-tick — thin deterministic runner for the composer kernel.

Loads the owner-approved enrollment manifest (default-deny), runs ONE
bounded kernel tick, appends the report to ~/.yggterm/log/composer-tick.jsonl.
No kernel logic lives here — this file only wires manifest -> tick().
"""
import json
import sys
import time
from pathlib import Path

KERNEL_DIR = Path.home() / ".yggterm/swarm/composer/kernel"
MANIFEST = Path.home() / ".yggterm/config/swarm/composer-sources.json"
STATE = Path.home() / ".yggterm/swarm/composer"
LOG = Path.home() / ".yggterm/log/composer-tick.jsonl"

sys.path.insert(0, str(KERNEL_DIR))
import composer  # noqa: E402


def main():
    m = json.loads(MANIFEST.read_text())
    rep = composer.tick(
        m.get("sources"), state_dir=str(STATE),
        max_bytes_per_source=m.get("bounds", {}).get("max_bytes_per_source", 262144),
        claim_cap=m.get("bounds", {}).get("claim_cap", 5),
        max_pages=m.get("bounds", {}).get("max_pages", 10),
        max_outbox=m.get("bounds", {}).get("max_outbox", 200))
    rep["enrolled_by"] = "owner directive 2026-10-26"
    LOG.parent.mkdir(parents=True, exist_ok=True)
    with LOG.open("a") as f:
        f.write(json.dumps(rep) + "\n")
    print(json.dumps({k: rep.get(k) for k in
                      ("census_sources", "gaps", "pages_read", "observations",
                       "patterns", "proposals", "scanned_lineages",
                       "enrolled_lineages", "backlog_remaining")}))
    for n in rep.get("notes", [])[:5]:
        print("  note:", n)
    # loop-closure trigger: if the outbox gained new proposals, the
    # brain-pair review runs automatically (idempotent per batch). Its
    # failure is review DEBT — reported distinctly, mining stays success.
    try:
        import composer_review
        review_rc = composer_review.main()  # prints its own noop/review line
    except Exception as e:  # a broken review module never breaks mining
        review_rc = None
        print("  review-step error:", e)
    if review_rc:
        print(f"  review-debt: proposal review did NOT complete "
              f"(exit {review_rc}) — debt retained, mining unaffected")
    return 0


if __name__ == "__main__":
    sys.exit(main())
