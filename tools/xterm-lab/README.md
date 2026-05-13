# xterm.js Lab

This is a static, local-only lab for reproducing terminal renderer behavior
outside the Yggterm shell. It uses the vendored xterm.js assets from
`assets/xterm/`.

Run it from the repository root:

```bash
python3 -m http.server 8765
```

Then open:

```text
http://127.0.0.1:8765/tools/xterm-lab/
```

The useful fixtures are:

- `Plain prompt, default bg`: proves a prompt row with default background has no
  band.
- `Prompt with truecolor bg`: proves xterm.js paints the band when PTY bytes set
  a background attribute.
- `Jojo partial payload sample`: replays the observed clear-line payload shape
  that uses `49m`, so it cannot create a prompt band.
- `Resize partial repaint fixture`: shows how a partial bottom repaint after
  resize can leave stale status/footer ordering.
- `Full repaint after resize`: comparison case for a clean repaint with a
  truecolor prompt row.
