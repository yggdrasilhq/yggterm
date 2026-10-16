#!/usr/bin/env python3
"""Jev 1.13 SHADOW client for the slice-6 routing corpus (TypeSafe first-party).

Discipline encoded here (jev skill + slice-6 contract):
  - cache by canonical request sha (model/state/instructions/criteria incl. order)
  - session counters: $0.10 OR 1000 attempts, 2 req/s, book usage tokens
  - unknown on ANY failure (transport/malformed/timeout) — never a guess
  - 'order' variant cases permute the criteria option order deterministically
  - SHADOW ONLY: writes nothing to any source, promotes nothing
"""
import hashlib
import json
import os
import time
import urllib.request

ENDPOINT = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-latest"
CACHE_PATH = os.path.expanduser("~/.cache/dream-routing/jev-cache.jsonl")
COUNTERS_PATH = os.path.expanduser("~/.cache/dream-routing/session-counters.json")
BUDGET_USD = 0.10
BUDGET_ATTEMPTS = 1000
RATE_S = 0.5  # 2 req/s

INSTRUCTIONS = {
    "hub": "Which supplied namespace owns this memory door (name, description, body excerpt)? Answer from the door's own content only.",
    "notes": "Which supplied section/book owns this note (title + text excerpt)? Answer from the note's own content only.",
    "cases": "Which supplied case does this passage belong to? Answer from the passage's own content only; use unknown when nothing specific matches.",
    "calls": "Which recorded conversation circle does this transcript excerpt belong to? Answer from the excerpt's own content only; use unknown when it cannot be told.",
}


def _load_counters():
    if os.path.exists(COUNTERS_PATH):
        return json.load(open(COUNTERS_PATH))
    return {"attempts": 0, "usd": 0.0, "input_tokens": 0, "retries": 0}


def _save_counters(c):
    os.makedirs(os.path.dirname(COUNTERS_PATH), exist_ok=True)
    json.dump(c, open(COUNTERS_PATH, "w"), indent=1)


def _load_cache():
    cache = {}
    if os.path.exists(CACHE_PATH):
        for line in open(CACHE_PATH, encoding="utf-8"):
            try:
                r = json.loads(line)
                cache[r["key"]] = r
            except (json.JSONDecodeError, KeyError):
                continue
    return cache


def _permute(options, case_id):
    h = int(hashlib.sha256(case_id.encode()).hexdigest(), 16)
    order = sorted(range(len(options)), key=lambda i: (h >> (i * 3)) % 997)
    return [options[i] for i in order]


def classify(case, options, config, key=None, dry_run=False):
    """Returns dict(prediction, source, usage, latency_ms, request_sha)."""
    kind = config["families"][case["family"]]["kind"]
    opts = list(options) + ["none", "unknown"]
    if case.get("variant") == "order":
        opts = _permute(opts, case["case_id"])  # option-order instability probe
    payload = {
        "model": MODEL,
        "state": {"text": case["input"]["text"], "source_ref": case["source_ref"]},
        "questions": {
            "route": {
                "type": "choice",
                "instructions": INSTRUCTIONS[kind],
                "criteria": {o: f"route to {o}" for o in opts},
            },
        },
    }
    request_sha = hashlib.sha256(json.dumps(payload, sort_keys=True,
                                            ensure_ascii=False).encode()).hexdigest()
    if key is None:
        key = request_sha
    counters = _load_counters()
    if counters["attempts"] >= BUDGET_ATTEMPTS or counters["usd"] >= BUDGET_USD:
        return {"prediction": "unknown", "source": "budget-exhausted",
                "usage": None, "latency_ms": 0, "request_sha": request_sha}
    cache = _load_cache()
    if key in cache:
        hit = cache[key]
        return {"prediction": hit["prediction"], "source": "cache",
                "usage": hit.get("usage"), "latency_ms": 0,
                "request_sha": request_sha}
    if dry_run:
        return {"prediction": None, "source": "dry-run", "usage": None,
                "latency_ms": 0, "request_sha": request_sha}
    api_key = os.environ.get("JEV_API_KEY") or os.environ.get("TYPESAFE_API_KEY")
    if not api_key:
        return {"prediction": "unknown", "source": "no-key", "usage": None,
                "latency_ms": 0, "request_sha": request_sha}
    counters["attempts"] += 1
    _save_counters(counters)
    time.sleep(RATE_S)
    req = urllib.request.Request(
        ENDPOINT, data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + api_key,
                 "Content-Type": "application/json"}, method="POST")
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=25) as r:
            resp = json.load(r)
    except Exception:
        counters["retries"] += 1
        _save_counters(counters)
        return {"prediction": "unknown", "source": "transport-error",
                "usage": None, "latency_ms": int((time.time() - t0) * 1000),
                "request_sha": request_sha}
    latency = int((time.time() - t0) * 1000)
    try:
        answer = resp["answers"]["route"]
        pred = answer.get("choice") or answer.get("value") or answer.get("answer")
        usage = resp.get("usage") or answer.get("usage")
        if usage and usage.get("input_tokens"):
            counters["input_tokens"] += usage["input_tokens"]
            counters["usd"] += usage["input_tokens"] * 0.042 / 1_000_000
            _save_counters(counters)
        if pred not in opts:
            return {"prediction": "unknown", "source": "invalid-option",
                    "usage": usage, "latency_ms": latency, "request_sha": request_sha}
        entry = {"key": key, "request_sha": request_sha, "prediction": pred,
                 "usage": usage, "model": MODEL, "ts": time.time()}
        os.makedirs(os.path.dirname(CACHE_PATH), exist_ok=True)
        with open(CACHE_PATH, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry, ensure_ascii=False) + "\n")
        return {"prediction": pred, "source": "live", "usage": usage,
                "latency_ms": latency, "request_sha": request_sha}
    except (KeyError, IndexError, TypeError):
        return {"prediction": "unknown", "source": "malformed-response",
                "usage": None, "latency_ms": latency, "request_sha": request_sha}
