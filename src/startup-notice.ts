// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * The startup notice band lists `snapshot.startupWarnings`. Dismissing it
 * hides the notices it showed, not every later one: a notice that arrives
 * afterwards (an import cancelled by a privacy switch, a cleanup failure)
 * brings the band back. A withdrawn notice does not.
 */
export function startupNoticeVisible(
  warnings: readonly string[] | undefined,
  dismissed: readonly string[],
): boolean {
  return (warnings ?? []).some((warning) => !dismissed.includes(warning));
}

/** The notices to remember as dismissed when the band is closed. */
export function dismissStartupNotice(
  warnings: readonly string[] | undefined,
  dismissed: readonly string[],
): string[] {
  return [...new Set([...dismissed, ...(warnings ?? [])])];
}
