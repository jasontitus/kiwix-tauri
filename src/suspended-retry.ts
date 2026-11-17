import { isSuspendedError } from "./error-visibility.ts";

/**
 * Why a gate opened. `native` is the platform's authoritative interactive
 * lifecycle event. `fallback` is a visibility/timer guess used when that event
 * may have been missed: the WebView can stay visible while the native shell is
 * still inactive (an Android Activity obscured by a system dialog, an iOS scene
 * that resigned active under Control Center), so it proves nothing.
 */
export type ForegroundWake = "native" | "fallback";

/** A foreground gate that can be resolved by native or visibility events. */
export interface ForegroundGate {
  wait(): Promise<ForegroundWake>;
}

/**
 * One queue for every suspended command. `resume` drains the current waiters
 * exactly once; a repeated signal with no waiters does nothing, so one
 * foreground transition cannot retry the same command twice.
 */
export class ForegroundQueue implements ForegroundGate {
  private waiters: Array<(wake: ForegroundWake) => void> = [];

  wait(): Promise<ForegroundWake> {
    // Promise.withResolvers is ES2024; this project targets ES2022.
    return new Promise((resolve) => {
      this.waiters.push(resolve);
    });
  }

  resume(wake: ForegroundWake = "native"): boolean {
    if (!this.waiters.length) return false;
    const waiters = this.waiters;
    this.waiters = [];
    for (const resolve of waiters) resolve(wake);
    return true;
  }
}

export interface SuspendedRetryOptions {
  /** Mobile builds gate commands on the foreground; desktop never sees one. */
  enabled: boolean;
  /**
   * Checked after the foreground arrives. A request whose privacy or request
   * context moved on is dropped quietly instead of being replayed.
   */
  stillRelevant?: () => boolean;
  /** Receipt/log hook, called once when a command starts waiting. */
  onSuspend?: () => void;
}

/**
 * Speculative (fallback) wakes that may each re-attempt a still-suspended
 * command before it is dropped as a SuspendedError.
 */
export const MAX_FALLBACK_RETRIES = 8;

/**
 * A suspended command is a lifecycle gate: wait for the foreground and run it
 * once more. A suspension after an authoritative native wake is not retried
 * again; it stays a SuspendedError so the UI can drop it without painting
 * anything. A suspension after a speculative fallback wake only means the guess
 * was early, so the command keeps waiting instead of being dropped, for a
 * bounded number of guesses (the native event may never arrive).
 */
export async function retrySuspended<T>(
  attempt: () => Promise<T>,
  gate: ForegroundGate,
  { enabled, stillRelevant, onSuspend }: SuspendedRetryOptions,
): Promise<T> {
  try {
    return await attempt();
  } catch (error) {
    if (!enabled || !isSuspendedError(error) || error.retry === false) throw error;
    onSuspend?.();
    let suspended = error;
    let guesses = 0;
    for (;;) {
      const wake = await gate.wait();
      if (stillRelevant && !stillRelevant()) throw suspended;
      try {
        return await attempt();
      } catch (next) {
        if (wake === "native" || !isSuspendedError(next) || next.retry === false) throw next;
        guesses += 1;
        // The native event can be missing entirely (no event bridge): after
        // a bounded number of guesses the command is dropped as before rather
        // than polling forever.
        if (guesses >= MAX_FALLBACK_RETRIES) throw next;
        suspended = next;
      }
    }
  }
}
