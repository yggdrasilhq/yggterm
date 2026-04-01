# Preview Scroll Stability

## User-visible problem

Long rendered previews could blank, drift in width, leak scaffold content, or mutate later while the user scrolled through remote sessions.

## What changed

The preview pipeline and its harnesses were tightened so that:

- stale scaffold turns are filtered out
- interrupted-turn scaffolding no longer leaks into visible preview text
- preview hydration and scrolling stay stable under repeated remote session churn
- daemon memory stays bounded during the heavy preview-scroll workload

## Verification

Primary artifact:

- `/tmp/yggterm-preview-ui-scroll-23-after-normalizer-full3/summary.json`

Verified full-count result:

- `count=23`
- `scroll_checks_per_session=23`
- `open_failures=0`
- `blank_failures=0`
- `width_failures=0`
- `semantic_failures=0`
- `markup_failures=0`
- `daemon_rss_failures=0`
