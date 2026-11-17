/**
 * Pure tab-list transitions shared by App's open/undo paths.
 *
 * Tab IDs are identities: React keys, reader-workspace keys, `closeTab`
 * filtering and the saved session all assume they are unique within a window.
 */
export interface TabIdentity {
  id: string;
}

/**
 * Insert `tab` at the end, or right after the active tab when opening in the
 * background. Returns null (no transition) when a tab with the same ID is
 * already open: replaying a stale reopen/launch must not duplicate it.
 */
export function insertTab<T extends TabIdentity>(
  tabs: readonly T[],
  tab: T,
  activeTab: string | null,
  background: boolean,
): T[] | null {
  if (tabs.some((item) => item.id === tab.id)) return null;
  const index = background
    ? tabs.findIndex((item) => item.id === activeTab) + 1
    : tabs.length;
  return [...tabs.slice(0, index), tab, ...tabs.slice(index)];
}

/**
 * Remove exactly the reopened tab from the closed stack. A functional update
 * over the live stack, so two reopen requests handled before a re-render
 * cannot each pop the top and discard an unrelated closed tab.
 */
export function removeReopened<T extends TabIdentity>(
  closed: readonly T[],
  id: string,
): T[] {
  const index = closed.map((item) => item.id).lastIndexOf(id);
  return index < 0 ? [...closed] : [...closed.slice(0, index), ...closed.slice(index + 1)];
}
