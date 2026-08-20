# Profiling notebooks

Executable analyses over the `ytrace` probe bus. Each one reads live fleet data
through the `ytrace` verbs, and **ends in a verdict cell** — thresholds, a
red/amber/green mark per finding, and the number that produced it — so the
reading is a conclusion rather than a plot to interpret.

The probe map they stand on is [`../docs/observability.md`](../docs/observability.md).

| notebook | asks |
|---|---|
| `01-input-latency` | how long a keypress takes to reach the PTY and come back, per session kind |
| `02-render-storms` | is the GUI rendering faster than anything upstream changed |
| `03-attach-storms` | what a GUI restart costs the sessions on it |
| `04-title-and-scan-churn` | what the background chores cost, and how much is re-work |
| `05-fleet-heat` | why the client machine gets hot when the work is elsewhere |
| `06-ui-blocks` | what was running when the interface stopped |

## Running them

```sh
./run.sh                 # all, 30m window, this machine
./run.sh 02 05           # just those
YGG_NOTEBOOK_HOSTS=alpha,beta YGG_NOTEBOOK_GUI_HOST=beta ./run.sh
YGG_NOTEBOOK_WINDOW=2h ./run.sh
```

`run.sh` bootstraps its own venv on first use and prints each notebook's output
to the terminal, verdict last. Executed copies land in `out/` (gitignored).

**Hosts are configuration, never literals in a tracked file.** `local` means this
machine; anything else is an ssh alias. `YGG_NOTEBOOK_GUI_HOST` matters because
the render, input, attach and ui-block probes only exist where a GUI runs — the
others will find nothing on a headless host, which is not the same as finding
nothing wrong.

## Why standard library only

No pandas, no numpy, no matplotlib. These run over ssh against a machine whose
first resource priority is memory, launched from a shell on whichever host is
free, and a notebook that needs a scientific stack installed cannot be executed
from a script on a headless box. Percentiles come from `statistics`, timelines
are unicode sparklines, and the output is text that an agent or an LLM can read
without rendering anything.

## Three rules the helpers enforce

1. **Never hand-resolve a ytrace home.** `ytrace`'s own resolution prefers the
   yggterm home when it exists, so the XDG path is usually a stale orphan full of
   well-formed, parseable, day-old records. Everything goes through the CLI.
2. **Never compare two clocks.** `duration_ms` is wall for every category except
   `render`, where it is CPU-ms consumed over `interval_ms`. Summing the two into
   one ranking produces a number with no unit.
3. **`INSUFFICIENT DATA` never collapses into `PASS`.** A probe that never fired
   and a probe that fired healthily produce the same empty result set. Calling
   that green is how an instrument gap becomes an all-clear — which is exactly
   how a GUI freeze came to be recorded as zero incidents.
