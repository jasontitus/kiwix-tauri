// Selects the interface skin in src/platform.css. Native builds know their
// target at compile time; the headless browser preview infers it from the
// user agent. Development builds accept `?platform=` to preview another skin.
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";

export type UiPlatform = "ios" | "android" | "macos" | "windows" | "linux";

const platforms: readonly UiPlatform[] = ["ios", "android", "macos", "windows", "linux"];

function fromTauriTarget(target: string | undefined): UiPlatform | null {
  switch (target) {
    case "ios":
    case "android":
    case "windows":
    case "linux":
      return target;
    case "darwin":
    case "macos":
      return "macos";
    default:
      return null;
  }
}

function fromUserAgent(): UiPlatform {
  const agent = navigator.userAgent;
  if (/android/i.test(agent)) return "android";
  // iPadOS reports a Mac user agent; touch support distinguishes it.
  if (/iphone|ipad|ipod/i.test(agent) || (/macintosh/i.test(agent) && navigator.maxTouchPoints > 1))
    return "ios";
  if (/macintosh|mac os x/i.test(agent)) return "macos";
  if (/windows/i.test(agent)) return "windows";
  return "linux";
}

function previewOverride(): UiPlatform | null {
  if (!import.meta.env.DEV) return null;
  const requested = new URLSearchParams(location.search).get("platform");
  return platforms.find((platform) => platform === requested) ?? null;
}

export const uiPlatform: UiPlatform =
  previewOverride() ?? fromTauriTarget(import.meta.env.TAURI_ENV_PLATFORM) ?? fromUserAgent();

const isDesktopApp =
  isTauri() && (uiPlatform === "macos" || uiPlatform === "windows" || uiPlatform === "linux");

/** macOS windows draw the traffic lights over the shell (tauri.conf.json and
 * src-tauri/src/windows.rs); the page supplies the drag regions and insets. */
export const usesOverlayTitleBar = isDesktopApp && uiPlatform === "macos";

/** Keep the native title bar in the application's theme rather than the OS's.
 * The setting is app-wide on macOS and Linux, per window on Windows. On
 * Android the strips behind the status and navigation bars are native window
 * background: the plugin tints them, and their icons, to match. */
export function applyWindowTheme(theme: "system" | "light" | "dark"): void {
  if (isTauri() && uiPlatform === "android") {
    api("mobile_apply_theme", { theme }).catch((error) =>
      console.warn("Could not apply the window theme", error));
    return;
  }
  if (!isDesktopApp) return;
  getCurrentWindow()
    .setTheme(theme === "system" ? null : theme)
    .catch((error) => console.warn("Could not apply the window theme", error));
}

// Full screen hides the traffic lights, so their inset is dropped with them.
function trackFullscreen(): void {
  const window = getCurrentWindow();
  let applied: string | undefined;
  let generation = 0;
  let pending: number | undefined;
  const write = (fullscreen: boolean) => {
    const next = fullscreen ? "fullscreen" : "overlay";
    if (applied === next) return;
    applied = next;
    document.documentElement.dataset.titlebar = next;
  };
  const sync = () => {
    // Only the newest read may write: resize events overlap their IPC replies.
    const issued = ++generation;
    void window.isFullscreen().then(
      (fullscreen) => {
        if (issued === generation) write(fullscreen);
      },
      (error) => {
        if (issued === generation)
          console.warn("Could not read the full-screen state", error);
      },
    );
  };
  write(false);
  void sync();
  // macOS streams resize events while an edge is dragged; coalesce them to one
  // trailing read so a drag costs one round trip instead of dozens.
  void window.onResized(() => {
    clearTimeout(pending);
    pending = setTimeout(() => {
      pending = undefined;
      sync();
    }, 100);
  });
}

export function applyPlatform(): void {
  document.documentElement.dataset.platform = uiPlatform;
  if (usesOverlayTitleBar) trackFullscreen();
  // The webview's own menu (Back, Reload, Inspect) gives the shell away. Keep
  // it for editable text and selections, and in development builds; handlers
  // that already show a custom menu call preventDefault themselves.
  if (isDesktopApp && !import.meta.env.DEV)
    document.addEventListener("contextmenu", (event) => {
      const target = event.target as Element | null;
      if (target?.closest("input, textarea, [contenteditable='true']")) return;
      if (!document.getSelection()?.isCollapsed) return;
      event.preventDefault();
    });
}
