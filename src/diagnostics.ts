import {
  isSuspendedError,
  normalizeSeverity,
  shouldRenderError,
  type ErrorSeverity,
} from "./error-visibility.ts";

export interface DiagnosticEntry {
  id: number;
  at: string;
  severity: ErrorSeverity;
  message: string;
}

export const DIAGNOSTIC_LIMIT = 50;

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Bounded, in-memory record of failures. Nothing is written to disk, sent over
 * the network, or retained across a process restart, so private mode and
 * read-only mode need no extra persistence gate.
 */
export class DiagnosticLog {
  private entries: DiagnosticEntry[] = [];
  private nextId = 1;
  private listeners = new Set<() => void>();
  readonly limit: number;

  constructor(limit = DIAGNOSTIC_LIMIT) {
    this.limit = limit;
  }

  record(error: unknown, severity?: ErrorSeverity): DiagnosticEntry {
    const entry: DiagnosticEntry = {
      id: this.nextId++,
      at: new Date().toISOString(),
      severity: normalizeSeverity(severity),
      message: messageOf(error),
    };
    this.entries = [...this.entries, entry].slice(-this.limit);
    this.emit();
    return entry;
  }

  snapshot = (): readonly DiagnosticEntry[] => this.entries;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  clear(): void {
    if (!this.entries.length) return;
    this.entries = [];
    this.emit();
  }

  /** Stable text for the Troubleshooting copy action. */
  text(): string {
    return JSON.stringify(
      this.entries.map(({ at, severity, message }) => ({ at, severity, message })),
      null,
      2,
    );
  }

  private emit(): void {
    for (const listener of this.listeners) listener();
  }
}

export const diagnostics = new DiagnosticLog();

/**
 * Record one failure and decide whether it should paint a band. A recoverable
 * failure with the setting off is recorded but not rendered; a blocking
 * failure is rendered in both setting states.
 *
 * A suspended command is a lifecycle gate, not a failure: it is neither
 * recorded in the diagnostics log nor rendered, even with technical details
 * on. The API boundary has already retried it once on the next foreground.
 */
export function reportError(
  log: DiagnosticLog,
  error: unknown,
  severity: ErrorSeverity | undefined,
  showTechnicalErrors: boolean,
): boolean {
  if (isSuspendedError(error)) return false;
  log.record(error, severity);
  return shouldRenderError(severity, showTechnicalErrors);
}
