"""Loader/validator for ~/.yggterm/swarmconfig.yaml - the swarm survival
ledger. Owner-editable in catastrophe; the kernel degrades, never dies:

  parse ok        -> config, "ok"
  parse fail      -> LAST-GOOD copy (kept on every successful parse), loud fault
  no file + no lg -> None + fault (the orchestrator runs floors-only and
                     reports the config fault in every ledger row)
"""
import os
import shutil
from pathlib import Path

try:
    import yaml
except ImportError:  # kernel portability: pyyaml is the only non-stdlib dep
    yaml = None

DEFAULT_PATH = Path(os.path.expanduser("~/.yggterm/swarmconfig.yaml"))
KNOWN_MAJOR = 1


def load(path=DEFAULT_PATH):
    """-> (cfg_dict_or_None, status_note). Keeps/uses <path>.last-good."""
    p = Path(os.path.expanduser(str(path)))
    lg = p.with_suffix(p.suffix + ".last-good")
    if yaml is None:
        return _last_good(lg), "pyyaml unavailable - using last-good"
    try:
        cfg = yaml.safe_load(p.read_text())
        err = validate(cfg)
        if err:
            raise ValueError(err)
        shutil.copyfile(p, lg)  # parse+validate ok: refresh last-good
        return cfg, "ok"
    except FileNotFoundError:
        if lg.exists():
            return _last_good(lg), f"{p} absent - using last-good"
        return None, f"{p} absent, no last-good"
    except Exception as e:
        return _last_good(lg), f"config INVALID ({str(e)[:160]}) - using last-good; OWNER: fix {p}"


def _last_good(lg):
    try:
        cfg = yaml.safe_load(lg.read_text())
        err = validate(cfg)
        if err:
            return None, f"last-good also invalid: {err}"
        return cfg, "ok (last-good)"
    except Exception as e:
        return None, f"last-good unreadable: {str(e)[:120]}"


def validate(cfg):
    """-> error string or None. Fails closed on newer major versions."""
    if not isinstance(cfg, dict):
        return "config is not a mapping"
    v = cfg.get("schema_version")
    if not isinstance(v, int) or v < 1:
        return "schema_version missing/not int"
    if v > KNOWN_MAJOR:
        return f"schema_version {v} newer than kernel major {KNOWN_MAJOR} - update the kernel first"
    for section in ("kernel", "intelligence", "orchestrators", "planes"):
        if section not in cfg:
            return f"missing section: {section}"
    for route, spec in (cfg.get("planes") or {}).items():
        if not isinstance(spec, dict) or "state" not in spec:
            return f"planes.{route} lacks state"
    return None


def intelligence_available(cfg):
    """-> (primary_desc, fallback_desc). Static read; live reachability is a
    decision-time consult concern, not a config truth."""
    intel = (cfg or {}).get("intelligence") or {}
    return intel.get("primary", {}), intel.get("fallback", "none")
