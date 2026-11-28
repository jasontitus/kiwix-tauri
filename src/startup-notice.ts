// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * The startup notice band lists `snapshot.startupWarnings`. Dismissing it
 * hides the notices it showed, not later ones. The dismissed set only ever
 * holds notices that are still current: once a notice is withdrawn it is
 * forgotten, so the same sentence noted again later (another cancelled
 * import, say) is new and brings the band back.
 */

/** The notices the band shows: the current ones the person has not closed. */
export function visibleStartupNotices(
  warnings: readonly string[] | undefined,
  dismissed: readonly string[],
): string[] {
  return (warnings ?? []).filter((warning) => !dismissed.includes(warning));
}

/** The notices to remember as dismissed when the band is closed. */
export function dismissStartupNotice(warnings: readonly string[] | undefined): string[] {
  return [...new Set(warnings ?? [])];
}

/**
 * Forget dismissals of notices that are no longer current. Returns `dismissed`
 * itself when nothing changes, so a state update can be skipped.
 */
export function pruneStartupNoticeDismissals(
  warnings: readonly string[],
  dismissed: string[],
): string[] {
  const kept = dismissed.filter((warning) => warnings.includes(warning));
  return kept.length === dismissed.length ? dismissed : kept;
}
