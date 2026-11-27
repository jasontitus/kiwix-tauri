/**
 * Which surface one "dismiss" gesture acts on. Android's system Back and a
 * hardware Escape both close the top-most transient surface before anything
 * else; only Back then navigates. The order lives here, away from the DOM and
 * from React, so it is one table that can be tested.
 */
export interface ShellSurfaces {
  /** A modal `<dialog>` is open. It owns the top layer; everything else is inert. */
  dialog: boolean;
  /** The phone's reader-tab sheet is open. */
  tabSheet: boolean;
  /** A picker, history menu, suggestion list or overlaid reader panel is open. */
  popover: boolean;
  /** Global search was requested (its dialog may still be loading). */
  search: boolean;
  pane: string;
  /** The active reader tab has an earlier article to return to. */
  readerHistory: boolean;
}

export type BackTarget =
  | "dialog"
  | "tab-sheet"
  | "popover"
  | "search"
  | "reader-history"
  | "library"
  | "leave";

/**
 * Android Back. `leave` means the shell has nothing left to dismiss: Back
 * belongs to the system, which backgrounds the task.
 */
export function backTarget(surfaces: ShellSurfaces): BackTarget {
  if (surfaces.dialog) return "dialog";
  if (surfaces.tabSheet) return "tab-sheet";
  if (surfaces.popover) return "popover";
  if (surfaces.search) return "search";
  if (surfaces.pane === "reader" && surfaces.readerHistory) return "reader-history";
  if (surfaces.pane !== "library") return "library";
  return "leave";
}

export type EscapeTarget = "dialog" | "tab-sheet" | "focused-control";

/**
 * A hardware Escape that the native layer consumed before the DOM saw it.
 * `focused-control` hands the key to the shell control that has focus, so a
 * picker or suggestion list closes exactly as it does for a DOM key press; the
 * reader receives it only when no shell control used it.
 */
export function escapeTarget(surfaces: Pick<ShellSurfaces, "dialog" | "tabSheet">): EscapeTarget {
  if (surfaces.dialog) return "dialog";
  if (surfaces.tabSheet) return "tab-sheet";
  return "focused-control";
}

/** Shell popovers that Back closes; each also closes on a DOM Escape. */
export const SHELL_POPOVERS =
  'details.facet-picker[open], details.rw-history-menu[open], [role="combobox"][aria-expanded="true"]';

/**
 * Reader chrome drawn over the article. A side panel that is docked beside the
 * article on a wide screen is layout, not a transient surface.
 */
export const READER_OVERLAYS = ".rw-findbar, .rw-article-preview, .rw-panel";

/** Every state change that can open or close one of the surfaces above. */
export const SURFACE_ATTRIBUTES = ["open", "aria-expanded"];

interface SurfaceDocument {
  querySelector(selectors: string): Element | null;
  querySelectorAll(selectors: string): ArrayLike<Element>;
}

/** The open modal dialogs, in document order. */
export function openDialogs(root: SurfaceDocument): HTMLDialogElement[] {
  return Array.from(root.querySelectorAll("dialog[open]")) as HTMLDialogElement[];
}

/** The first open shell popover, or the reader's overlay when one covers the article. */
export function openPopover(
  root: SurfaceDocument,
  overlaid: (element: Element) => boolean,
): { kind: "shell" | "reader"; element: Element } | null {
  const shell = root.querySelector(SHELL_POPOVERS);
  if (shell) return { kind: "shell", element: shell };
  for (const element of Array.from(root.querySelectorAll(READER_OVERLAYS)))
    if (!element.matches(".rw-panel") || overlaid(element))
      return { kind: "reader", element };
  return null;
}
