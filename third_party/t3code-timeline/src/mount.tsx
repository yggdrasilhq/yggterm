// yggterm's entry point into the vendored T3 Code timeline.
//
// This file is OURS (not vendored): it is the thin host that supplies the
// props `MessagesTimeline` expects and owns the state upstream's `ChatView`
// used to own — scroll container, expanded work groups, image preview. Keeping
// it separate from `src/vendor/` is what lets a future re-sync overwrite the
// vendor tree wholesale.
//
// The git-shaped props are deliberately inert. yggterm does not manage
// branches, worktrees or checkpoints for a session — the agent CLI owns its own
// working tree — so turn diffs and checkpoint revert receive no-ops and empty
// maps rather than being deleted from upstream's code.

import { StrictMode, useCallback, useMemo, useState } from "react";
import { createRoot, type Root } from "react-dom/client";

import { MessagesTimeline } from "./vendor/components/chat/MessagesTimeline";
import type { ExpandedImagePreview as ExpandedImagePreviewValue } from "./vendor/components/chat/ExpandedImagePreview";
import { deriveTimelineEntries } from "./vendor/session-logic";
import { installNativeApi, type NativeApi } from "./vendor/nativeApi";
import { setEmbeddedTheme } from "./vendor/hooks/useTheme";
import type { ChatMessage } from "./vendor/types";
import type { MessageId, TurnId } from "./vendor/contracts";
import "./theme.css";

/// What yggterm hands the renderer. Deliberately close to `ChatMessage` so the
/// Rust adapter has one obvious target — see `docs/spec-rendered-transcript.md`.
export interface TranscriptMessage {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  /// ISO-8601. The timeline sorts on this and shows it, so an adapter that
  /// cannot date a record should omit it rather than invent `now`.
  createdAt: string;
  completedAt?: string;
  /// True only for a message the agent is still writing.
  streaming?: boolean;
}

export interface MountOptions {
  messages: TranscriptMessage[];
  /// yggterm owns the theme (DESIGN.md); upstream read it from their store.
  theme?: "light" | "dark";
  /// Absolute cwd, so relative file links in markdown resolve.
  cwd?: string;
  /// Whether the agent is mid-turn — drives the working indicator.
  working?: boolean;
  /// Optional: lets a file link open in the user's editor. Absent ⇒ upstream's
  /// own "no native API" branch runs and the link is inert.
  nativeApi?: NativeApi;
}

function toChatMessage(message: TranscriptMessage): ChatMessage {
  return {
    id: message.id as MessageId,
    role: message.role,
    text: message.text,
    createdAt: message.createdAt,
    completedAt: message.completedAt,
    streaming: message.streaming ?? false,
  };
}

function Timeline({ messages, theme, cwd, working }: MountOptions) {
  const [scrollContainer, setScrollContainer] = useState<HTMLDivElement | null>(null);
  const [expandedWorkGroups, setExpandedWorkGroups] = useState<Record<string, boolean>>({});
  const [imagePreview, setImagePreview] = useState<ExpandedImagePreviewValue | null>(null);

  const timelineEntries = useMemo(
    // No proposed plans or work-log entries: those are produced by upstream's
    // orchestrator, which yggterm does not run. The timeline handles empty
    // arrays as its normal case.
    () => deriveTimelineEntries(messages.map(toChatMessage), [], []),
    [messages],
  );

  const onToggleWorkGroup = useCallback((groupId: string) => {
    setExpandedWorkGroups((current) => ({ ...current, [groupId]: !current[groupId] }));
  }, []);
  const noop = useCallback(() => {}, []);

  return (
    <div
      ref={setScrollContainer}
      className="yggterm-transcript-scroll"
      data-yggterm-transcript="1"
    >
      <MessagesTimeline
        hasMessages={messages.length > 0}
        isWorking={working ?? false}
        activeTurnInProgress={working ?? false}
        activeTurnStartedAt={null}
        scrollContainer={scrollContainer}
        timelineEntries={timelineEntries}
        completionDividerBeforeEntryId={null}
        completionSummary={null}
        // Git-shaped, therefore empty here: see the header note.
        turnDiffSummaryByAssistantMessageId={new Map()}
        nowIso={new Date().toISOString()}
        expandedWorkGroups={expandedWorkGroups}
        onToggleWorkGroup={onToggleWorkGroup}
        onOpenTurnDiff={noop as (turnId: TurnId, filePath?: string) => void}
        revertTurnCountByUserMessageId={new Map()}
        onRevertUserMessage={noop as (messageId: MessageId) => void}
        isRevertingCheckpoint={false}
        onImageExpand={setImagePreview}
        markdownCwd={cwd}
        resolvedTheme={theme ?? "dark"}
        timestampFormat="time"
        workspaceRoot={cwd}
      />
      {imagePreview ? (
        <button
          type="button"
          className="yggterm-transcript-image-backdrop"
          onClick={() => setImagePreview(null)}
        >
          <img src={imagePreview.src} alt={imagePreview.alt ?? ""} />
        </button>
      ) : null}
    </div>
  );
}

let root: Root | null = null;

/// Render (or re-render) the transcript into `element`.
///
/// Idempotent: calling it again with new messages updates in place rather than
/// remounting, so the scroll position and any expanded groups survive a
/// refresh — which is what makes live-appending a streaming turn usable.
export function mount(element: HTMLElement, options: MountOptions): void {
  installNativeApi(options.nativeApi);
  // The design system keys its dark tokens off a `dark` class on the ROOT
  // element (upstream's useTheme does the same). Owning it here rather than
  // trusting the host page means yggterm's theme is the only input — a page
  // that forgets the class no longer renders a light transcript inside a dark
  // app, which is exactly what the first render harness showed.
  document.documentElement.classList.toggle("dark", (options.theme ?? "dark") === "dark");
  document.documentElement.style.colorScheme = options.theme ?? "dark";
  // The vendored markdown reads the theme through their hook; push it there too
  // so code highlighting matches the surrounding app.
  setEmbeddedTheme(options.theme ?? "dark");
  if (!root) {
    root = createRoot(element);
  }
  root.render(
    <StrictMode>
      <Timeline {...options} />
    </StrictMode>,
  );
}

export function unmount(): void {
  root?.unmount();
  root = null;
}

declare global {
  interface Window {
    yggtermTranscript?: { mount: typeof mount; unmount: typeof unmount };
  }
}

window.yggtermTranscript = { mount, unmount };
