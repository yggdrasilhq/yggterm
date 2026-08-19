#!/usr/bin/env python3
"""
sanitize-titles.py — fleet-wide title audit + LLM rescue via interface LLM (gpt-5.6-luna).

Phase 1 (harness): identify *why* yggterm shows a weird title for each CLI type.
  - Fetches verb ground truth: `server startpage ls --json` + `server cwdtree ls --json`
  - Manually walks stores via AGENT_CLIS globs (independent of Rust) to get expected
  - Flags weird titles: shorthash, raw path, "New session", generated-fallback,
    low-signal heuristics (mirrors yggterm_core::looks_like_generated_fallback_title)

Phase 2 (rescue): batches weird sessions to the wired LiteLLM endpoint
  (the fleet's wired LiteLLM proxy) with model chatgpt/gpt-5.6-luna.
  Uses the same prompt as crates/yggterm-core/src/titles.rs request_litellm_title.
  Writes back via SessionTitleStore when --write is passed; otherwise dry-run.

Usage:
  python3 scripts/sanitize-titles.py --host *** --host oc --host local --verbose
  python3 scripts/sanitize-titles.py --host *** --rescue --model chatgpt/gpt-5.6-luna
  python3 scripts/sanitize-titles.py --rescue --write --model chatgpt/gpt-5.6-luna

Env:
  LITELLM_ENDPOINT / LITELLM_API_KEY / INTERFACE_LLM_MODEL override settings.json
  YGGTERM_CHECK_HOSTS overrides host discovery.

Requires: yggterm-headless on each host, ssh BatchMode, optional: pip install requests
"""
import argparse
import json
import os
import re
import shlex
import subprocess
import sys
from pathlib import Path
from collections import defaultdict

# Keep CLI_STORES in sync with crates/yggterm-core/src/agent_cli.rs
CLI_STORES = [
    {"slug": "muse", "globs": [".local/share/muse/sessions/**/session.jsonl"], "exclude": ["/subagent/", "/tool-outputs/"], "kind": "muse"},
    {"slug": "codex", "globs": [".codex/sessions/**/rollout-*.jsonl"], "exclude": [".bak."], "kind": "codex"},
    {"slug": "codex-litellm", "globs": [".codex-litellm/sessions/**/rollout-*.jsonl"], "exclude": [".bak."], "kind": "codex-litellm"},
    {"slug": "claude-code", "globs": [".claude/projects/*/*.jsonl"], "exclude": ["agent-", "/subagents/", "/workflows/"], "kind": "claude-code"},
    {"slug": "pi", "globs": [".pi/agent/sessions/*/*.jsonl"], "exclude": [], "kind": "pi"},
    {"slug": "qwen", "globs": [".qwen/projects/*/chats/*.jsonl"], "exclude": [".runtime."], "kind": "qwen"},
    {"slug": "antigravity", "globs": [".gemini/antigravity-cli/conversations/*.db", ".gemini/antigravity-cli/brain/*/.system_generated/logs/transcript.jsonl"], "exclude": ["-shm", "-wal"], "kind": "antigravity"},
    {"slug": "grok", "globs": [".grok/sessions/*/*/summary.json"], "exclude": [], "kind": "grok-build"},
]

# From crates/yggterm-core/src/titles.rs + lib.rs low-signal checks (replicated)
WEIRD_PATTERNS = [
    (re.compile(r"^[0-9a-fA-F]{7,8}$"), "shorthash 7-8 hex"),
    (re.compile(r"(?i)^remote\s+[a-z-]+\s+[0-9a-f]{7,8}$"), "remote breed shorthash"),
    (re.compile(r"(?i)^[0-9a-f]{7,8}\s*[-·/]"), "shorthash prefix"),
    (re.compile(r"^local::"), "raw scheme local::"),
    (re.compile(r"^live::"), "raw scheme live::"),
    (re.compile(r"^document::"), "raw scheme document::"),
    (re.compile(r"^codex::"), "raw scheme codex::"),
    (re.compile(r"^codex-litellm::"), "raw scheme codex-litellm::"),
    (re.compile(r"/home/"), "raw absolute path"),
    (re.compile(r"^/home"), "raw absolute path"),
    (re.compile(r"^/"), "raw absolute path"),
    (re.compile(r"(?i)\bhome\s+(codex|claude|claude-code|claude code|muse|antigravity|pi|qwen|grok|opencode|kimi|shell|terminal)\b"), "user home breed concatenation"),
    (re.compile(r"(?i)^local\s+(codex|claude|claude-code|claude code|muse|antigravity|pi|qwen|grok|opencode|kimi|shell|terminal|session)$"), "local breed concatenation"),
    (re.compile(r"(?i)^(codex|claude|claude-code|claude code|muse|antigravity|pi|qwen|grok|opencode|kimi)\s+session$"), "generic breed session"),
    (re.compile(r"(?i)^new\s+(session|terminal|ychrome|muse|antigravity|codex|claude|pi|qwen|grok|kimi|opencode)(\s+session)?$"), "New session placeholder"),
    (re.compile(r"(?i)^untitled"), "Untitled placeholder"),
    (re.compile(r"(?i)^unknown"), "Unknown placeholder"),
    (re.compile(r"(?i)^(hi|hello|hey|test|/status|/help|/context|/clear)$"), "single-word low-signal"),
]

GENERATED_FALLBACK_FRAGMENTS = [
    "how use skills", "dev sta", "fix issue", "work session", "debug ui",
    "need help", "fix but only typed first", "qabc", "remote codex",
]

def looks_like_weird_title(title: str, cwd: str = "") -> str | None:
    if not title or not title.strip():
        return "empty title"
    t = title.strip()
    for pat, reason in WEIRD_PATTERNS:
        if pat.search(t):
            return reason
    low = t.lower()
    if len(t) <= 8 and low in ("hi","hello","hey","test","/status","/help"):
        return "ultra-short low-signal"
    for frag in GENERATED_FALLBACK_FRAGMENTS:
        if frag in low:
            return f"generated-fallback fragment '{frag}'"
    # Title equals cwd (the "raw path session" bug)
    if cwd and t == cwd:
        return "title == cwd"
    if cwd and cwd in t and len(t) < len(cwd)+10:
        return "title is cwd-derived"
    # shorthash with prefix like "abc123 - /home"
    if re.match(r"^[0-9a-f]{7,8}\s*[-·]", t):
        return "shorthash prefix"
    return None

def run_on_host(host, cmd, timeout=45):
    if host == "local":
        full = cmd
    else:
        full = f"ssh -o ConnectTimeout=5 -o BatchMode=yes {shlex.quote(host)} {shlex.quote(cmd)}"
    try:
        out = subprocess.check_output(full, shell=True, text=True, timeout=timeout, stderr=subprocess.STDOUT)
        return out, None
    except subprocess.CalledProcessError as e:
        return e.output, f"exit {e.returncode}"
    except Exception as e:
        return "", str(e)

def fleet_hosts():
    hosts = ["local"]
    try:
        out = subprocess.check_output(["yggterm-headless","server","daemons","--json"], text=True, timeout=5)
        data = json.loads(out)
        for entry in data.get("daemons",[])+data.get("machines",[]):
            label = entry.get("label") or entry.get("host") or entry.get("machine_key")
            if label and label not in hosts:
                hosts.append(label)
    except Exception:
        pass
    try:
        out = subprocess.check_output(["bash","-c","grep -h '^Host ' ~/.ssh/config 2>/dev/null | awk '{print $2}'"], text=True)
        for h in out.split():
            if h not in hosts and h not in ("*",):
                hosts.append(h)
    except Exception:
        pass
    env = os.environ.get("YGGTERM_CHECK_HOSTS")
    if env:
        hosts = [h.strip() for h in env.split(",") if h.strip()]
    return hosts

def verb_on_host(host, verb):
    if verb == "startpage":
        cmd = "~/.local/bin/yggterm-headless server startpage ls --json --limit 10000 2>&1 || yggterm-headless server startpage ls --json --limit 10000 2>&1 || yggterm server startpage ls --json --limit 10000 2>&1"
    else:
        cmd = "~/.local/bin/yggterm-headless server cwdtree ls --json --limit 10000 2>&1 || yggterm-headless server cwdtree ls --json --limit 10000 2>&1 || yggterm server cwdtree ls --json --limit 10000 2>&1"
    out, err = run_on_host(host, cmd)
    if err:
        return None, err, out
    try:
        data = json.loads(out)
        return data, None, out
    except Exception as e:
        return None, f"json parse: {e}", out[:3000]

def get_litellm_config(host="local"):
    # Prefer env, then yggterm settings.json on that host, then default
    endpoint = os.environ.get("LITELLM_ENDPOINT") or os.environ.get("litellm_endpoint")
    api_key = os.environ.get("LITELLM_API_KEY") or os.environ.get("litellm_api_key")
    model = os.environ.get("INTERFACE_LLM_MODEL") or "chatgpt/gpt-5.6-luna"
    # Try to read from host's yggterm settings.json
    cmd = "cat ~/.yggterm/settings.json 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get(\"litellm_endpoint\",\"\")); print(d.get(\"litellm_api_key\",\"\")); print(d.get(\"interface_llm_model\",\"\"))' 2>/dev/null"
    out, _ = run_on_host(host, cmd)
    if out:
        lines = [l.strip() for l in out.splitlines()]
        if len(lines) >= 1 and lines[0] and not endpoint:
            endpoint = lines[0]
        if len(lines) >= 2 and lines[1] and not api_key:
            api_key = lines[1]
        if len(lines) >= 3 and lines[2]:
            model = lines[2]
    # Fleet default — read from yggterm settings or env; fleet's wired proxy is private and not hardcoded in public repo
    if not endpoint:
        endpoint = os.environ.get("YGGTERM_LITELLM_ENDPOINT") or ""
        if not endpoint:
            # Try fleet private config (not in git) — e.g. ~/.config/yggterm/litellm_endpoint
            try:
                with open(os.path.expanduser("~/.config/yggterm/litellm_endpoint")) as f:
                    endpoint = f.read().strip()
            except:
                pass
        # Final: leave empty — caller must set env or settings.json litellm_endpoint
        if not endpoint:
            endpoint = ""
    if not endpoint.endswith("/v1"):
        endpoint = endpoint.rstrip("/") 
        if not endpoint.endswith("/v1"):
            endpoint = endpoint + "/v1"
    return endpoint, api_key or "", model

def extract_tail_context(host, path, cli_slug, limit_chars=6000):
    # Use yggterm's own extraction when available: generation_context_from_messages tail 96
    # Fallback: cat last 96 jsonl lines via tail, or heuristic_title path
    if cli_slug == "muse":
        # Muse JSONL is payload_type runtime.user_intent + model_messages
        cmd = f"python3 -c '\nimport json\npath={shlex.quote(path)}\ntry:\n with open(path) as f:\n  lines=f.readlines()[-200:]\n ctx=[]\n for l in lines:\n  try:\n   j=json.loads(l)\n   pt=j.get(\"payload_type\") or j.get(\"type\")\n   if pt==\"runtime.user_intent\" and j.get(\"payload\",{{}}).get(\"accepted\"):\n    txt=j[\"payload\"].get(\"text\",\"\")\n    if txt: ctx.append(\"USER: \"+txt[:500])\n   mm=j.get(\"payload\",{{}}).get(\"model_messages\") if isinstance(j.get(\"payload\"),dict) else None\n   if mm and isinstance(mm, list) and mm:\n    m0=mm[0]\n    if isinstance(m0,dict):\n     c=m0.get(\"content\")\n     if isinstance(c,list) and c:\n      t=c[0].get(\"text\") if isinstance(c[0],dict) else None\n      if t: ctx.append(\"USER: \"+t[:500])\n  except: pass\n print(\"\\n\".join(ctx[-6:]))\nexcept Exception as e:\n print(\"\")\n' 2>&1 | head -c 8000"
        out, _ = run_on_host(host, cmd)
        return (out or "").strip()[:limit_chars]
    elif cli_slug == "antigravity":
        cmd = f"cat {shlex.quote(path)} 2>/dev/null | tail -n 100 | head -c 8000"
        out, _ = run_on_host(host, cmd)
        return (out or "").strip()[:limit_chars]
    else:
        # Generic: use yggterm title helper's extract_tail_context via headless if available,
        # else fallback to tail of jsonl -> recent_context
        cmd = f"tail -n 96 {shlex.quote(path)} 2>/dev/null | head -c 8000"
        out, _ = run_on_host(host, cmd)
        # Try to parse as jsonl recent_context
        snippets = []
        for line in (out or "").splitlines()[-20:]:
            line=line.strip()
            if not line:
                continue
            try:
                j=json.loads(line)
                # Codex rollout has payload.type message role content
                p=j.get("payload",{{}} ) if isinstance(j.get("payload"),dict) else {{}} 
                if p.get("type")=="message":
                    role=p.get("role","")
                    content=p.get("content",[])
                    if isinstance(content, list):
                        txt=" ".join([c.get("text","") for c in content if isinstance(c,dict)][:1])
                        if txt.strip():
                            label="USER" if role=="user" else "ASSISTANT"
                            snippets.append(f"{label}: {txt[:300]}")
                # Claude cc has type human/assistant
                if j.get("type") in ("human","assistant"):
                    txt=j.get("message",{{}}).get("content","") if isinstance(j.get("message"),dict) else j.get("content","")
                    if isinstance(txt, list):
                        txt=" ".join([x.get("text","") for x in txt if isinstance(x,dict)][:1])
                    if isinstance(txt,str) and txt.strip():
                        snippets.append(f"{'USER' if j['type']=='human' else 'ASSISTANT'}: {txt[:300]}")
            except:
                continue
        if snippets:
            return "\n".join(snippets[-6:])[:limit_chars]
        return (out or "").strip()[:limit_chars]

def request_litellm_title(endpoint, api_key, model, context):
    import urllib.request
    import urllib.error
    url = endpoint.rstrip("/") + "/chat/completions"
    if url.endswith("/v1/chat/completions"):
        pass
    elif url.endswith("/v1"):
        url = url + "/chat/completions"
    elif not url.endswith("/chat/completions"):
        url = url.rstrip("/") + "/chat/completions"
    body = {
        "model": model,
        "temperature": 0.2,
        "max_tokens": 256,
        "messages": [
            {"role":"system","content":"Generate a short, high-signal tab title for a long-running coding or terminal session. Infer the real job from the overall objective, the latest concrete progress, and the strongest user intent. Prefer the larger effort over temporary substeps like screenshot reading, launch notes, status checks, quoted bad titles, or one-off UI pokes. Use a specific engineering noun phrase, 2 to 6 words, no quotes, no markdown, no trailing punctuation. Do not return a question, instruction fragment, or word salad. Never start with How, Why, What, When, Where, or Who. Never end with an article or preposition such as The, A, An, To, For, Of, With, or Into. Good: Yggterm Titlebar Fix, Daemon Lifecycle Leak Audit. Bad: How Use Skills Discovery The, Dev Sta, Fix Issue, Work Session."},
            {"role":"user","content": f"Create a concise session title from this structured session context.\nPrioritize: 1) the main user goal, 2) the active system/repo, and 3) the concrete engineering work happening now.\nIf the latest turns are screenshot inspection or modal polish inside a longer debugging effort, title the larger effort.\nUse a noun phrase that can sit on a sidebar row. Do not echo raw metadata, shell paths, existing sidebar labels, screenshot labels, quoted bad generated titles, question words, or cute placeholder labels.\nReturn the title only.\n\n{context}"}
        ]
    }
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            res_body = response.read().decode("utf-8")
            j = json.loads(res_body)
            choices = j.get("choices", [])
            if choices and isinstance(choices[0], dict):
                msg = choices[0].get("message", {}) or choices[0].get("delta", {})
                txt = msg.get("content")
                if txt and txt.strip():
                    title = txt.strip().strip('"').strip("'").strip()
                    title = re.sub(r"\s+", " ", title).strip(" .")
                    words = title.split()
                    if len(words) > 6:
                        title = " ".join(words[:6])
                    return title, None
            return None, f"no choices in response: {str(j)[:500]}"
    except urllib.error.HTTPError as e:
        if e.code == 429:
            return None, "rate limited 429 — retry next tick, do not persist heuristic"
        if e.code == 500 and model == "chatgpt/gpt-5.6-luna":
            fallback = "antigravity/gemini-3.7-flash"
            body["model"] = fallback
            data_fb = json.dumps(body).encode("utf-8")
            req_fb = urllib.request.Request(url, data=data_fb, headers=headers, method="POST")
            try:
                with urllib.request.urlopen(req_fb, timeout=30) as fb_resp:
                    res_body = fb_resp.read().decode("utf-8")
                    j = json.loads(res_body)
                    choices = j.get("choices", [])
                    if choices and isinstance(choices[0], dict):
                        msg = choices[0].get("message", {}) or choices[0].get("delta", {})
                        txt = msg.get("content")
                        if txt and txt.strip():
                            title = txt.strip().strip('"').strip("'").strip()
                            title = re.sub(r"\s+", " ", title).strip(" .")
                            words = title.split()
                            if len(words) > 6:
                                title = " ".join(words[:6])
                            return title, None
            except Exception as ex:
                return None, f"fallback error: {ex}"
        return None, f"HTTP error {e.code}: {e.reason}"
    except Exception as e:
        return None, str(e)

def audit_live_host(host, verbose=False):
    # Live sessions (snapshot) — these are what the GUI's live_rail and startpage live block show
    cmd = "~/.local/bin/yggterm-headless server snapshot 2>&1 || yggterm-headless server snapshot 2>&1 || yggterm server snapshot 2>&1"
    out, err = run_on_host(host, cmd)
    live_weird = []
    live_rows = []
    if not err:
        try:
            d = json.loads(out)
            live = d.get("data", d).get("live_sessions", d.get("live_sessions", []))
            live_rows = live
            for s in live:
                title = s.get("title") or s.get("session_title") or s.get("label") or ""
                kind = s.get("kind") or s.get("session_kind") or ""
                cwd = s.get("cwd") or ""
                reason = looks_like_weird_title(title, cwd)
                if reason:
                    live_weird.append({"session_id": s.get("session_id") or s.get("id","") , "kind": kind, "title": title, "cwd": cwd, "reason": reason, "session_path": s.get("session_path","")})
        except Exception as e:
            pass
    return live_weird, live_rows, err

def audit_host(host, verbose=False):
    start_data, err, raw = verb_on_host(host, "startpage")
    cw_data, cw_err, cw_raw = verb_on_host(host, "cwdtree")
    endpoint, api_key, model = get_litellm_config(host)
    live_weird, live_rows, live_err = audit_live_host(host, verbose=verbose)
    results = {
        "host": host,
        "endpoint": endpoint,
        "model": model,
        "has_api_key": bool(api_key),
        "startpage": {"error": err, "count": len(start_data.get("rows",[])) if start_data else 0, "raw": raw[:500] if err else ""},
        "cwdtree": {"error": cw_err, "count": 0, "raw": cw_raw[:500] if cw_err else ""},
        "live": {"error": live_err, "count": len(live_rows), "weird": len(live_weird)},
        "weird": [],
        "live_weird": live_weird,
        "ordering_issues": [],
    }
    if cw_data:
        # cwdtree ls returns groups; count sessions
        groups = cw_data.get("groups") or cw_data.get("rows") or []
        total = 0
        for g in groups:
            if isinstance(g, dict):
                total += g.get("descendant_sessions", 0) or len(g.get("sessions", [])) or 0
        results["cwdtree"]["count"] = total
        results["cwdtree"]["groups"] = len(groups)

    # Check ordering: rows should be descending modified_epoch_ms (or recency)
    if start_data and start_data.get("rows"):
        rows = start_data["rows"]
        epochs = [r.get("modified_epoch_ms", r.get("modified_epoch", 0)) for r in rows]
        # Detect constant-zero (the 2026-08-13 bug: all epochs 0 -> alphabetical uuid)
        if epochs and all(e == 0 for e in epochs[:10]):
            results["ordering_issues"].append("all modified_epoch_ms == 0 — sort falls through to session_id (alphabetical uuid, not recency) — see startpage.rs fix 2026-08-13")
        # Check descending
        for i in range(min(len(epochs)-1, 20)):
            if epochs[i] < epochs[i+1]:
                results["ordering_issues"].append(f"not descending at {i}: {epochs[i]} < {epochs[i+1]} (id {rows[i].get('session_id','')[:8]} vs {rows[i+1].get('session_id','')[:8]})")
                break
        # Flag weird titles
        for r in rows:
            title = r.get("effective_title") or r.get("title") or ""
            # Fallback to detail short id
            if not title:
                title = r.get("detail") or ""
            reason = looks_like_weird_title(title, r.get("cwd",""))
            if reason:
                results["weird"].append({
                    "session_id": r.get("session_id"),
                    "kind": r.get("kind"),
                    "title": title,
                    "cwd": r.get("cwd"),
                    "reason": reason,
                    "modified_epoch_ms": r.get("modified_epoch_ms"),
                    "storage_path": r.get("storage_path"),
                })
    if verbose and results["weird"]:
        print(f"[{host}] weird durable: {len(results['weird'])}")
        for w in results["weird"][:10]:
            print(f"  {w['kind']:14s} {w['session_id'][:8]} {w['reason']:30s} {w['title'][:40]!r} cwd={w['cwd'][:30]}")
    if verbose and results["live_weird"]:
        print(f"[{host}] weird live: {len(results['live_weird'])}")
        for w in results["live_weird"][:10]:
            print(f"  live {w['kind']:14s} {w['session_id'][:8]} {w['reason']:30s} {w['title'][:40]!r}")
    if verbose and results["ordering_issues"]:
        print(f"[{host}] ordering: {results['ordering_issues']}")
    if verbose:
        print(f"[{host}] summary: durable {results['startpage']['count']} cwdtree-groups {results['cwdtree'].get('groups',0)} live {results['live']['count']} (weird live {len(results['live_weird'])})")
    return results

def main():
    ap = argparse.ArgumentParser(description="Fleet title audit + LLM rescue")
    ap.add_argument("--host", action="append", dest="hosts", help="host to check (repeatable)")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--json", action="store_true", help="machine-readable report")
    ap.add_argument("--rescue", action="store_true", help="call LLM to rescue weird titles (dry-run unless --write)")
    ap.add_argument("--write", action="store_true", help="write rescued titles back via SessionTitleStore")
    ap.add_argument("--model", default=None, help="override model (default chatgpt/gpt-5.6-luna)")
    ap.add_argument("--limit", type=int, default=20, help="max weird to rescue per host")
    args = ap.parse_args()

    hosts = args.hosts if args.hosts else fleet_hosts()
    if not hosts:
        hosts = ["local"]

    reports = []
    all_ok = True
    for host in hosts:
        rep = audit_host(host, verbose=args.verbose or not args.json)
        reports.append(rep)
        if rep["weird"] or rep["ordering_issues"]:
            all_ok = False

    if args.rescue:
        for rep in reports:
            host = rep["host"]
            endpoint, api_key, model = get_litellm_config(host)
            if args.model:
                model = args.model
            # Rescue both durable and live weird — live is what the user sees in sidebar/cwdtree
            weird = (rep["weird"] + rep.get("live_weird", []))[:args.limit]
            if not weird:
                continue
            print(f"\n[{host}] rescuing {len(weird)} titles (durable {len(rep['weird'])} + live {len(rep.get('live_weird',[]))}) via {model} @ {endpoint} ...")
            for w in weird:
                sid = w["session_id"]
                cli_slug = w["kind"]
                # Map kind to slug for context extraction
                slug_map = {"muse":"muse","codex":"codex","codex-litellm":"codex-litellm","claude-code":"claude-code","pi":"pi","qwen":"qwen","antigravity":"antigravity","grok-build":"grok"}
                slug = slug_map.get(cli_slug, cli_slug)
                path = w.get("storage_path") or ""
                context = extract_tail_context(host, path, slug) if path else ""
                if not context or len(context.strip()) < 30:
                    # Fallback: use title+cwd as context
                    context = f"cwd: {w['cwd']}\ntitle: {w['title']}\nkind: {w['kind']}"
                print(f"  {sid[:8]} ({cli_slug}) context {len(context)} chars -> ", end="", flush=True)
                title, err = request_litellm_title(endpoint, api_key, model, context)
                if err:
                    print(f"ERR: {err}")
                    if "429" in err:
                        print("  (rate-limited — chore cap 3 per tick; stopping this host)")
                        break
                    continue
                print(f"LLM: {title!r}")
                if args.write and title:
                    # Write via yggterm headless title store on that host
                    # Use sqlite directly: ~/.yggterm/session-titles.db
                    cwd_esc = w["cwd"].replace("'", "''") if w["cwd"] else ""
                    title_esc = title.replace("'", "''")
                    cmd = f"sqlite3 ~/.yggterm/session-titles.db \"INSERT OR REPLACE INTO session_titles (session_id, title, cwd, source, model, updated_at) VALUES ('{sid}', '{title_esc}', '{cwd_esc}', 'litellm', '{model}', datetime('now')); SELECT changes();\" 2>&1 | head -n 5"
                    out, err2 = run_on_host(host, cmd)
                    print(f"    write: {out.strip()[:100] if out else err2}")
                w["rescued_title"] = title
                w["rescue_error"] = err

    if args.json:
        print(json.dumps(reports, indent=2))
    else:
        if not args.rescue:
            fleet_weird = sum(len(r["weird"]) for r in reports)
            fleet_order = sum(len(r["ordering_issues"]) for r in reports)
            print("\n" + "="*60)
            print(f"Fleet weird titles: {fleet_weird}  ordering issues: {fleet_order}")
            if fleet_weird and not args.rescue:
                print("Run with --rescue to batch to LLM (dry-run). Add --write to persist.")
            print("Title wiring per CLI: check weird 'reason' — shorthash/path/New session -> wiring bug; generated-fallback -> prompt/heuristic bug; empty -> no context extracted.")

    if not all_ok:
        sys.exit(2)
    sys.exit(0)

if __name__ == "__main__":
    main()
