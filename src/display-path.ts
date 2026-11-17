// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Native Windows paths may arrive in verbatim form (`\\?\C:\…`,
 * `\\?\UNC\server\share\…`). They are correct identities, so the app keeps
 * comparing them as received, but people expect the ordinary form.
 * Display only: never compare, store or send the result back to the backend.
 */
export function displayPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  if (/^\\\\\?\\[A-Za-z]:\\/.test(path)) return path.slice(4);
  return path;
}
