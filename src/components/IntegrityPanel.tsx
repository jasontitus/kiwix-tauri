import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, capturePrivacyContext, isMobile, isPrivacyContextCurrent, isPrivacyTransitionError, saveText, type PrivacyContext } from "../api";
import { formatBytes, formatDate, formatNumber, t } from "../i18n";
import type { Book } from "../types";

interface IntegrityTarget {
  bookId: string;
  installationId: string;
}
export interface IntegrityResult extends IntegrityTarget {
  status: "valid" | "invalid" | "unknown" | "unavailable";
  checkedAt: string | null;
  persisted: boolean;
}
interface IntegrityJob {
  running: boolean;
  phase: string;
  bookId: string | null;
  completed: number;
  total: number;
  bytesChecked: number;
  totalBytes: number;
  targets: IntegrityTarget[];
  results: IntegrityResult[];
}
interface IntegritySnapshot {
  job: IntegrityJob;
  books: IntegrityResult[];
}
/** A book and a verification target name the same installation through different
 * fields; one key formula keeps every index and lookup in lockstep. */
function identityKey(id: string, installationId: string) {
  return `${id}\u0000${installationId}`;
}
/** Field order matters: results are compared positionally to avoid allocating. */
function sameSnapshot(a: IntegritySnapshot, b: IntegritySnapshot) {
  if (a === b) return true;
  const before = a.job;
  const after = b.job;
  if (before.running !== after.running || before.phase !== after.phase || before.bookId !== after.bookId ||
    before.completed !== after.completed || before.total !== after.total ||
    before.bytesChecked !== after.bytesChecked || before.totalBytes !== after.totalBytes ||
    before.targets.length !== after.targets.length || before.results.length !== after.results.length ||
    a.books.length !== b.books.length) return false;
  for (let index = 0; index < before.targets.length; index++) {
    const x = before.targets[index];
    const y = after.targets[index];
    if (x.bookId !== y.bookId || x.installationId !== y.installationId) return false;
  }
  for (let index = 0; index < before.results.length; index++) {
    const x = before.results[index];
    const y = after.results[index];
    if (x.bookId !== y.bookId || x.installationId !== y.installationId ||
      x.status !== y.status || x.checkedAt !== y.checkedAt || x.persisted !== y.persisted) return false;
  }
  for (let index = 0; index < a.books.length; index++) {
    const x = a.books[index];
    const y = b.books[index];
    if (x.bookId !== y.bookId || x.installationId !== y.installationId ||
      x.status !== y.status || x.checkedAt !== y.checkedAt || x.persisted !== y.persisted) return false;
  }
  return true;
}
/** A running verification needs fast progress; an idle panel only needs to notice
 * a verification another view started. Both stay cheap while the document is hidden. */
const ACTIVE_POLL_MS = 1000;
const IDLE_POLL_MS = 5000;
/** Read snapshots only; the service owns verification across navigation. */
export function useIntegritySnapshot(privateMode: boolean, readOnly: boolean, onError: (error: unknown) => void) {
  const [state, setState] = useState<{ snapshot: IntegritySnapshot; privacy: PrivacyContext; privateMode: boolean; readOnly: boolean } | null>(null);
  const errors = useRef(onError);
  errors.current = onError;
  const revision = useRef(0);
  const mounted = useRef(false);
  const verifying = useRef(false);
  const wake = useRef<(() => void) | null>(null);
  const refresh = useCallback(async () => {
    const request = ++revision.current;
    const privacy = capturePrivacyContext();
    if (!privacy || !mounted.current || document.visibilityState === "hidden") return;
    try {
      const snapshot = await api<IntegritySnapshot>("integrity_status");
      if (mounted.current && request === revision.current && isPrivacyContextCurrent(privacy)) {
        verifying.current = snapshot.job.running;
        // Polling returns an equal payload most of the time; storing it would
        // re-render every result row once per second.
        setState((previous) =>
          previous && previous.privateMode === privateMode && previous.readOnly === readOnly &&
            previous.privacy.generation === privacy.generation && previous.privacy.epoch === privacy.epoch &&
            previous.privacy.privateMode === privacy.privateMode && previous.privacy.readOnly === privacy.readOnly &&
            sameSnapshot(previous.snapshot, snapshot)
            ? previous
            : { snapshot, privacy, privateMode, readOnly });
        // A run that just started or finished changes the cadence immediately
        // instead of waiting out an idle interval.
        wake.current?.();
      }
    } catch (error) {
      if (!mounted.current || request !== revision.current || !isPrivacyContextCurrent(privacy)) return;
      // Native retirement can precede the root's replacement snapshot. This
      // observation has lost its authority even while the shell still holds the
      // old generation; discard it rather than presenting it as verification.
      if (isPrivacyTransitionError(error)) {
        setState(null);
        return;
      }
      errors.current(error);
    }
  }, [privateMode, readOnly]);
  useEffect(() => {
    mounted.current = true;
    verifying.current = false;
    setState(null);
    let timer: number | undefined;
    let active = true;
    let polling = false;
    function schedule() {
      clearTimeout(timer);
      const fast = verifying.current && document.visibilityState !== "hidden";
      timer = window.setTimeout(poll, fast ? ACTIVE_POLL_MS : IDLE_POLL_MS);
    }
    async function poll() {
      // A slow idle cadence must not delay the first update after the window
      // returns; a focus-driven poll replaces the pending timer.
      if (polling) return;
      polling = true;
      await refresh();
      polling = false;
      if (active) schedule();
    }
    function onVisibility() {
      if (!active || document.visibilityState === "hidden") return;
      clearTimeout(timer);
      void poll();
    }
    wake.current = schedule;
    document.addEventListener("visibilitychange", onVisibility);
    void poll();
    return () => {
      active = false;
      wake.current = null;
      mounted.current = false;
      revision.current++;
      document.removeEventListener("visibilitychange", onVisibility);
      clearTimeout(timer);
    };
  }, [refresh]);
  const snapshot = state && state.privateMode === privateMode && state.readOnly === readOnly && isPrivacyContextCurrent(state.privacy) ? state.snapshot : null;
  return { snapshot, refresh };
}
function statusText(result: IntegrityResult | undefined) {
  return result?.status === "valid" ? t("Archive integrity verified.")
    : result?.status === "invalid" ? t("Archive integrity check failed. The file may be damaged.")
    : result?.status === "unavailable" ? t("Verification unavailable. The archive could not be read; it has not been marked damaged.")
    : t("Archive integrity unknown.");
}
export function IntegrityBadge({ book, snapshot }: { book: Book; snapshot: IntegritySnapshot | null }) {
  const key = identityKey(book.id, book.installationId);
  const result = snapshot?.job.results.find((item) => identityKey(item.bookId, item.installationId) === key) ?? snapshot?.books.find((item) => identityKey(item.bookId, item.installationId) === key);
  return <span className={`badge ${result?.status === "invalid" ? "warning" : ""}`} title={statusText(result)}>
    {result?.status === "valid" ? t("Integrity verified") : result?.status === "invalid" ? t("Integrity failed") : t("Integrity unknown")}
  </span>;
}
interface Props {
  book?: Book;
  books: Book[];
  snapshot: IntegritySnapshot | null;
  refresh: () => Promise<void>;
  privateMode: boolean;
  readOnly: boolean;
  onError: (error: unknown) => void;
}
export default function IntegrityPanel({ book, books, snapshot, refresh, privateMode, readOnly, onError }: Props) {
  const [submitting, setSubmitting] = useState(false);
  const [confirmation, setConfirmation] = useState<{ book: Book; privacy: PrivacyContext } | null>(null);
  const [report, setReport] = useState<{ text: string; privacy: PrivacyContext } | null>(null);
  // Verification results and library titles are resolved by identity, once per
  // snapshot, instead of scanning both lists for every rendered row.
  const localBooks = useMemo(() => {
    const index = new globalThis.Map<string, Book>();
    for (const item of books) {
      const key = identityKey(item.id, item.installationId);
      if (!index.has(key)) index.set(key, item);
    }
    return index;
  }, [books]);
  const resultIndex = useMemo(() => {
    const run = new globalThis.Map<string, IntegrityResult>();
    for (const item of snapshot?.job.results || []) {
      const key = identityKey(item.bookId, item.installationId);
      if (!run.has(key)) run.set(key, item);
    }
    // A run result outranks the persisted library result for the same archive.
    const merged = new globalThis.Map(run);
    for (const item of snapshot?.books || []) {
      const key = identityKey(item.bookId, item.installationId);
      if (!merged.has(key)) merged.set(key, item);
    }
    return { run, merged };
  }, [snapshot]);
  const cancelConfirmation = useRef<HTMLButtonElement>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    setConfirmation(null);
    setReport(null);
  }, [book?.id, book?.installationId, privateMode, readOnly]);
  useEffect(() => {
    if (confirmation) cancelConfirmation.current?.focus({ preventScroll: true });
  }, [confirmation]);
  async function start(target?: Book, privacy = capturePrivacyContext()) {
    if (!privacy || !isPrivacyContextCurrent(privacy)) return;
    setConfirmation(null);
    setReport(null);
    setSubmitting(true);
    try {
      await api(target ? "check_integrity" : "diagnostics_integrity", target ? { id: target.id, installationId: target.installationId, privacyGeneration: privacy.generation } : { privacyGeneration: privacy.generation });
      if (alive.current && isPrivacyContextCurrent(privacy)) await refresh();
    } catch (error) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(error);
    } finally { if (alive.current) setSubmitting(false); }
  }
  function requestStart() {
    const privacy = capturePrivacyContext();
    if (!privacy) return;
    if (book && book.size >= 100 * 1024 * 1024) setConfirmation({ book, privacy });
    else void start(book, privacy);
  }
  async function cancel() {
    const privacy = capturePrivacyContext();
    if (!privacy) return;
    setSubmitting(true);
    try {
      await api("cancel_integrity", { privacyGeneration: privacy.generation });
      if (alive.current && isPrivacyContextCurrent(privacy)) await refresh();
    } catch (error) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(error);
    } finally { if (alive.current) setSubmitting(false); }
  }
  async function reviewReport() {
    const privacy = capturePrivacyContext();
    if (!privacy) return;
    setSubmitting(true);
    try {
      const value = await api<unknown>("diagnostics", { privacyGeneration: privacy.generation });
      if (alive.current && isPrivacyContextCurrent(privacy)) setReport({ text: JSON.stringify(value, null, 2), privacy });
    } catch (error) {
      if (alive.current && isPrivacyContextCurrent(privacy)) onError(error);
    } finally { if (alive.current) setSubmitting(false); }
  }
  async function exportReport() {
    if (readOnly || !report || !isPrivacyContextCurrent(report.privacy)) return;
    setSubmitting(true);
    try {
      await saveText(`kiwix_diagnostic_${new Date().toISOString().slice(0, 10)}.json`, report.text, report.privacy);
    } catch (error) {
      if (alive.current && isPrivacyContextCurrent(report.privacy)) onError(error);
    } finally { if (alive.current) setSubmitting(false); }
  }
  const job = snapshot?.job;
  const result = book ? resultIndex.merged.get(identityKey(book.id, book.installationId)) : undefined;
  const reviewed = report && isPrivacyContextCurrent(report.privacy) ? report : null;
  return (
    <section aria-label={t("ZIM archive integrity")}>
      {confirmation ? <div role="alertdialog" aria-modal="false" aria-label={t("Verify large archive?")}>
        <p>{t("This archive is at least 100 MiB. Checking it may take a while. You can keep reading and cancel verification at any time.")}</p>
        <div className="button-row">
          <button ref={cancelConfirmation} type="button" onClick={() => setConfirmation(null)}>{t("Cancel")}</button>
          <button type="button" disabled={submitting || job?.running} onClick={() => void start(confirmation.book, confirmation.privacy)}>{t("Verify archive")}</button>
        </div>
      </div> : <div className="button-row">
        <button type="button" disabled={!snapshot || submitting || job?.running || (book && !book.available)} onClick={requestStart}>
          {book ? result?.checkedAt ? t("Recheck archive") : t("Verify archive") : t("Verify available archives")}
        </button>
        {job?.running && <button type="button" disabled={submitting} onClick={() => void cancel()}>{t("Cancel verification")}</button>}
        {!book && <button type="button" disabled={!snapshot || submitting || job?.running} onClick={() => void reviewReport()}>{t("Review diagnostic report")}</button>}
      </div>}
      <p className="muted">{t("Checks archive bytes and structure, not publisher authenticity.")}</p>
      {(privateMode || readOnly) && <p className="muted">{t("Results are temporary in private or read-only mode.")}</p>}
      {job?.phase === "complete" && job.total === 0 && <p role="status">{t("No available archives to verify.")}</p>}
      {job && job.total > 0 && <div role="status" aria-live="polite">
        <p>{job.running ? t("Checking…") : job.phase === "cancelled" ? t("Verification cancelled.") : t("Verification complete.")} {t("{checked} of {total} checked", { checked: job.completed, total: job.total })}</p>
        {job.running && <>
          <progress max={job.totalBytes || 1} value={job.bytesChecked} aria-label={t("Archive verification progress")} />
          <p className="muted">{job.phase === "structure" ? t("Checking archive structure…") : job.phase === "opening" ? t("Opening archive…") : t("{checked} of {total} checked", { checked: formatBytes(job.bytesChecked), total: formatBytes(job.totalBytes) })}</p>
        </>}
      </div>}
      {!book && job && job.targets.length > 0 && <div>
      <h3>{t("Current verification run")}</h3>
      <ol aria-label={t("Archive verification results")}>
        {job.targets.map((target, index) => {
          const key = identityKey(target.bookId, target.installationId);
          const local = localBooks.get(key);
          const measured = resultIndex.run.get(key);
          const current = job.running && job.bookId === target.bookId;
          return <li key={`${target.bookId}:${target.installationId}`}>
            <strong>{t("Archive {number}", { number: formatNumber(index + 1) })}: {local?.title ?? t("Archive no longer in library")}</strong>
            <p className={measured?.status === "invalid" ? "inline-error" : "muted"}>
              {measured ? statusText(measured) : current ? t("Checking…") : job.running ? t("Queued") : t("Not checked")}
              {measured?.checkedAt && <> {t("Last checked: {date}", { date: formatDate(measured.checkedAt) })} {!measured.persisted && t("Temporary result")}</>}
            </p>
          </li>;
        })}
      </ol></div>}
      {!book && snapshot && snapshot.books.length > 0 && <div>
        <h3>{t("Library integrity results")}</h3>
        <ol aria-label={t("Library integrity results")}>
          {snapshot.books.map((stored, index) => {
            const local = localBooks.get(identityKey(stored.bookId, stored.installationId));
            return <li key={`${stored.bookId}:${stored.installationId}`}>
              <strong>{t("Archive {number}", { number: formatNumber(index + 1) })}: {local?.title ?? t("Archive no longer in library")}</strong>
              <p className={stored.status === "invalid" ? "inline-error" : "muted"}>
                {statusText(stored)}
                {stored.checkedAt && <> {t("Last checked: {date}", { date: formatDate(stored.checkedAt) })}</>}
              </p>
            </li>;
          })}
        </ol>
      </div>}
      {book && <p role="status" className={result?.status === "invalid" ? "inline-error" : result?.status === "valid" ? "success-text" : "muted"}>
        {statusText(result)}
        {result?.checkedAt && <> {t("Last checked: {date}", { date: formatDate(result.checkedAt) })} {!result.persisted && t("Temporary result")}</>}
      </p>}
      {!book && reviewed && <div>
        <h3>{t("Diagnostic report preview")}</h3>
        <p className="muted">{t("Review before sharing. Run-order archive numbers match the current verification run; library-order numbers match library integrity results. The report includes measured integrity results and local events, but no archive names, paths, private identifiers or access keys.")}</p>
        {readOnly && <p className="muted">{t("Saving or sharing reports is disabled in read-only mode. You can still review the report here.")}</p>}
        <label className="field">{t("Diagnostic report JSON")}<textarea readOnly rows={12} value={reviewed.text} spellCheck={false} /></label>
        <div className="button-row">
          <button type="button" disabled={submitting || readOnly} onClick={() => void exportReport()}>{isMobile ? t("Save or share diagnostic report") : t("Save diagnostic report")}</button>
          <button type="button" disabled={submitting} onClick={() => setReport(null)}>{t("Close preview")}</button>
        </div>
      </div>}
    </section>
  );
}
