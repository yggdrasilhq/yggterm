# yggterm

Yggdrasil Terminal (`yggterm`) is a Rust-first terminal workspace designed for operators managing many long-running services and containers.

## Why it exists

When your day is split across many LXC hosts, maintenance shells, and incident windows, a plain tab list stops scaling.
`yggterm` is designed around session trees and operator workflows, not around single-command demos.

## Scope

- terminal session tree model (`core`)
- UI/workspace shell (`ui`)
- platform integration (`platform`)
- optional Ghostty bridge (`ghostty-bridge`)

## Build

```bash
cargo build --release
```

Binary:

- `target/release/yggterm`

## Docs

Documentation is centralized in `yggdocs` under ecosystem `quickstart/wiki/dev` sections.

## License

Apache-2.0
