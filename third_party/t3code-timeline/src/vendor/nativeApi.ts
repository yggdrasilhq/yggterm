// LOCAL REPLACEMENT for T3 Code's apps/web/src/nativeApi.ts (see ../NOTICE.md §1).
//
// Upstream this module reaches Electron over a WebSocket transport
// (`wsNativeApi` → `wsTransport`), which drags in most of `@t3tools/contracts`
// — their whole WS protocol — for the sake of exactly two call sites:
// "open this file in my editor" (ChatMarkdown) and "save this plan into the
// workspace" (ProposedPlanCard).
//
// yggterm is not Electron and owns those capabilities itself, so the transport
// is replaced by one seam the embedder fills in. The SHAPE upstream expects is
// kept — `readNativeApi()` returning a falsy value when absent — so the
// vendored call sites are untouched: both already handle "no native API".

/// The subset of upstream's NativeApi the vendored timeline actually calls.
/// Everything else upstream declares is transport we do not have.
export interface NativeApi {
  openInEditor?: (input: { editorId: string; path: string }) => void | Promise<void>;
  writeProjectFile?: (input: { path: string; contents: string }) => void | Promise<void>;
}

let hostApi: NativeApi | undefined;

/// Installed once by `mount()`. `undefined` (the default) makes every vendored
/// call site take its own "no native API" branch, so an embedder that wires
/// nothing still renders a correct transcript — it just cannot open files.
export function installNativeApi(api: NativeApi | undefined): void {
  hostApi = api;
}

export function readNativeApi(): NativeApi | undefined {
  return hostApi;
}

export function ensureNativeApi(): NativeApi {
  const api = readNativeApi();
  if (!api) {
    throw new Error("Native API not available in this embedding");
  }
  return api;
}
