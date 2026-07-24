// LOCAL STUB for `@tanstack/react-router` (see ../NOTICE.md).
//
// Upstream is a routed single-page app: several threads live in one window and
// the route says which one you are looking at. The only vendored consumer is
// `ui/toast.tsx`, which reads the `threadId` param to decide whether a toast
// belongs to the thread on screen.
//
// In yggterm a surface renders exactly ONE session, so that question has a
// constant answer and a router would be ceremony around it. `useParams`
// therefore selects from an empty param set: the toast's own guard already
// treats "no thread id" as "show it", which is right when there is only one
// thread to show.

export function useParams<T>(options: { strict?: boolean; select?: (params: Record<string, unknown>) => T }): T {
  const params: Record<string, unknown> = {};
  return options.select ? options.select(params) : (params as T);
}
