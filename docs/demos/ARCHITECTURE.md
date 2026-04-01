# Demo And Changelog Architecture

This directory defines the evidence pipeline for `yggterm` and future `yggui` apps.

The goal is simple: every meaningful shipped feature should be able to produce:

- a deterministic reproduction path
- screenshots and optional recordings
- trace and app-state evidence
- a concise narrative summary
- a curated changelog entry backed by those artifacts

## Product Intent

The long-term target is a shared `yggui` platform for:

- desktop app-control
- UI observability
- macro automation
- proof bundle generation
- demo composition
- release-page publishing

`yggterm` is the first consumer, not the final shape.

## Layers

The pipeline should be thought of in four layers.

### 1. Capture

Runtime capture comes from:

- `server app ...` commands
- event traces
- perf telemetry
- deterministic macro scripts
- screenshots
- recordings

This layer is about raw evidence, not storytelling.

### 2. Proof Bundles

A proof bundle is the durable artifact for one feature or fix. Each bundle should contain:

- `manifest.json`
- `summary.md`
- `captures/`
- `trace/`

This is the bridge between QA, release notes, and marketing.

### 3. Curated Narrative

`CHANGELOG.md` stays human-readable. It should reference proof bundles rather than embedding all raw evidence directly.

Release pages should be assembled from:

- curated changelog text
- selected proof bundle summaries
- screenshots and short recordings

### 4. Demo Composition

Demo composition should feel deliberate and legible:

- deterministic scene scripts
- clean framing
- restrained highlights and overlays
- motion used to reveal structure, not to decorate

The aesthetic target is evidence-first product storytelling: closer to careful technical explanation than flashy promo montage.

## CI And Publishing

The intended release path is:

1. build binaries and packages
2. run selected proof manifests
3. collect screenshots, recordings, traces, and summaries
4. validate changelog references
5. publish release artifacts and curated release notes
6. publish selected assets to the ecosystem website

GitHub Actions should handle the CI-safe subset. A self-hosted GUI runner should handle the full desktop capture path.

## Reuse Across Apps

Future apps such as `yggtopo` or `cellulose` should be able to reuse:

- the manifest schema
- the proof bundle folder layout
- the app-control concepts
- the changelog/storytelling workflow

Only app-specific selectors, assertions, and feature manifests should need to change.
