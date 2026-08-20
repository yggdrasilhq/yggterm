# Spec: `ytop` — Modern Fleet Infrastructure & Agent Cockpit

**Status:** PROPOSED (2026-08-15)  
**Target Repositories:** `ytop` (`~/gh/ytop`), `yggterm` (`~/gh/yggterm`), `libyggterm` (`~/gh/libyggterm`)

---

## 1. Vision & Purpose

`ytop` replaces unstyled, terminal-dump telemetry with a **modern, rich, colorful desktop-class monitoring and operations console** inside Yggterm's `libyggterm` document-surface architecture.

It provides a unified, real-time control plane across two fundamental operational modes:
1. **`Top` (Infrastructure & Host Topology)**: Physical & virtual machines, LXC containers with collapsible process consumption trees, ZFS storage health & real-time `zpool iostat`, and persistent multi-host management.
2. **`Dash` (Yggterm Agent Fleet Cockpit)**: Complete $N.x$ agent row census, real-time per-row CPU/Memory footprint, token context budget gauges, "resource jankbox" bottleneck profiling, and live supervision controls (Booter/Monitor/Quota Holds).

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐
│  Y T O P   ·   [ ⚡ TOP ]   [ 📊 DASH ]                             [ 🔍 Filter ] [ 🔄 2s ] │
├──────────────────────────┬──────────────────────────────────────────────────────────────────┤
│ MACHINES & FLEET         │  ⚡ TOP VIEW: alpha (Local Host)                              │
│                          │                                                                  │
│ ● alpha (local)       │  ┌─ SYSTEM METRICS ────────────────────────────────────────────┐ │
│   37% CPU · 24.1/62.7 GB │  │ CPU: [████████████░░░░░░░░░░░░] 37.2% · 32 Cores · Load 9.78 │ │
│                          │  │ RAM: [████████████████░░░░░░░░] 320.9 / 503.6 GB (63.7%)    │ │
│ ● beta (ssh beta)          │  │ SWP: [░░░░░░░░░░░░░░░░░░░░░░░░] 0.0 MB / 0.0 MB              │ │
│   12% CPU · 8.4/32.0 GB  │  └─────────────────────────────────────────────────────────────┘ │
│                          │                                                                  │
│ ● delta (ssh delta)        │  ┌─ ZFS STORAGE & IOSTAT (Main Storage Pool) ──────────────────┐ │
│   5% CPU · 4.2/16.0 GB   │  │ zpool: rpool [ONLINE] · 1.8 TB / 3.6 TB Allocated (50%)     │ │
│                          │  │ Read:  1.2 MB/s (142 IOPS)  ·  Write: 8.4 MB/s (820 IOPS)   │ │
│ ● gamma (ssh gamma)        │  └─────────────────────────────────────────────────────────────┘ │
│   45% CPU · 94.2/128 GB  │                                                                  │
│                          │  ┌─ LXC CONTAINERS (with expandable process consumption) ──────┐ │
│ [ + Add SSH Machine ]    │  │ ▼ 📦 ct-alpha [RUNNING]  ·  24.5% CPU  ·  1.8 GB RAM     │ │
│                          │  │   ├─ PID 1235385  agy            24.7% CPU   336 MB RSS     │ │
│                          │  │   ├─ PID 1109226  claude          2.5% CPU   612 MB RSS     │ │
│                          │  │   └─ PID 3970214  claude          2.5% CPU   564 MB RSS     │ │
│                          │  │ ▶ 📦 ct-builder  [RUNNING]  ·   0.2% CPU  ·  256 MB RAM     │ │
│                          │  │ ▶ 📦 ct-postgres [RUNNING]  ·   1.1% CPU  ·  4.2 GB RAM     │ │
│                          │  └─────────────────────────────────────────────────────────────┘ │
└──────────────────────────┴──────────────────────────────────────────────────────────────────┘
```

---

## 2. Core UI Philosophy & Aesthetics (DESIGN.md Alignment)

1. **No Raw Markdown Tables in the Main Viewport**:
   - The viewport is rendered as **native shell DOM widgets** (`card`, `section`, `meter` / proportion bars, metric grids, status badges, interactive list rows, collapsible container nodes, process breakdown tables).
   - Inherits Yggterm’s background gradient, soft shadows, rounded surfaces, and crisp typography.
2. **Color-Coded Status & Health Tokens**:
   - **Emerald / Green (`durable`)**: Healthy processes, online ZFS pools, active normal agents.
   - **Sky Blue (`transient`)**: Running LXC containers, temporary tasks.
   - **Amber (`warning`)**: Context size bloat (>10MB), high CPU usage (>80%), degraded storage, parked agents.
   - **Rose (`danger`)**: Critical context size (>30MB), twin duplicate processes, spinning leaked subshell loops, dead processes.
   - **Indigo / Violet (`supervision`)**: Orchestrator seats, quota holds, rate limit states.
3. **Fluid Responsiveness**:
   - Multi-column card layout on wide desktop viewports; cleanly collapsing to single-column on narrower panes or sidebar rails.

---

## 3. Top-Level Mode Architecture

The application titlebar / header hosts the primary mode toggle:
- `[ ⚡ Top ]`: Infrastructure, machines, ZFS, LXC containers, host processes.
- `[ 📊 Dash ]`: Yggterm agent fleet rows, resource jankbox profiling, supervision & watchdog controls.

---

## 4. `Top` View (Infrastructure & Host Topology)

### 4.1. Connected Machines Sidebar & Persistent Machine Registry
- **Sources of Machines**:
  1. Local machine (`local` / `alpha`).
  2. Auto-discovered remote SSH sessions from live Yggterm daemon snapshots (`server daemons`).
  3. Stored user configurations from `~/.yggterm/config/machines.json`.
- **`[ + Add SSH Machine ]` Action**:
  - Modal or inline card prompting for `SSH Alias / Host`, `Label`, and optional `Tags` (e.g. `is_yggdrasil_host: true`).
  - Persisted in structured JSON format under `~/.yggterm/config/machines.json`.
- **Machine Row**:
  - Host label, status dot (green=responsive, red=unreachable), instant CPU% and RAM% chips.

### 4.2. Host System Metrics Card
- Hostname, CPU Model, Physical/Logical Cores, Kernel, Uptime.
- **Visual Progress Gauges**:
  - **CPU Utilization**: `[██████░░░░] 37%` (real-time delta sampled over 400ms).
  - **Memory Utilization**: `[████████░░] 320.9 GB / 503.6 GB (63%)`.
  - **Swap Space**: `[░░░░░░░░░░] 0 MB / 0 MB`.
  - **Load Averages**: 1m, 5m, 15m indicators.

### 4.3. Yggdrasil Host Special Features (e.g. `main` / server)
When a machine is designated as a Yggdrasil node (or has ZFS / LXC installed):
1. **ZFS Storage & IOSTAT Card**:
   - Pools overview: Pool name, health state (`ONLINE`, `DEGRADED`, `FAULTED`), capacity, allocation, fragmentation %.
   - **Real-Time `zpool iostat`**:
     - Read IOPS & Read Bandwidth (MB/s).
     - Write IOPS & Write Bandwidth (MB/s).
   - Dataset list with mountpoint and space usage (`zfs list -Hp`).
2. **LXC Container Consumptions & Expandable Process Drill-Down**:
   - Container summary table: Name, Status (`RUNNING`, `STOPPED`), Memory RAM, CPU%.
   - **Collapsible Disclosure Tree**:
     - Each container row can be uncollapsed (`expanded: true/false`).
     - When expanded, shows the top CPU and Memory processes running **inside that specific LXC container** (PID, user, CPU%, RSS MB, process name, command).
     - Enables immediate isolation of which container or process is consuming host capacity.

### 4.4. Host Top Processes Card
- Top 10-20 processes across the entire machine.
- Sortable by CPU% or Memory RSS.
- Interactive search/filter box.

---

## 5. `Dash` View (Yggterm Agent Fleet Cockpit)

### 5.1. Agent Fleet Rows Table Card
- Grouped by Campaign / Outline Prefix:
  - `6.x (widgets: refactor)`
  - `2.x (graph-demo)`
  - `3.x (finance-demo)`
  - `9.x (social-demo)`
  - `7.x (practice)`
  - `Unregistered / Custom Sessions`
- **For Each Row**:
  - **Seat & Role Badge**: `2.0 [orchestrator]`, `6.6 [relay]`.
  - **Title / Intent Label**: Clear task synopsis.
  - **UUID**: 8-char display with full copy affordance.
  - **Process Liveness**:
    - `LIVE [PIDs]`
    - `⛔ TWIN DUPLICATE [PIDs]` (Alert!)
    - `⚠️ LEAKS [N child loops]` (Alert!)
    - `💀 DEAD / RETIRED`
  - **Resource Consumption**: Real-time CPU% and RAM MB of the agent CLI and its child subshells.
  - **Context Budget & Transcript**:
    - Size in KB/MB.
    - Line count & last active timestamp.
    - Warning chips: `Normal (<5MB)`, `⚠️ HEAVY (>10MB)`, `🚨 CRITICAL (>30MB)`.
  - **Supervision State**: `⏸ Quota Held`, `🛡️ Never-Arm`, `⏸ Parked`, `⚡ Armed (Booter/Monitor)`, `Unsupervised`.

### 5.2. Resource "Jankbox" Profiling & Diagnostics Card
Dedicated diagnostic engine to explain and eliminate host lag:
- **Spinning Subshell Leaks**: Detects orphaned `until ... sleep` test loops or dangling bash child processes.
- **Twin Duplicate Instances**: Identifies dual `claude` / `codex` processes attached to identical session IDs from daemon bumps.
- **Bloated Cold Transcripts**: Flags inactive rows with massive transcripts that would waste tokens if resumed.
- **Daemon Footprint**: Measures PTY buffer memory and server daemon consumption.
- **One-Click Action**: `[ 🧹 Clean Leaks & Stale Twins ]` button to instantly reap safe orphaned processes without disturbing active agents.

### 5.3. Supervision & Watchdog Control Card
- Live status of Booter and Monitor watchdogs across the fleet.
- **Quota Hold Banner**: Shows active rate limit countdown with `[ ⏸ Set Quota Hold ]` and `[ ▶ Release Hold ]` triggers.
- **Arm / Disarm Toggles**: Per-row or fleet-wide booter and monitor subscription controls.
- **Never-Arm Ledger Editor**: View and toggle rows that must never be auto-woken.

---

## 6. Widget Schema & YggUI Component Contract

`ytop` declares widgets following the `libyggterm` Tier A/C specification.

### 6.1. Reusable Widget Vocabulary
| Widget Kind | Role in `ytop` |
|---|---|
| `section` | Card container with title, optional `card: true`, and trailing action buttons (e.g. `+ Add Machine`). |
| `tabs` | Top-level mode switch (`Top` <-> `Dash`) and machine selectors. |
| `list-row` | Collapsible tree rows (`depth`, `expanded`, `expand_action`) for LXC container process trees and agent rows. |
| `meter` / `progress` | Visual proportion bars for CPU %, Memory GB, Swap %, and ZFS pool capacity. |
| `search-box` | Live filtering of processes, containers, and agent rows. |
| `button` | Action triggers (`Refresh`, `Clean Leaks`, `Add Machine`, `Arm/Disarm`, `Hold`). |
| `toggle` | Supervision flags and auto-refresh intervals. |

---

## 7. Configuration & State Storage

- **Machine Registry**: `~/.yggterm/config/machines.json`
  ```json
  {
    "machines": [
      {
        "alias": "beta",
        "label": "beta (Backend Compute)",
        "is_yggdrasil": true
      },
      {
        "alias": "gamma",
        "label": "gamma (Server & Storage)",
        "is_yggdrasil": true
      }
    ]
  }
  ```
- **Never-Arm Ledger**: `~/.yggterm/relay/never-arm.tsv`
- **Quota Hold File**: `~/.yggterm/relay/booter.rate-limit-hold`
- **Booter State**: `~/.yggterm/relay/booter/<uuid>.json`
- **Monitor State**: `~/.yggterm/relay/monitor/<uuid>.json`

---

## 8. Implementation Plan

1. **Phase 1: Probing & Backend Extensions (`ytop::probe`, `ytop::rows`, `ytop::fleet`)**:
   - Add ZFS pool status and `zpool iostat` metrics collection to host probe.
   - Add LXC container process hierarchy extraction (cgroup-aware per-container process tree).
   - Add persistent `~/.yggterm/config/machines.json` loader and writer.
   - Add agent row resource footprint summation (CPU% and RSS RAM per seat).
2. **Phase 2: Modern Card & Gauge Schema (`ytop::schema`)**:
   - Implement the `Top` mode layout with System Gauges, ZFS Storage Card, and Collapsible LXC Container trees.
   - Implement the `Dash` mode layout with Agent Fleet Rows, Resource Jankbox Profiling, and Supervision Controls.
   - Eliminate plain unstyled markdown dumps in viewport; render native cards and structured rows.
3. **Phase 3: Interactive Server & Actions (`ytop::server`, `ytop::osc`)**:
   - Handle tab switching (`top` <-> `dash`), machine selection, container expansion toggles, and `+ Add Machine` form submissions.
   - Handle `Clean Leaks & Stale Twins` and supervision actions.
4. **Phase 4: CLI & Automation Verification**:
   - Support `ytop --once --tab top`, `ytop --once --tab dash`, and `--json`.
   - Validate on live GUI with `yggterm` and test suites.
