import { api, isNative } from "./api";

type BootWindow = Window & {
  __KIWIX_BOOT_STATE__?: {
    phase: string;
    detail: string;
    ready: boolean;
  };
  __kiwixBootPhase?: (phase: string, detail?: string) => void;
  __kiwixFrontendReady?: () => void;
};

let reported = false;

/** Mark the window ready only after React committed a usable snapshot. */
export function reportFrontendReady(): void {
  if (reported) return;
  reported = true;
  const boot = window as BootWindow;
  boot.__kiwixBootPhase?.("ready", "first snapshot applied");
  boot.__kiwixFrontendReady?.();
  if (isNative) {
    // The native watchdog only needs the acknowledgement. A failed invoke is
    // already represented by the panel the page can show on its own.
    void api("frontend_ready", {}).catch(() => undefined);
  }
}

/** Record the last startup stage for the native/page watchdog diagnostics. */
export function reportBootPhase(phase: string, detail?: string): void {
  (window as BootWindow).__kiwixBootPhase?.(phase, detail);
}
