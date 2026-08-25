# Dream: ytrace Super-Observability & Agent Diagnostic Fabric

**Date:** 2026-08-25 · **Status:** IMPLEMENTED · **Authority:** `ytrace` + `yggterm-core::perf` + `notebooks/ytrace_helpers.py`

---

## 1. The Vision

To make `ytrace` the fastest, most ergonomic, and most powerful application-layer observability system in existence:
1. **Application-First Domain Semantics**: Probes record meaningful domain entities (`session_path`, `reads_since`, `component_window`, `clock: wall|cpu`), bridging what kernel-level eBPF cannot know.
2. **Sub-Millisecond Emission & Held Buffers**: Zero-allocation hot paths, held file handles, and generational byte-capped rotation to prevent telemetry from perturbing the measured application.
3. **Frictionless Agent-Human Scratch Analytics**: Instant queryability in Python scratch scripts, interactive notebooks, terminal `top` tables, folded-stack flamegraphs, and time-series buckets.
4. **Mechanical Anti-False-Zero Protection**: Queries fail fast or distinguish "no activity" from "missing schema field", preventing false negatives from misleading autonomous agents.

---

## 2. Core Capabilities Implemented

### 1. `ytrace top` — Interactive & Tabular Probe Ranking
Displays a live, ranked breakdown of all probes across the fleet, categorized by wall time, CPU time, invocation count, and p50/p95/max latency percentiles:
```bash
ytrace top --since 60m --top 15
```

### 2. `ytrace flame` — Application-Layer Latency Flamegraphs
Generates folded-stack traces (`app;component;category;name <duration_or_count>`) directly compatible with FlameGraph, speedscope, and terminal visualizers:
```bash
ytrace flame --since 60m | head -n 20
```

### 3. `ytrace timeseries` — Trend & Incident Bucketing
Aggregates telemetry into configurable time windows (e.g. `5s`, `1m`, `5m`), highlighting incident spikes and latency shifts:
```bash
ytrace timeseries --bucket 5m --since 2h
```

### 4. `ytop` Native Notebook Engine & Multi-Modal Visual Widgets
A libyggterm document-surface notebook system (`src/notebook.rs`) rendering directly into Yggterm's viewport:
- Bookshelf on the sidebar rail (`mode: "dash"` exclusively for ytrace adventures; `mode: "top"` for host atlas).
- Native widget dispatch: `chart: "table"` (top probe rankings), `chart: "flamegraph"` (folded-stack latency flamegraphs), and `chart: "timeseries"` (trend and incident rollup).
- Direct persistence to `~/.local/share/ytop/notebooks/<id>.json`.

---

## 3. Standardized Diagnostic Dash Notebooks

Three automated diagnostic notebooks are maintained in `~/.local/share/ytop/notebooks/`:
- **`dash-ui-latency-blocks.json`**: Analyzes UI thread responsiveness, ranks stalls exceeding the 200 ms budget, and displays latency flamegraphs.
- **`dash-web-surface-liveness.json`**: Investigates OSC 7717 heartbeats, touch intervals, and background listening clock transitions (`reads_since`).
- **`dash-fleet-health-spikes.json`**: Tracks rolling time-series trends, incident spikes, and health rollups across the fleet.

Agents and humans inspect and interact with these notebooks via:
```bash
ytop --mode dash
```
or via the ytop bookshelf on the sidebar rail.
