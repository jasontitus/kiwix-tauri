/* Runs synchronously before the application module and its stylesheet paint.
   The native shell injects the saved theme (and platform) as a window global
   before any page script; browser previews fall back to the mirror written by
   src/App.tsx, then to the system theme. Nothing here trusts archive content. */
(() => {
  "use strict";
  const themes = ["light", "dark", "system"];
  const key = "kiwix.shell-theme";
  const root = document.documentElement;
  let boot;
  try {
    boot = window.__KIWIX_BOOT__;
  } catch (_) {
    /* A locked-down global cannot be read; use the fallbacks below. */
  }
  let theme = boot && themes.includes(boot.theme) ? boot.theme : "";
  if (!theme) {
    try {
      const stored = window.localStorage.getItem(key);
      if (themes.includes(stored)) theme = stored;
    } catch (_) {
      /* Storage can be unavailable; the system theme is still correct. */
    }
  }
  root.dataset.theme = theme || "system";
  if (boot && typeof boot.platform === "string")
    root.dataset.platform = boot.platform;
})();
