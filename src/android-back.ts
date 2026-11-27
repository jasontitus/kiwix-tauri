/**
 * Ownership of Android's system Back.
 *
 * Tauri's Android `AppPlugin` owns the activity's Back callback. While no
 * `back-button` listener is registered it applies the platform default (walk
 * WebView history, otherwise hand Back to the system, which backgrounds the
 * task). Once a listener is registered the press is only delivered to the
 * page: the plugin offers no call that hands that press back to the system,
 * and its one exit command finishes the task, which would end work in
 * progress (and is not granted to this window).
 *
 * So the listener is held only while the shell has something to dismiss, and
 * withdrawn when it does not: leaving the app at the root stays exactly the
 * platform default.
 *
 * The plugin keeps its listener list for the life of the process, while the
 * activity's WebView (and this document) can be replaced. A listener that a
 * previous document left behind would make every later Back a dead key, so the
 * registered channel is remembered and removed by the next document.
 */
export interface BackListener {
  channelId: number;
  unregister(): Promise<void>;
}

export interface BackBridge {
  listen(handler: () => void): Promise<BackListener>;
  /** Remove a listener that a previous document of this process registered. */
  forget(channelId: number): Promise<void>;
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> | null;
}

export interface BackRegistration {
  /** Whether the shell currently has something for Back to dismiss. */
  want(active: boolean): void;
  /** Resolves when every requested change has been applied. */
  settled(): Promise<void>;
}

export const BACK_LISTENER_KEY = "kiwix-android-back-listener";

export function createBackRegistration(
  bridge: BackBridge,
  handler: () => void,
  onError: (error: unknown) => void,
): BackRegistration {
  let wanted = false;
  let current: BackListener | null = null;
  let recovered = false;
  let queue: Promise<void> = Promise.resolve();

  const stored = (): string | null => {
    try {
      return bridge.storage?.getItem(BACK_LISTENER_KEY) ?? null;
    } catch {
      return null;
    }
  };
  const remember = (listener: BackListener) => {
    try {
      bridge.storage?.setItem(BACK_LISTENER_KEY, String(listener.channelId));
    } catch {
      // Without storage a replaced document cannot be recovered from; the
      // listener itself still works for this one.
    }
  };
  const release = (listener: BackListener) => {
    try {
      // A newer registration may have replaced the record.
      if (stored() === String(listener.channelId))
        bridge.storage?.removeItem(BACK_LISTENER_KEY);
    } catch {
      // See `remember`.
    }
  };
  const recover = async () => {
    const value = stored();
    if (value === null) return;
    const channelId = Number(value);
    if (Number.isSafeInteger(channelId) && channelId >= 0)
      await bridge.forget(channelId);
    try {
      if (stored() === value) bridge.storage?.removeItem(BACK_LISTENER_KEY);
    } catch {
      // See `remember`.
    }
  };

  // Each step applies the newest wish, so a burst of changes settles on the
  // last one with at most one registration alive.
  const step = async () => {
    if (!recovered) {
      recovered = true;
      await recover();
    }
    if (wanted && !current) {
      const listener = await bridge.listen(handler);
      current = listener;
      remember(listener);
    } else if (!wanted && current) {
      const listener = current;
      await listener.unregister();
      // Only now is the native default back in charge.
      if (current === listener) current = null;
      release(listener);
    }
  };
  const sync = () => {
    queue = queue.then(step).catch(onError);
  };
  // A stale listener blocks Back even when this document never wants one.
  sync();

  return {
    want(active) {
      wanted = active;
      // Also retries a registration or removal that failed earlier.
      if ((current !== null) !== active) sync();
    },
    settled: () => queue,
  };
}

/** The real bridge. Imported lazily: only the Android shell ever calls it. */
export function tauriBackBridge(): BackBridge {
  let storage: BackBridge["storage"] = null;
  try {
    storage = window.localStorage;
  } catch {
    storage = null;
  }
  return {
    async listen(handler) {
      const { onBackButtonPress } = await import("@tauri-apps/api/app");
      return onBackButtonPress(() => handler());
    },
    async forget(channelId) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("plugin:app|remove_listener", { event: "back-button", channelId });
    },
    storage,
  };
}
