# mock-tui (OpenTUI)

The deterministic agent-CLI TUI yggterm can interrogate. **Spec + witness
recipes: [`docs/spec-mock-tui-opentui.md`](../../docs/spec-mock-tui-opentui.md)**
— read that before adding a scenario; a scenario without a witness recipe is
a demo, not a test.

```sh
node src/mock-tui.js --list
node src/mock-tui.js --scenario bg-fill            # raw engine (zero deps)
node src/mock-tui.js --scenario bg-fill --engine opentui   # needs @opentui/core
node src/mock-tui.js --scenario bg-fill --hold-ms 8000     # for screenshots
```

Scripted stimuli over one PTY (stdin): `:fill blue`, `:title OC | driven`,
`:alt-enter`, `:alt-exit`, `q` quits with `MOCKTUI <scenario> ok`.

Law: the mock is the STIMULUS, yggterm is the WITNESS — it emits nothing
into the ytrace plane; every scenario's claim is falsified through
yggterm-side probes (`mouse_mode_probe`, `frame_hash_probe`, `resize*`,
`pty_in_alternate_screen`, the `cli/*` chain).
