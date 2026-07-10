# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## 1. Working-dot lag: sidebar dot keeps blinking 10–45 s after the agent finished

- **Symptom:** the session's working/blinking dot in the sidebar keeps
  animating for ~10–45 s after Claude Code has finished, even though the
  "finished working" notification already fired at the right time.
- **Hypothesis:** the GUI's working-flag copy only updates via the background
  live-session refresh, which
  `focused_terminal_should_defer_background_refreshes` plus the 15 s
  interactive defer delays; the toast rides a different/earlier apply.
- **Measurement trap:** app-control probes CANNOT observe this — every probe
  forces a refresh and masks the lag (proven with a dot-lag monitor script;
  observer effect). It needs in-process timing instrumentation: a trace event
  on the working-flag edge in `notify_finished_working_sessions` vs. the
  sidebar render that visually clears the dot. The `agent_session_error` /
  event-trace machinery from 2.10.2 is a good pattern to copy.

## 2. Local machine row shows no working dot when collapsed

- **Symptom:** when the LOCAL machine group in the sidebar tree is collapsed,
  the collapsed row does not show the blinking working/traffic dot beside it
  while a session under it is working. SSH machine rows DO show the dot when
  collapsed. Expanded local rows are fine.
- **Likely shape:** the collapsed-row working indicator aggregates child
  session working state per machine; the local machine row probably takes a
  different code path (it is not a "remote machine" row) and never got the
  aggregation. Find where the ssh machine row derives its collapsed dot and
  mirror it for the local root.

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
