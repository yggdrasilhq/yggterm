# Proof Bundle Format

Each feature or fix should be able to map to one proof bundle.

## Bundle Location

During active development:

- `artifacts/demos/unreleased/<feature-id>/`

At release cut time, selected bundles can be promoted or copied into a release-specific location outside this repo or into a website ingestion pipeline.

## Required Files

Each bundle should contain:

- `manifest.json`
- `summary.md`
- `captures/`
- `trace/`

Recommended structure:

```text
artifacts/demos/unreleased/<feature-id>/
  manifest.json
  summary.md
  captures/
    before.png
    after.png
    demo.mp4
  trace/
    app-state.json
    event-trace.jsonl
    perf-telemetry.jsonl
```

## Manifest Fields

The manifest should be machine-readable and stable enough for CI. Recommended fields:

```json
{
  "app_id": "yggterm",
  "feature_id": "preview-scroll-stability",
  "title": "Preview scroll stays stable under long remote sessions",
  "status": "unreleased",
  "category": "preview",
  "user_value": "Scrolling long remote previews no longer blanks or shifts width.",
  "macro": {
    "script": "scripts/ui_preview_ui_scroll_23.py",
    "mode": "launch-local"
  },
  "proof": {
    "screenshots": true,
    "recording": false,
    "trace": true
  },
  "changelog": {
    "section": "Fixed",
    "summary": "keep long remote preview scrolling stable without blanking or width shifts"
  }
}
```

## Summary Contract

`summary.md` should answer:

- what problem the user would have noticed
- what changed
- how it was verified
- where the evidence lives

Keep it concise and visual.

## Commit Policy

Small manifests, summaries, and selected screenshots can live in git. Heavy recordings and transient capture outputs may be published externally or attached in CI artifacts rather than committed by default.
