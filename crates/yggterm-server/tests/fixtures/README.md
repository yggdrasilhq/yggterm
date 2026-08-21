# Screen fixtures

Captured pty screens, kept as raw bytes because **how a screen is drawn is half
the evidence**. Stripping the escapes for legibility would delete the very
property these fixtures exist to pin.

`startup-gate-screen.bin` — a first-run workspace-trust gate, captured
2026-08-21 from a row spawned into a directory its CLI had not opened before.
Nine visible rows, painted with eleven absolute cursor moves and no newlines
between them, with single spaces emitted as cursor-forward. Any identifying
path in the original was replaced with an invented one; nothing else was
touched, because the drawing grammar IS the fixture.
