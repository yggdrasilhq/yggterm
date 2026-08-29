---
name: ytrace
description: The fleet's DTrace — runtime-attached script clauses (predicates + in-process aggregates) over live ytrace providers, plus the file-first query verbs. Use when an instrument must answer a question NOW (slow frames, stall rates, per-host latency, crime-scene capture) without perturbing the measured app, or when ytrace query/tail/incidents is the first-line tool.
---

# ytrace — the fleet probe bus + script plane

Providers (`yggterm`, `ytop`, `ychrome`, proprietary apps) emit spans/events/metrics/incidents to `$XDG_DATA_HOME/ytrace/<app>/ytrace.jsonl`. Since ytrace 0.2.0 every live provider also runs a **control socket** where you attach scripts that aggregate at the probe site. Spec SSOT: `~/gh/ytrace/docs/spec-ytrace.md` (§11 = script plane).

## Attach a script (the DTrace move)

```sh
# slow-frame histogram per host — durable until you detach it
ytrace attach --app yggterm 'render/gui where duration_ms > 16 -> @quantize(duration_ms) by payload.host_id'

# the µs-per-row slope — arithmetic inside aggregate arguments
ytrace attach --app yggterm 'render/gui -> @quantize(duration_ms / payload.rows * 1000)'

# crime-scene capture: last N matching records (byte-capped, truncation visible)
ytrace attach --app yggterm 'daemon_terminal_read where payload.pending_chars == 0 keep payload, duration_ms ring 32'

ytrace scripts --app yggterm                      # ids, clauses, counters
ytrace drain  --app yggterm <id> --watch 2        # live rate view
ytrace drain  --app yggterm <id> --reset --json   # machine-readable snapshot
ytrace detach --app yggterm <id>
```

Grammar (the whole language): `category/name [where expr] [-> @agg(expr), ...] [by path, ...] [keep path, ...] [ring N]` — aggs `count sum min max avg quantize`; bare paths are header fields (`duration_ms`, `component`, `category`, `name`, `clock`), payload fields are `payload.x.y`.

## The laws (why this instrument doesn't lie)

- **Scripts see every firing, unsampled.** Sampling is a file-stream policy; a quantize that saw 1:50 of fast frames would be a lying instrument.
- **Attach is durable.** Attach once, leave it — always-on p99 since boot costs nanoseconds per event.
- **Drains ride the socket, never the plane.** Aggregate snapshots do not shorten the JSONL diagnostic window (the 11.8 lesson).
- **`fired / matched / schema_miss` are three different findings.** `fired=0` = probe silent; `matched=0` with `fired>0` = predicate wrong, not system healthy; `schema_miss>0` = the record didn't look the way the script assumed. Never read `matched=0` as "no problem" without checking the other two.
- **Bounds are visible.** Group cap 1024 (+counted overflow bucket), ring ≤4096, captures >4 KiB truncate to a `{"_truncated":true}` marker.

## File-first verbs (always work, even for dead processes)

```sh
ytrace query --app yggterm --category render --since 15m --json   # ranked, clock-aware
ytrace tail --app yggterm --category render --lines 200           # ⛔ ALWAYS pass --lines (see below)
ytrace incidents --app yggterm --since 1h
ytrace registry        # live providers, pids, sockets, probe lists
```

## Instrument traps (each cost a session once — do not re-pay them)

- ⛔ **`tail --since` without `--lines` is capped at 20 records.** An hour window can return the last few seconds, well-formed. Always pass an explicit `--lines`.
- ⛔ **Never read the ytrace files directly.** `compat::resolve_home` prefers the yggterm home, so `~/.local/share/ytrace/<app>` can be a stale orphan — go through the CLI or the registry, which resolve the home the way the writer does.
- ⛔ **`duration_ms` is on two clocks.** `render/*` is cpu-ms, everything else wall latency — rank within one clock at a time.
- ⛔ **A freeze measured from inside the freeze reads zero.** In-process probes on a blocked UI thread cannot fire. For block/liveness questions use the out-of-process watchdog or `server monitor`, not an attached script on the thread you suspect.
- A script attached to a probe of a **dead process is gone** (socket + state die with the pid) — re-attach after restart; ids are stable.

## Non-goals

No loops/variables/user functions in clauses; no cross-probe joins (correlate in a Python sink script or a ytop Dash notebook); not uprobes — probes stay declared at compile time.
