import { useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Check,
  FileUp,
  FolderOpen,
  RefreshCw,
} from "lucide-react";
import { api, capturePrivacyContext, chooseDirectory, editionDownload, isIOS, isMobile, isPrivacyContextCurrent, isPrivacyPolicyContextCurrent, readTextFile, type PrivacyContext } from "../api";
import { t, formatBytes, getLocale } from "../i18n";
import { displayPath } from "../display-path";
import {
  queueEditionUpdates,
  selectBulkEditionUpdates,
  type EditionUpdateQueueProgress,
} from "../edition-update-queue";
import type {
  Book,
  ContentUpdates,
  DownloadAdmission,
  DownloadTarget,
  EditionPreparation,
  ImportResult,
  Settings,
} from "../types";
import { errorText, Modal, pathFileName } from "./UI";
import IntegrityPanel, { useIntegritySnapshot } from "./IntegrityPanel";
import { CrashReportSection } from "./CrashReport";
import { useSettingsAutosave } from "./SettingsAutosave";
import { AgentConnectorSettings } from "./AgentConnectorSettings";
interface Props {
  settings: Settings;
  books: Book[];
  onError: (error: unknown) => void;
  onRefresh: () => void;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
}
export default function AdvancedSettings({
  settings,
  books,
  onError,
  onRefresh,
  onDownload,
}: Props) {
  const integrity = useIntegritySnapshot(settings.privateMode, settings.readOnly, onError);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [migration, setMigration] = useState<{
    text: string; baseDirectory: string; privacy: PrivacyContext;
  } | null>(null);
  const [importing, setImporting] = useState(false);
  const [updates, setUpdates] = useState<ContentUpdates | null>(null);
  const [integrityOpen, setIntegrityOpen] = useState(false);
  const [updatesOpen, setUpdatesOpen] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const editionRequest = useRef(0);
  const editionAlive = useRef(true);
  const [editionError, setEditionError] = useState("");
  const [associations, setAssociations] = useState<Record<string, boolean>>({});
  const [editionDecision, setEditionDecision] = useState<{ preparation: EditionPreparation; privacy: PrivacyContext } | null>(null);
  const editionPending = useRef(false);
  const [bulkConfirm, setBulkConfirm] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<EditionUpdateQueueProgress | null>(null);
  const [bulkFailures, setBulkFailures] = useState<{ title: string; message: string }[]>([]);
  const [bulkError, setBulkError] = useState("");
  const [bulkActive, setBulkActive] = useState(false);
  const [bulkStopping, setBulkStopping] = useState(false);
  const bulkCancel = useRef(false);
  const bulkRunning = useRef(false);
  const bulkRequest = useRef(0);
  useEffect(() => {
    editionAlive.current = true;
    return () => { editionAlive.current = false; editionRequest.current++; };
  }, []);
  const catalogSources = JSON.stringify(settings.opdsSources);
  useEffect(() => {
    editionRequest.current++;
    bulkRequest.current++;
    bulkCancel.current = true;
    bulkRunning.current = false;
    setBusy(false);
    setUpdates(null);
    setEditionError("");
    setAssociations({});
    setEditionDecision(null);
    setCheckingUpdates(false);
    setBulkConfirm(false);
    setBulkProgress(null);
    setBulkFailures([]);
    setBulkError("");
    setBulkActive(false);
    setBulkStopping(false);
  }, [settings.privateMode, settings.readOnly, settings.catalogUrl, catalogSources]);
  useEffect(() => { setMigration(null); }, [settings.privateMode, settings.readOnly]);
  const integrityRunning = integrity.snapshot?.job.running ?? false;
  const integrityIssues = [
    ...(integrity.snapshot?.job.results ?? []),
    ...(integrity.snapshot?.books ?? []),
  ].filter((result) => result.status === "invalid" || result.status === "unavailable");
  const integrityIssueKey = JSON.stringify(integrityIssues.map((result) => [
    result.bookId, result.installationId, result.status, result.checkedAt,
  ]));
  useEffect(() => {
    if (integrityRunning) setIntegrityOpen(true);
  }, [integrityRunning]);
  useEffect(() => {
    if (integrityIssueKey !== "[]") setIntegrityOpen(true);
  }, [integrityIssueKey]);
  const { draft, change, input, flush } = useSettingsAutosave(settings, {
    onRefresh,
    onError,
  });
  async function migrate() {
    const current = capturePrivacyContext();
    if (!migration || importing || !current || current.readOnly ||
        !isPrivacyPolicyContextCurrent(migration.privacy)) return;
    setImporting(true);
    try {
      const result = await api<ImportResult>("migrate_library", {
        text: migration.text,
        baseDirectory: migration.baseDirectory || undefined,
        privacyGeneration: current.generation,
      });
      if (!editionAlive.current || !isPrivacyPolicyContextCurrent(migration.privacy)) return;
      setMigration(null);
      setNotice(
        t(
          "Imported {count} library archives. Original files and missing-file references were preserved.",
          { count: result.books.length },
        ),
      );
      onRefresh();
      if (result.errors.length)
        onError(
          result.errors.map((item) => `${displayPath(item.path)}: ${item.error}`).join("\n"),
        );
    } catch (e) {
      if (editionAlive.current && isPrivacyPolicyContextCurrent(migration.privacy)) onError(e);
    } finally {
      setImporting(false);
    }
  }
  async function findContentUpdates() {
    const privacy = capturePrivacyContext();
    if (!privacy || checkingUpdates) return;
    const request = ++editionRequest.current;
    const current = () => editionAlive.current && request === editionRequest.current &&
      isPrivacyPolicyContextCurrent(privacy);
    setCheckingUpdates(true);
    setEditionError("");
    setUpdatesOpen(true);
    try {
      const result = await api<ContentUpdates>("find_updates", {
        locale: getLocale(),
        privacyGeneration: privacy.generation,
      });
      if (current()) setUpdates(result);
    } catch (e) {
      if (current()) setEditionError(errorText(e));
    } finally {
      if (current()) setCheckingUpdates(false);
    }
  }
  async function prepareEdition(update: ContentUpdates["updates"][number]) {
    const privacy = capturePrivacyContext();
    const key = `${update.bookId}:${update.selection.sourceId}:${update.selection.entryId}`;
    if (editionPending.current || !privacy || privacy.privateMode || privacy.readOnly ||
      settings.privateMode || settings.readOnly ||
      !books.some((book) => book.id === update.bookId && book.installationId === update.installationId) ||
      (update.associationRequired && !associations[key])) return;
    const request = editionRequest.current;
    editionPending.current = true;
    setBusy(true);
    setEditionError("");
    try {
      const preparation = await api<EditionPreparation>("edition_prepare", {
        bookId: update.bookId,
        installationId: update.installationId,
        sourceId: update.selection.sourceId,
        entryId: update.selection.entryId,
        catalogGeneration: update.selection.catalogGeneration,
        associationConfirmed: !!associations[key],
        privacyGeneration: privacy.generation,
      });
      if (editionAlive.current && request === editionRequest.current && isPrivacyContextCurrent(privacy)) {
        setEditionDecision({ preparation, privacy });
      }
    } catch (error) {
      if (editionAlive.current && request === editionRequest.current && isPrivacyContextCurrent(privacy)) setEditionError(errorText(error));
    } finally {
      editionPending.current = false;
      if (editionAlive.current) setBusy(false);
    }
  }
  async function startEdition() {
    const decision = editionDecision;
    if (editionPending.current || !decision || !isPrivacyContextCurrent(decision.privacy) ||
      settings.privateMode || settings.readOnly ||
      !books.some((book) => book.id === decision.preparation.installed.id &&
        book.installationId === decision.preparation.installed.installationId)) return;
    editionPending.current = true;
    setBusy(true);
    setEditionError("");
    try {
      const target = await editionDownload(decision.preparation.token);
      const present = editionAlive.current && isPrivacyContextCurrent(decision.privacy);
      onDownload(target, { privacy: decision.privacy, focus: present });
      if (present) {
        setEditionDecision(null);
        setNotice(t("Full edition download started. Downloads will ask you to Replace only after verification. Your original archive is kept."));
      }
    } catch (error) {
      if (editionAlive.current && isPrivacyContextCurrent(decision.privacy)) setEditionError(errorText(error));
    } finally {
      editionPending.current = false;
      if (editionAlive.current) setBusy(false);
    }
  }
  async function startBulkUpdates() {
    const privacy = capturePrivacyContext();
    if (bulkRunning.current || !privacy || privacy.privateMode || privacy.readOnly ||
      settings.privateMode || settings.readOnly) return;
    const candidates = updates ? selectBulkEditionUpdates(updates.updates, books) : [];
    if (!candidates.length) return;
    const request = editionRequest.current;
    const alive = () => editionAlive.current && request === editionRequest.current;
    const current = () => alive() && !bulkCancel.current && isPrivacyContextCurrent(privacy);
    bulkRunning.current = true;
    bulkRequest.current = request;
    bulkCancel.current = false;
    const startedKeys = new Set<string>();
    setBulkConfirm(false);
    setBusy(true);
    setBulkActive(true);
    setBulkStopping(false);
    setBulkError("");
    setBulkFailures([]);
    setBulkProgress({
      total: candidates.length,
      processed: 0,
      started: 0,
      failed: 0,
      skipped: 0,
      stopped: false,
      failures: [],
    });
    try {
      const result = await queueEditionUpdates(candidates, {
        isCurrent: current,
        concurrency: 2,
        prepare: (candidate) => api<EditionPreparation>("edition_prepare", {
          bookId: candidate.bookId,
          installationId: candidate.installationId,
          sourceId: candidate.selection.sourceId,
          entryId: candidate.selection.entryId,
          catalogGeneration: candidate.selection.catalogGeneration,
          associationConfirmed: false,
          privacyGeneration: privacy.generation,
        }),
        start: (token) => editionDownload(token),
        onStarted: (candidate, target) => {
          startedKeys.add(`${candidate.bookId}\u0000${candidate.installationId}`);
          // An accepted transfer belongs to Downloads even if this panel retires.
          if (alive() && isPrivacyContextCurrent(privacy)) {
            onDownload(target, { privacy, focus: false });
          }
        },
        onProgress: (progress) => {
          if (alive()) setBulkProgress(progress);
        },
      });
      if (!alive()) return;
      setBulkProgress(result);
      if (startedKeys.size) {
        setUpdates((previous) => previous && ({
          ...previous,
          updates: previous.updates.filter((update) =>
            !startedKeys.has(`${update.bookId}\u0000${update.installationId}`)),
        }));
      }
      if (result.started || result.skipped) {
        setNotice([
          result.started
            ? t("Started {count} edition downloads.", { count: result.started })
            : "",
          result.skipped
            ? t("{count} were skipped after the queue stopped.", { count: result.skipped })
            : "",
          result.started
            ? t("Downloads will ask you to Replace only after verification; your original archives are kept.")
            : "",
        ].filter(Boolean).join(" "));
      }
      if (result.failures.length) {
        setBulkFailures(result.failures.map((failure) => ({
          title: failure.title,
          message: errorText(failure.error),
        })));
      }
    } catch (error) {
      if (alive()) setBulkError(errorText(error));
    } finally {
      if (bulkRequest.current === request) {
        bulkRunning.current = false;
        if (editionAlive.current) {
          setBusy(false);
          setBulkActive(false);
          setBulkStopping(false);
        }
      }
    }
  }
  const bulkCandidates = updates ? selectBulkEditionUpdates(updates.updates, books) : [];
  const bulkSize = bulkCandidates.reduce(
    (total, candidate) => total + Math.max(0, candidate.size),
    0,
  );
  const editionDecisionCurrent = !editionDecision || (isPrivacyContextCurrent(editionDecision.privacy) &&
    books.some((book) => book.id === editionDecision.preparation.installed.id &&
      book.installationId === editionDecision.preparation.installed.installationId));
  return (
    <div className="advanced-settings">
      <form className="settings-section panel" onSubmit={(event) => { event.preventDefault(); flush(); }}>
        <h2>{t("Privacy, transfer & advanced preferences")}</h2>
        <div className="form-grid">
          <div>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.privateMode}
                onChange={(e) =>
                  change("privateMode", e.target.checked)
                }
              />
              <span>{t("Private reading mode")}</span>
            </label>
            <p className="muted">
              {t(
                "Do not persist reading history, positions or sessions. Existing history is not removed.",
              )}
            </p>
          </div>
          <div>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.readOnly}
                onChange={(e) =>
                  change("readOnly", e.target.checked)
                }
              />
              <span>{t("Strict read-only mode")}</span>
            </label>
            <p className="muted">
              {t(
                "Prevent library, download and reading-data changes. Private mode and read-only mode are separate controls.",
              )}
            </p>
          </div>
          <AgentConnectorSettings
            enabled={draft.agentConnector}
            privateMode={settings.privateMode}
            readOnly={settings.readOnly}
            onChange={(enabled) => change("agentConnector", enabled)}
          />
        </div>
        <details className="secondary-controls" onInvalidCapture={(event) => { event.currentTarget.open = true; }}>
          <summary>{t("Advanced preferences")}</summary>
          <div className="form-grid">
          <label className="field">
            {t("Download bandwidth limit (bytes/s)")}
            <input
              type="number"
              min={0}
              step={16384}
              {...input("downloadLimit")}
            />
            <span>
              {t("0 means unlimited; otherwise use at least 16384 bytes/s.")}
            </span>
          </label>
          <label className="field">
            {t("Upload bandwidth limit (bytes/s)")}
            <input
              type="number"
              min={0}
              step={16384}
              {...input("uploadLimit")}
            />
            <span>
              {t("0 means unlimited; otherwise use at least 16384 bytes/s.")}
            </span>
          </label>
        </div>
        <label className="check-field">
          <input
            type="checkbox"
            checked={draft.autoResumeDownloads}
            onChange={(e) =>
              change("autoResumeDownloads", e.target.checked)
            }
          />
          <span>
            {t("Automatically resume unfinished downloads at startup")}
          </span>
        </label>
        <label className="field">
          {t("Additional OPDS catalog sources, one URL per line")}
          <textarea
            {...input("opdsSources")}
            rows={3}
            placeholder={t("https://your-catalog.example/entries")}
          />
        </label>
        <label className="check-field">
          <input
            type="checkbox"
            checked={draft.diagnosticsEnabled}
            onChange={(e) =>
              change("diagnosticsEnabled", e.target.checked)
            }
          />
          <span>{t("Enable local diagnostic logging")}</span>
        </label>
        <p className="muted">
          {t("Diagnostic logs stay on this device. No telemetry is sent.")}
        </p>
        <label className="field">
          {t("Diagnostic detail")}
          <select
            value={draft.logLevel}
            onChange={(e) => change("logLevel", e.target.value)}
          >
            <option value="error">{t("Errors only")}</option>
            <option value="warn">{t("Warnings")}</option>
            <option value="info">{t("Information")}</option>
            <option value="debug">{t("Debug")}</option>
          </select>
        </label>
        </details>
      </form>
      <details
        className="settings-section panel secondary-controls"
        open={integrityOpen || integrityRunning}
        onToggle={(event) => {
          if (integrityRunning) event.currentTarget.open = true;
          setIntegrityOpen(event.currentTarget.open);
        }}
      >
        <summary onClick={(event) => { if (integrityRunning) event.preventDefault(); }}>
          {t("ZIM archive integrity")}
          {integrityRunning
            ? <> · {t("Checking…")}</>
            : integrityIssues.length > 0 && (
              <span className="warning-text"> · {integrityIssues.some((result) => result.status === "invalid")
                ? t("Integrity failed")
                : t("Verification unavailable")}</span>
            )}
        </summary>
        <IntegrityPanel books={books} snapshot={integrity.snapshot} refresh={integrity.refresh} privateMode={settings.privateMode} readOnly={settings.readOnly} onError={onError} />
      </details>
      <details className="settings-section panel secondary-controls">
        <summary>{t("Crash reports")}</summary>
        <CrashReportSection
          books={books}
          readOnly={settings.readOnly}
          onError={onError}
        />
      </details>
      <details className="settings-section panel secondary-controls">
        <summary>{t("Import library XML")}</summary>
        <p className="muted">
          {t(
            "Import an existing Kiwix library XML file. Archives are referenced in place, and missing files remain visible so they can be relinked.",
          )}
        </p>
        <button
          disabled={busy || importing || settings.readOnly}
          onClick={async () => {
            const privacy = capturePrivacyContext();
            if (!privacy || privacy.readOnly) return;
            setImporting(true);
            try {
              const text = await readTextFile(privacy);
              if (editionAlive.current && text !== null && isPrivacyPolicyContextCurrent(privacy))
                setMigration({ text, baseDirectory: "", privacy });
            } catch (e) {
              if (editionAlive.current && isPrivacyPolicyContextCurrent(privacy)) onError(e);
            } finally {
              setImporting(false);
            }
          }}
        >
          <FileUp size={16} />
          {t("Import library XML")}
        </button>
      </details>
      <details
        className="settings-section panel secondary-controls"
        open={updatesOpen || checkingUpdates || bulkActive}
        aria-busy={checkingUpdates || bulkActive}
        onToggle={(event) => {
          if (checkingUpdates || bulkActive) event.currentTarget.open = true;
          setUpdatesOpen(event.currentTarget.open);
        }}
      >
        <summary onClick={(event) => { if (checkingUpdates || bulkActive) event.preventDefault(); }}>
          {t("Archive editions")}
          {checkingUpdates
            ? <> · {t("Checking…")}</>
            : updates?.errors.length
              ? <span className="warning-text"> · {t("Review update errors")}</span>
              : updates
                ? <> · {t("{count} edition choices", { count: updates.updates.length })}</>
                : null}
        </summary>
        <p className="muted">
          {t(
            "Check all configured catalogs only when you choose. Editions are full archive downloads, not small patches. Your original archive is kept; replacement always needs your explicit approval after verification.",
          )}
        </p>
        <div className="toolbar">
          <button
            disabled={busy || checkingUpdates || !books.length}
            onClick={findContentUpdates}
          >
            <RefreshCw size={16} />
            {t("Find newer archive editions")}
          </button>
          {bulkCandidates.length > 0 && (
            <button
              disabled={busy || checkingUpdates || settings.privateMode || settings.readOnly}
              onClick={() => setBulkConfirm(true)}
            >
              <ArrowDownToLine size={16} />
              {t("Update all {count} editions", { count: bulkCandidates.length })}
            </button>
          )}
        </div>
        {bulkActive && bulkProgress && (
          <div className="toolbar" role="status" aria-live="polite">
            <p>
              {t("Processed {processed} of {total} edition downloads…", {
                processed: bulkProgress.processed,
                total: bulkProgress.total,
              })}
            </p>
            <button
              disabled={bulkStopping}
              onClick={() => {
                bulkCancel.current = true;
                setBulkStopping(true);
              }}
            >
              {bulkStopping ? t("Stopping…") : t("Stop queueing")}
            </button>
          </div>
        )}
        {editionError && <p className="inline-error" role="alert">{editionError}</p>}
        {bulkError && <p className="inline-error" role="alert">{bulkError}</p>}
        {bulkFailures.map((failure, index) => (
          <p className="inline-error" role="alert" key={`${failure.title}:${index}`}>
            {failure.title}: {failure.message}
          </p>
        ))}
        {updates && (
          <div className="edition-results" aria-live="polite">
            <p>{t("{count} edition choices", { count: updates.updates.length })}</p>
            <ul aria-label={t("Catalog refresh results")}>
              {updates.sources.map((source) => (
                <li key={source.sourceId}>
                  <p className="file-path"><bdi>{source.sourceUrl}</bdi></p>
                  <p className={source.stale || source.completeness === "partial" || source.error ? "warning-text" : "muted"}>
                    {source.stale ? t("Offline or stale metadata — editions may be out of date.")
                      : source.error ? t("Catalog refresh incomplete.")
                        : source.pages.length > 0 && source.pages.every((page) => page.status === "revalidated")
                          ? t("Catalog metadata revalidated.") : t("Catalog metadata refreshed.")}
                    {" "}{source.completeness === "traversed"
                      ? t("All linked pages were traversed; this is not a publisher-verified snapshot.")
                      : t("Partial catalog results — more editions may exist.")}
                    {!source.stored && <>{" "}{t("These results are not saved for offline use.")}</>}
                  </p>
                  {source.error && <p className="inline-error">{errorText(source.error)}</p>}
                </li>
              ))}
            </ul>
            <p className="muted">{t("An edition not seen in a catalog refresh is not proof of deletion. Installed archives are never removed by this check.")}</p>
            {updates.errors.map((error, index) => (
              <p className="inline-error" key={index}>{errorText(error)}</p>
            ))}
            {updates.updates.map((update) => {
              const key = `${update.bookId}:${update.selection.sourceId}:${update.selection.entryId}`;
              const installed = books.find((book) => book.id === update.bookId && book.installationId === update.installationId);
              const source = updates.sources.find((item) => item.sourceId === update.selection.sourceId);
              return (
                <article className="edition-result" key={key}>
                  <div>
                    <h3><bdi>{update.selection.entry.title}</bdi></h3>
                    <p>{t("Installed edition: {title} ({date})", {
                      title: installed?.title ?? update.bookId,
                      date: installed?.date || t("Date unknown"),
                    })}</p>
                    <p className="muted">
                      {t("Catalog edition: {date}", { date: update.selection.entry.date || t("Date unknown") })}
                      {" · "}{t("Full download: {size}", { size: update.selection.entry.size > 0 ? formatBytes(update.selection.entry.size) : t("Size unknown") })}
                    </p>
                    <p className="file-path"><bdi>{source?.sourceUrl ?? update.selection.sourceId}</bdi></p>
                    {update.reason && <p className="warning-text">{update.reason}</p>}
                    {update.associationRequired && (
                      <label className="check-field">
                        <input type="checkbox" checked={!!associations[key]}
                          disabled={busy || settings.privateMode || settings.readOnly || !installed}
                          onChange={(event) => setAssociations((previous) => ({ ...previous, [key]: event.target.checked }))} />
                        <span>{t("I associate this catalog series with this installed archive. Its lineage is unbound or ambiguous; matching titles alone do not prove an update.")}</span>
                      </label>
                    )}
                    {!installed && <p className="warning-text">{t("The installed archive changed. Check for editions again.")}</p>}
                  </div>
                  <button
                    disabled={busy || checkingUpdates || settings.privateMode || settings.readOnly || !installed ||
                      (update.associationRequired && !associations[key])}
                    onClick={() => void prepareEdition(update)}
                  >
                    {t("Prepare full edition download")}
                  </button>
                </article>
              );
            })}
          </div>
        )}
      </details>
      {editionDecision && (
        <Modal title={t("Prepare full archive edition")} onClose={() => { if (!busy) setEditionDecision(null); }}>
          <p>{t("Installed edition: {title} ({date})", {
            title: editionDecision.preparation.installed.title,
            date: editionDecision.preparation.installed.date || t("Date unknown"),
          })}</p>
          <p>{t("New edition: {title} ({date})", {
            title: editionDecision.preparation.selection.entry.title,
            date: editionDecision.preparation.selection.entry.date || t("Date unknown"),
          })}</p>
          <p className="file-path"><bdi>{editionDecision.preparation.selection.sourceUrl}</bdi></p>
          <p><strong>{t("Full download: {size}", {
            size: editionDecision.preparation.selection.entry.size > 0
              ? formatBytes(editionDecision.preparation.selection.entry.size) : t("Size unknown"),
          })}</strong></p>
          <p>{t("Your original archive and its files are retained. The new archive downloads separately and is verified before Downloads offers Replace. Nothing is replaced by starting this download.")}</p>
          {!editionDecisionCurrent && <p className="warning-text" role="status">{t("This preparation expired or the installed archive changed. Cancel and prepare the edition again.")}</p>}
          {editionError && <p className="inline-error" role="alert">{editionError}</p>}
          <div className="modal-actions">
            <button disabled={busy} onClick={() => setEditionDecision(null)}>{t("Cancel")}</button>
            <button className="primary" disabled={busy || settings.privateMode || settings.readOnly || !editionDecisionCurrent}
              onClick={() => void startEdition()}>
              <ArrowDownToLine size={16} aria-hidden="true" />
              {busy ? t("Starting…") : t("Start full download")}
            </button>
          </div>
        </Modal>
      )}
      {bulkConfirm && (
        <Modal title={t("Update all eligible editions")} onClose={() => { if (!busy) setBulkConfirm(false); }}>
          <p>{t("Eligible installed archives: {count}", {
            count: bulkCandidates.length,
          })}</p>
          {updates && updates.updates.length > bulkCandidates.length && (
            <p className="muted">{t("{count} other edition choices need individual review because their lineage is unbound or ambiguous.", {
              count: updates.updates.length - bulkCandidates.length,
            })}</p>
          )}
          <p><strong>{t("Full download: {size}", {
            size: bulkSize > 0 ? formatBytes(bulkSize) : t("Size unknown"),
          })}</strong></p>
          <p>{t("Each archive downloads separately and is verified before Downloads offers Replace. Your original archives and reading state are kept. Editions that need an explicit association are not included; update those individually.")}</p>
          <div className="modal-actions">
            <button disabled={busy} onClick={() => setBulkConfirm(false)}>{t("Cancel")}</button>
            <button className="primary" disabled={busy || !bulkCandidates.length || settings.privateMode || settings.readOnly}
              onClick={() => void startBulkUpdates()}>
              <ArrowDownToLine size={16} aria-hidden="true" />
              {t("Start {count} downloads", { count: bulkCandidates.length })}
            </button>
          </div>
        </Modal>
      )}
      {notice && (
        <p className="success-text" role="status">
          <Check size={16} />
          {notice}
        </p>
      )}
      {migration !== null && isPrivacyPolicyContextCurrent(migration.privacy) && (
        <Modal
          title={t("Import an existing library")}
          onClose={() => { if (!importing) setMigration(null); }}
        >
          <p>
            {isMobile
              ? t("Import a copy of the original library folder to resolve relative archive paths. Original files are unchanged and are not monitored. Archives with old absolute paths may need to be relinked.")
              : t(
                  "Choose the folder that contained the original library XML when its archive paths are relative. Absolute paths are preserved.",
                )}
          </p>
          <label className="field">
            {isMobile ? t("Imported library folder copy (optional)") : t("Original library folder (optional)")}
            <div className="input-action">
              <input
                value={isIOS && migration.baseDirectory ? pathFileName(migration.baseDirectory) : migration.baseDirectory}
                readOnly={isMobile}
                disabled={importing}
                onChange={(e) => setMigration({ ...migration, baseDirectory: e.target.value })}
              />
              <button
                type="button"
                aria-label={isMobile ? t("Import original library folder copy") : t("Choose original library folder")}
                disabled={importing || (isMobile && (settings.privateMode || settings.readOnly))}
                onClick={async () => {
                  const privacy = capturePrivacyContext();
                  if (!privacy) return;
                  try {
                    const path = isMobile
                      ? await api<string | null>("mobile_pick_directory_copy", { privacyGeneration: privacy.generation })
                      : await chooseDirectory();
                    if (path && isPrivacyPolicyContextCurrent(privacy))
                      setMigration((current) => current === migration ? { ...current, baseDirectory: path } : current);
                  } catch (e) {
                    if (editionAlive.current && isPrivacyPolicyContextCurrent(privacy)) onError(e);
                  }
                }}
              >
                <FolderOpen size={16} />
              </button>
            </div>
          </label>
          <div className="modal-actions">
            <button disabled={importing} onClick={() => setMigration(null)}>{t("Cancel")}</button>
            <button className="primary" disabled={importing} onClick={migrate}>
              <FileUp size={16} />
              {importing ? t("Importing…") : t("Import library")}
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
