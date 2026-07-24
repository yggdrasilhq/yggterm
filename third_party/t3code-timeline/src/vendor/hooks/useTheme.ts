// LOCAL REPLACEMENT for T3 Code's apps/web/src/hooks/useTheme.ts
// (see ../../NOTICE.md §2).
//
// Upstream this hook OWNS the theme: it reads `localStorage["t3code:theme"]`,
// falls back to `prefers-color-scheme`, toggles the `dark` class on
// `document.documentElement` at MODULE LOAD, and re-applies it from an effect
// whenever a consumer mounts. That is right for their app, which has a theme
// switcher. It is wrong here: yggterm owns the theme (DESIGN.md), and the class
// has already been set by `mount()` before this module could run.
//
// This cost a real debugging round, so the mechanism is worth recording. The
// symptom was that a transcript containing MARKDOWN rendered light while a
// plain-text one rendered dark. `ChatMarkdown` is the only consumer of this
// hook, so its mount effect — resetting the class to the "system" default,
// light under a headless browser — only fired when a markdown message existed.
// Probing the DOM looked fine (`class="dark"`) because in those runs the
// virtualizer had rendered nothing yet, so ChatMarkdown never mounted. The
// computed style and the pixel genuinely disagreed until the hook ran.
//
// Now the theme is pushed in by the embedder and nothing here touches the
// document. `setTheme` is kept because upstream's signature has it, but it is
// inert: a component inside the transcript must not restyle the app around it.

import { useSyncExternalStore } from "react";

type Theme = "light" | "dark" | "system";

let current: "light" | "dark" = "dark";
let listeners: Array<() => void> = [];

/// Called by `mount()`. Notifies subscribers so a live theme change re-renders
/// the markdown, whose code highlighting picks a light or dark palette.
export function setEmbeddedTheme(theme: "light" | "dark"): void {
  if (current === theme) return;
  current = theme;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners = [...listeners, listener];
  return () => {
    listeners = listeners.filter((entry) => entry !== listener);
  };
}

function getSnapshot(): "light" | "dark" {
  return current;
}

export function useTheme() {
  const resolvedTheme = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return {
    theme: resolvedTheme as Theme,
    setTheme: (_next: Theme) => {},
    resolvedTheme,
  } as const;
}
