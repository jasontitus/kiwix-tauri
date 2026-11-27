/**
 * Three error classes, decided at the call site. The reporting path takes a
 * severity, never a parsed error string.
 *
 * `blocking` means the user's own action failed and the app cannot claim
 * success (unreadable archive, storage handoff, privacy refusal, disk full,
 * a download/import the user started, quit/close). It is always rendered.
 *
 * `recoverable` means the app keeps working (a navigation miss, optional
 * metadata/illustration/suggestion refresh, remembering a dialog directory,
 * a sensor/capability activation the reader can retry). It is recorded and
 * rendered only when "Show technical error details" is on. A sensor
 * decision that was not persisted is blocking: the live decision is still
 * the old one, and the reader must not reload into it.
 *
 * `suspended` is a lifecycle gate, not a failure. It is never recorded or
 * rendered. Ordinary gates retry once on foreground; an ended, finite
 * background checkpoint is cancellation and must never be replayed.
 */
export type ErrorSeverity = "blocking" | "recoverable" | "suspended";

/** Stable machine code returned by the Rust lifecycle gate. */
export const SUSPENDED_CODE = "KIWIX_SUSPENDED";

/**
 * A refused command that only the foreground can clear. This is a distinct
 * error kind so no caller has to match the user-facing sentence.
 */
export class SuspendedError extends Error {
  readonly code = SUSPENDED_CODE;
  readonly retry: boolean;

  constructor(message: string, retry = true) {
    super(message);
    this.name = "SuspendedError";
    this.retry = retry;
  }
}

export function isSuspendedError(value: unknown): value is SuspendedError {
  return (
    value instanceof Error &&
    "code" in value &&
    value.code === SUSPENDED_CODE
  );
}

/** Stable code of a cancellation the user explicitly chose. */
export const CANCELLED_CODE = "KIWIX_USER_CANCELLED";

/**
 * The user declined or cancelled their own action (a consent prompt they
 * answered "Cancel", a transfer they stopped). Like a lifecycle gate it is not
 * a failure: it is never painted as an error. Only a decision the user made
 * may be raised as this type; a prompt that could not be shown, or one the
 * lifecycle withdrew, stays an ordinary error.
 */
export class UserCancelledError extends Error {
  readonly code = CANCELLED_CODE;

  constructor(message: string) {
    super(message);
    this.name = "UserCancelledError";
  }
}

export function isUserCancelledError(value: unknown): value is UserCancelledError {
  return (
    value instanceof Error &&
    "code" in value &&
    value.code === CANCELLED_CODE
  );
}

/**
 * Translate the one Rust envelope (`{"code":"KIWIX_SUSPENDED",...}`) into the
 * typed error. Called once at the API boundary; every other layer uses
 * `isSuspendedError`.
 */
export function parseSuspendedError(message: string): SuspendedError | null {
  const trimmed = message.trim();
  if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return null;
  try {
    const value: unknown = JSON.parse(trimmed);
    if (
      !value ||
      typeof value !== "object" ||
      !("code" in value) ||
      value.code !== SUSPENDED_CODE
    )
      return null;
    return new SuspendedError(
      "message" in value &&
        typeof value.message === "string" &&
        value.message.trim()
        ? value.message
        : SUSPENDED_CODE,
      !("retry" in value && value.retry === false),
    );
  } catch {
    return null;
  }
}

/** A trusted native provider acquisition failure, never a rendered-page hint. */
export class ProviderDocumentError extends Error {
  readonly code: string;
  readonly retry: boolean;
  readonly sourceReference: string;
  readonly bookId?: string;
  readonly installationId?: string;
  constructor(
    message: string,
    code: string,
    retry: boolean,
    sourceReference: string,
    bookId?: string,
    installationId?: string,
  ) {
    super(message);
    this.name = "ProviderDocumentError";
    this.code = code;
    this.retry = retry;
    this.sourceReference = sourceReference;
    this.bookId = bookId;
    this.installationId = installationId;
  }
}

export function parseProviderDocumentError(message: string): ProviderDocumentError | null {
  try {
    const value: unknown = JSON.parse(message);
    if (!value || typeof value !== "object") return null;
    const error = value as Record<string, unknown>;
    if (typeof error.code !== "string" || !/^[A-Z][A-Z0-9_]{0,127}$/.test(error.code) ||
        typeof error.message !== "string" || !error.message.trim() || error.message.length > 4096 ||
        typeof error.retry !== "boolean" ||
        typeof error.sourceReference !== "string" ||
        !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(error.sourceReference) ||
        (error.bookId !== undefined && (typeof error.bookId !== "string" || !error.bookId || error.bookId.length > 256)) ||
        (error.installationId !== undefined && (typeof error.installationId !== "string" || !error.installationId || error.installationId.length > 256)))
      return null;
    return new ProviderDocumentError(error.message, error.code, error.retry, error.sourceReference,
      error.bookId as string | undefined, error.installationId as string | undefined);
  } catch {
    return null;
  }
}

/** Only temporary unavailability of this exact retained source is locally retryable.
 * Revocation, deletion, replacement installations and unscoped failures remain
 * ordinary visible failures; matching bytes/metadata alone grant no authority.
 */
export function isProviderUnavailableFor(
  value: unknown,
  book: { id: string; installationId: string; sourceReference?: { reference: string } | null } | undefined,
): value is ProviderDocumentError {
  return value instanceof ProviderDocumentError &&
    (value.code === "PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT" || value.code === "PROVIDER_LEASE_CONFLICT") && value.retry &&
    !!book && value.bookId === book.id && value.installationId === book.installationId &&
    value.sourceReference === book.sourceReference?.reference;
}

/**
 * Classification for every call site converted to the severity-aware path.
 * Keeping the table here makes the classification testable and prevents a
 * literal at one call site from drifting away from the documented class.
 */
export const ERROR_SITES = {
  "reader.navigation-missing": "recoverable",
  // An archive with nothing to open is a capability limit of that archive,
  // not a failed user action; the current page stays readable.
  "reader.random-article": "recoverable",
  "reader.record-visit": "recoverable",
  "reader.record-search": "recoverable",
  "reader.remove-search": "recoverable",
  "reader.search-cancel": "recoverable",
  "reader.search-dispose": "recoverable",
  "reader.sensor-activate": "recoverable",
  "reader.sensor-deactivate": "recoverable",
  "reader.sensor-location": "recoverable",
  // A revoke/reset the store did not accept leaves the old decision live; the
  // reader must show that failure instead of reloading into an auto-grant.
  "reader.sensor-revoke": "blocking",
  "reader.fullscreen": "recoverable",
  "reader.shortcut-listener": "recoverable",
  "reader.pin-shortcut": "recoverable",
  "library.record-search": "recoverable",
  "library.suggestion-refresh": "recoverable",
  "library.cover-refresh": "recoverable",
  "library.speech-search": "recoverable",
  "dialog.remember-directory": "recoverable",
} as const satisfies Record<string, ErrorSeverity>;

export type ErrorSite = keyof typeof ERROR_SITES;

export function normalizeSeverity(severity?: ErrorSeverity): ErrorSeverity {
  return severity ?? "blocking";
}

/**
 * The one surface that owns a rendered failure. The band sits in the page, so
 * while a modal dialog is open it is under the backdrop, inert and cannot be
 * dismissed. A blocking failure reported then is owned by an alert dialog in
 * the top layer instead; it is never painted in both. A recoverable diagnostic
 * (shown only with technical details on) does not interrupt a dialog.
 */
export type ErrorSurface = "band" | "dialog";

export function errorSurface(
  severity: ErrorSeverity | undefined,
  coveredByDialog: boolean,
): ErrorSurface {
  return coveredByDialog && normalizeSeverity(severity) === "blocking"
    ? "dialog"
    : "band";
}

/** Whether a failure should paint a band for this setting state. */
export function shouldRenderError(
  severity: ErrorSeverity | undefined,
  showTechnicalErrors: boolean,
): boolean {
  const normalized = normalizeSeverity(severity);
  // A lifecycle gate is not a failure: no card, banner or retry affordance.
  if (normalized === "suspended") return false;
  return normalized === "blocking" || showTechnicalErrors;
}
