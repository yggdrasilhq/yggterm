// Vendored from T3 Code — apps/web/src/editorPreferences.ts
// Copyright (c) 2026 T3 Tools Inc. — MIT (see ../../LICENSE.t3code).
// Upstream: https://github.com/pingdotgg/t3code (pin: UPSTREAM_COMMIT).
// Local edits are catalogued in ../../NOTICE.md; keep them minimal so a
// re-sync stays a copy rather than a merge.
import { EDITORS, EditorId, NativeApi } from "@t3tools/contracts";
import { getLocalStorageItem, setLocalStorageItem, useLocalStorage } from "./hooks/useLocalStorage";
import { useMemo } from "react";

const LAST_EDITOR_KEY = "t3code:last-editor";

export function usePreferredEditor(availableEditors: ReadonlyArray<EditorId>) {
  const [lastEditor, setLastEditor] = useLocalStorage(LAST_EDITOR_KEY, null, EditorId);

  const effectiveEditor = useMemo(() => {
    if (lastEditor && availableEditors.includes(lastEditor)) return lastEditor;
    return EDITORS.find((editor) => availableEditors.includes(editor.id))?.id ?? null;
  }, [lastEditor, availableEditors]);

  return [effectiveEditor, setLastEditor] as const;
}

export function resolveAndPersistPreferredEditor(
  availableEditors: readonly EditorId[],
): EditorId | null {
  const availableEditorIds = new Set(availableEditors);
  const stored = getLocalStorageItem(LAST_EDITOR_KEY, EditorId);
  if (stored && availableEditorIds.has(stored)) return stored;
  const editor = EDITORS.find((editor) => availableEditorIds.has(editor.id))?.id ?? null;
  if (editor) setLocalStorageItem(LAST_EDITOR_KEY, editor, EditorId);
  return editor ?? null;
}

export async function openInPreferredEditor(api: NativeApi, targetPath: string): Promise<EditorId> {
  const { availableEditors } = await api.server.getConfig();
  const editor = resolveAndPersistPreferredEditor(availableEditors);
  if (!editor) throw new Error("No available editors found.");
  await api.shell.openInEditor(targetPath, editor);
  return editor;
}
