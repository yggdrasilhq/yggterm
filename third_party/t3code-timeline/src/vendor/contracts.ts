// LOCAL REPLACEMENT for T3 Code's `@t3tools/contracts` (see ../NOTICE.md).
//
// Upstream that package is the whole client/server protocol, built on `effect`
// Schema: orchestration, git, terminals, projects, editors, the WS envelope.
// The vendored timeline needs a sliver of it — branded id types it uses as Map
// keys and prop types, plus two runtime values. Pulling in the real package
// would drag in the server contract this integration exists to avoid.
//
// The ids are reproduced as TypeScript brands rather than effect schemas: the
// timeline only ever *reads* them, so the compile-time distinctness is the part
// that matters, and re-deriving them from `effect` would buy nothing but a
// dependency. The runtime values below are copied verbatim from upstream and
// must be re-copied if it changes — they are behaviour, not types.

declare const brand: unique symbol;
type Branded<Base, Name extends string> = Base & { readonly [brand]: Name };

export type TrimmedNonEmptyString = Branded<string, "TrimmedNonEmptyString">;
export type ThreadId = Branded<string, "ThreadId">;
export type ProjectId = Branded<string, "ProjectId">;
export type CommandId = Branded<string, "CommandId">;
export type MessageId = Branded<string, "MessageId">;
export type TurnId = Branded<string, "TurnId">;
export type ApprovalRequestId = Branded<string, "ApprovalRequestId">;
export type CheckpointRef = Branded<string, "CheckpointRef">;
export type OrchestrationProposedPlanId = Branded<string, "OrchestrationProposedPlanId">;

// Upstream these are effect schema objects that double as validators. The
// vendored code calls them only where a branded value must be produced from a
// string, so a cast-through helper is the faithful minimum.
const makeId = <T>() =>
  Object.assign((value: string) => value as T, {
    make: (value: string) => value as T,
    // Upstream's escape hatch for values already known to be valid.
    makeUnsafe: (value: string) => value as T,
  });
export const TrimmedNonEmptyString = makeId<TrimmedNonEmptyString>();
export const ThreadId = makeId<ThreadId>();
export const ProjectId = makeId<ProjectId>();
export const CommandId = makeId<CommandId>();
export const MessageId = makeId<MessageId>();
export const TurnId = makeId<TurnId>();
export const ApprovalRequestId = makeId<ApprovalRequestId>();
export const CheckpointRef = makeId<CheckpointRef>();
export const OrchestrationProposedPlanId = makeId<OrchestrationProposedPlanId>();

/// Copied verbatim from upstream `packages/contracts/src/editor.ts`.
export const EDITORS = [
  { id: "cursor", label: "Cursor", command: "cursor" },
  { id: "vscode", label: "VS Code", command: "code" },
  { id: "zed", label: "Zed", command: "zed" },
  { id: "antigravity", label: "Antigravity", command: "agy" },
  { id: "file-manager", label: "File Manager", command: null },
] as const;
export type EditorId = (typeof EDITORS)[number]["id"];
/// Upstream this is an effect literal schema used as a value too.
export const EditorId = Object.assign((value: string) => value as EditorId, {
  make: (value: string) => value as EditorId,
  makeUnsafe: (value: string) => value as EditorId,
  literals: EDITORS.map((editor) => editor.id),
});

/// Copied verbatim from upstream `packages/contracts/src/providerRuntime.ts`.
export const TOOL_LIFECYCLE_ITEM_TYPES = [
  "command_execution",
  "file_change",
  "mcp_tool_call",
  "dynamic_tool_call",
  "collab_agent_tool_call",
  "web_search",
  "image_view",
] as const;
export type ToolLifecycleItemType = (typeof TOOL_LIFECYCLE_ITEM_TYPES)[number];
export function isToolLifecycleItemType(value: string): value is ToolLifecycleItemType {
  return (TOOL_LIFECYCLE_ITEM_TYPES as readonly string[]).includes(value);
}

export type ProviderKind = "codex" | "claude" | "cursor" | "opencode";
export type RuntimeMode = "full-access" | "read-only" | "safe";
export type ProviderInteractionMode = "default" | "plan" | "auto";
export type OrchestrationSessionStatus = string;

export interface OrchestrationLatestTurn {
  turnId: TurnId | null;
  startedAt?: string | null;
  completedAt?: string | null;
  status?: string | null;
}

export interface OrchestrationThreadActivity {
  id: string;
  createdAt: string;
  kind?: string;
  [key: string]: unknown;
}

export interface ProjectScript {
  id: string;
  name: string;
  command: string;
}

export interface GitBranch {
  name: string;
  current?: boolean;
}

export interface UserInputQuestion {
  id: string;
  prompt: string;
  [key: string]: unknown;
}

export type { NativeApi } from "./nativeApi";
