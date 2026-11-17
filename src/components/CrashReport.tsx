import { useCallback, useEffect, useState } from "react";
import { Copy, Eye, EyeOff, Mail, Trash2 } from "lucide-react";
import { api, isNative } from "../api";
import { formatDate, t } from "../i18n";
import type { Book } from "../types";
import { confirmAction, Modal } from "./UI";

/** Matches `src-tauri/src/crash.rs` schema 1. */
export interface CrashRecord {
  schema: number;
  capturedAt: string;
  appVersion: string;
  platform: string;
  architecture: string;
  phase: "application" | "reader";
  operation: string;
  thread: string;
  message: string;
  location: string;
  stack: string[];
  archive?: {
    id: string;
    validation: "valid" | "invalid" | "unknown";
  };
  reportedAt?: string;
}
export interface CrashReportState {
  record: CrashRecord | null;
  canSend: boolean;
}

const CRASH_RECIPIENT = "contact@kiwix.org";
const MAX_MAIL_CHARS = 6_000;

export function crashReportText(record: CrashRecord): string {
  return JSON.stringify(record, null, 2);
}

async function copyText(text: string) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.append(area);
  try {
    area.select();
    if (!document.execCommand("copy"))
      throw new Error(t("The report could not be copied."));
  } finally {
    area.remove();
  }
}

function validationLabel(record: CrashRecord): string | null {
  if (!record.archive) return null;
  switch (record.archive.validation) {
    case "valid":
      return t("Archive validation at crash time: verified.");
    case "invalid":
      return t("Archive validation at crash time: failed.");
    default:
      return t("Archive validation at crash time: not checked.");
  }
}

interface ActionsProps {
  state: CrashReportState;
  books?: Book[];
  readOnly: boolean;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}
export function CrashReportActions({
  state,
  books = [],
  readOnly,
  onChanged,
  onError,
}: ActionsProps) {
  const [viewing, setViewing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const record = state.record;
  if (!record)
    return <p className="muted">{t("No crash report is stored on this device.")}</p>;
  const current = record;
  const text = crashReportText(current);
  const archiveTitle = record.archive
    ? books.find((book) => book.id === record.archive?.id)?.title ??
      t("Archive no longer in the library")
    : null;
  const validation = validationLabel(record);
  async function copy() {
    if (busy) return;
    setBusy(true);
    setNotice("");
    try {
      await copyText(text);
      setNotice(t("Crash report copied to the clipboard."));
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  }
  async function send() {
    if (busy || readOnly) return;
    setBusy(true);
    setNotice("");
    try {
      const subject = `${t("Kiwix Workbench crash report")} ${current.appVersion} (${current.platform})`;
      const body =
        text.length > MAX_MAIL_CHARS
          ? `${text.slice(0, MAX_MAIL_CHARS)}\n\n[${t("Report truncated for email")}]`
          : text;
      const url = `mailto:${CRASH_RECIPIENT}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
      // The existing external-link command validates the scheme and asks the
      // platform for a compose intent. No report content is sent by this app.
      await api("open_external", { url, confirmed: true });
      await api("crash_report_acknowledge");
      setNotice(
        t(
          "Your mail app opened with the report. The local record is marked as reported; nothing was sent automatically.",
        ),
      );
      await onChanged();
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  }
  async function remove() {
    if (busy || readOnly) return;
    if (
      !(await confirmAction(
        t("Delete this crash report? This cannot be undone."),
        { title: t("Delete crash report"), confirmLabel: t("Delete"), danger: true },
      ))
    )
      return;
    setBusy(true);
    setNotice("");
    try {
      await api("crash_report_delete");
      await onChanged();
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="crash-report">
      <dl className="facts">
        <div>
          <dt>{t("Captured")}</dt>
          <dd>{formatDate(record.capturedAt)}</dd>
        </div>
        <div>
          <dt>{t("Application")}</dt>
          <dd>
            {record.appVersion} · {record.platform}/{record.architecture}
          </dd>
        </div>
        <div>
          <dt>{t("Operation")}</dt>
          <dd>
            {record.phase === "reader" ? t("Reading") : t("Application")} ·{" "}
            {record.operation}
          </dd>
        </div>
        {archiveTitle && (
          <div>
            <dt>{t("Archive")}</dt>
            <dd>{archiveTitle}</dd>
          </div>
        )}
        {record.reportedAt && (
          <div>
            <dt>{t("Status")}</dt>
            <dd>{t("Handed to a mail app on {date}", { date: formatDate(record.reportedAt) })}</dd>
          </div>
        )}
      </dl>
      {validation && (
        <p
          role="status"
          className={
            record.archive?.validation === "invalid"
              ? "inline-error"
              : record.archive?.validation === "valid"
                ? "success-text"
                : "muted"
          }
        >
          {validation}
        </p>
      )}
      <p className="muted">
        {t(
          "The report contains the app version, platform, operation, a redacted panic message and stack, and the last archive validation result. Paths are reduced to file names, URL queries and reader tokens are removed, and secret-looking values are redacted. Review it before sharing.",
        )}
      </p>
      {viewing && (
        <label className="field">
          {t("Crash report JSON")}
          <textarea readOnly rows={14} value={text} spellCheck={false} />
        </label>
      )}
      {notice && <p role="status">{notice}</p>}
      {readOnly && (
        <p className="muted">
          {t(
            "Deleting or marking a report is disabled in strict read-only mode.",
          )}
        </p>
      )}
      <div className="button-row">
        <button type="button" onClick={() => setViewing((value) => !value)}>
          {viewing ? <EyeOff size={16} /> : <Eye size={16} />}
          {viewing ? t("Hide report") : t("View report")}
        </button>
        <button type="button" disabled={busy} onClick={() => void copy()}>
          <Copy size={16} />
          {t("Copy report")}
        </button>
        {state.canSend && isNative && !record.reportedAt && (
          <button
            type="button"
            disabled={busy || readOnly}
            onClick={() => void send()}
          >
            <Mail size={16} />
            {t("Send by email")}
          </button>
        )}
        <button
          type="button"
          disabled={busy || readOnly}
          onClick={() => void remove()}
        >
          <Trash2 size={16} />
          {t("Delete report")}
        </button>
      </div>
    </div>
  );
}

/** Settings panel: always shows the latest record, reported or not. */
export function CrashReportSection({
  books,
  readOnly,
  onError,
}: {
  books: Book[];
  readOnly: boolean;
  onError: (error: unknown) => void;
}) {
  const [state, setState] = useState<CrashReportState | null>(null);
  const load = useCallback(async () => {
    try {
      setState(await api<CrashReportState>("crash_report"));
    } catch (error) {
      onError(error);
    }
  }, [onError]);
  useEffect(() => {
    void load();
  }, [load]);
  if (!state)
    return (
      <p className="muted" role="status">
        {t("Reading the local crash report…")}
      </p>
    );
  return (
    <CrashReportActions
      state={state}
      books={books}
      readOnly={readOnly}
      onChanged={load}
      onError={onError}
    />
  );
}

/**
 * Startup prompt. A crash record stays local until the user views, copies,
 * mails or deletes it; closing the prompt without acting keeps it for the next
 * launch.
 */
export default function CrashReportPrompt({
  books,
  readOnly,
  onError,
}: {
  books: Book[];
  readOnly: boolean;
  onError: (error: unknown) => void;
}) {
  const [state, setState] = useState<CrashReportState | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const load = useCallback(async () => {
    try {
      setState(await api<CrashReportState>("crash_report"));
    } catch (error) {
      onError(error);
    }
  }, [onError]);
  useEffect(() => {
    void load();
  }, [load]);
  if (dismissed || !state?.record || state.record.reportedAt) return null;
  return (
    <Modal
      title={t("Crash report available")}
      onClose={() => setDismissed(true)}
    >
      <p>
        {t(
          "Kiwix did not close normally last time. A local crash report is available. Nothing has been sent automatically.",
        )}
      </p>
      <CrashReportActions
        state={state}
        books={books}
        readOnly={readOnly}
        onChanged={load}
        onError={onError}
      />
      <div className="modal-actions">
        <button autoFocus onClick={() => setDismissed(true)}>
          {t("Close")}
        </button>
      </div>
    </Modal>
  );
}
