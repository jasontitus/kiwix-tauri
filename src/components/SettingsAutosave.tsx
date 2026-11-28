import { useEffect, useLayoutEffect, useState, useSyncExternalStore } from "react";
import {
  api,
  getConfirmedSettings,
  isIOS,
  registerSettingsDraftFlusher,
  subscribeReaderPreferences,
} from "../api";
import { t } from "../i18n";
import type { Settings } from "../types";
import { errorText, Modal } from "./UI";

type Key = keyof Settings;
type TextKey = "fontSize" | "zoom" | "downloadLimit" | "uploadLimit" |
  "catalogUrl" | "mapCatalogUrl" | "mapProxyUrl" | "opdsSources" | "downloadDirectory";
type Handlers = {
  onRefresh: () => void;
  onError?: (error: unknown) => void;
  onCommitted?: (key: Key, saved: Settings, previous: Settings) => void;
};
type Edit = {
  value?: Settings[Key];
  raw?: string;
  phase: "waiting" | "confirming" | "saving" | "error" | "invalid";
  consented?: boolean;
  error?: unknown;
  timer?: number;
  handlers: Handlers;
};
// Shell-owned drafts, not component-owned promises: route changes cannot discard
// accepted writes, debounce timers, or the value needed to retry a failed write.
const edits = new Map<Key, Edit>();
// A later invalid draft must not erase the valid failed write needed to unblock
// a lifecycle drain. It remains retryable without discarding the invalid text.
const failedEdits = new Map<Key, Edit>();
const listeners = new Set<() => void>();
let version = 0;
let hasSaved = false;
type DirectoryChange = { from: string; to: string; onRefresh: () => void };
type MigrationResult = {
  moved: { id: string; title: string; from: string; to: string }[];
  errors: { id: string; title: string; error: string }[];
  skipped: number;
};
const directoryChanges: DirectoryChange[] = [];
function publish() {
  version++;
  for (const listener of listeners) listener();
}
subscribeReaderPreferences(publish);
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
function snapshot() { return version; }

function needsConfirmation(key: Key, edit: Edit): boolean {
  if (edit.consented) return false;
  return key === "downloadDirectory" || key === "privateMode" || key === "readOnly";
}
function commit(key: Key, edit: Edit, retryRetained = false) {
  clearTimeout(edit.timer);
  edit.timer = undefined;
  if ((!retryRetained && edits.get(key) !== edit) ||
      edit.phase === "saving" || edit.phase === "invalid") return;
  if (needsConfirmation(key, edit)) {
    edit.phase = "confirming";
    publish();
    return;
  }
  const previous = getConfirmedSettings();
  edit.phase = "saving";
  publish();
  void api<Settings>("save_settings", { settings: { [key]: edit.value } }).then(
    (saved) => {
      failedEdits.delete(key);
      if (key === "downloadDirectory" && previous &&
          saved.downloadDirectory !== previous.downloadDirectory) {
        directoryChanges.push({
          from: previous.downloadDirectory,
          to: saved.downloadDirectory,
          onRefresh: edit.handlers.onRefresh,
        });
      }
      if (edits.get(key) === edit) {
        edits.delete(key);
        hasSaved = true;
        if (previous) edit.handlers.onCommitted?.(key, saved, previous);
      }
      publish();
      edit.handlers.onRefresh();
    },
    (error: unknown) => {
      edit.phase = "error";
      edit.error = error;
      failedEdits.set(key, edit);
      publish();
      edit.handlers.onError?.(error);
    },
  );
}
// Whether the App-level prompt can be shown. While it cannot (Quit, window
// close, suspension) a typed change that needs review is neither applied nor
// thrown away: it stays in its field, marked as not saved.
let confirmationAvailable = false;
function awaitsReview(key: Key, edit: Edit): boolean {
  return edit.phase === "waiting" && needsConfirmation(key, edit);
}
function flushDrafts() {
  for (const [key, edit] of edits)
    if (edit.phase === "waiting" && (confirmationAvailable || !needsConfirmation(key, edit)))
      commit(key, edit);
}
// Explicit recovery retries the retained writes before lifecycle reconciliation
// drains the same settings queue; the failed-network-policy fence stays intact.
export function retryFailedSettings(): void {
  for (const [key, failed] of failedEdits) {
    const current = edits.get(key);
    if (current?.phase === "saving") continue;
    if (current?.phase === "waiting") commit(key, current);
    else commit(key, failed, true);
  }
}
registerSettingsDraftFlusher(flushDrafts);

function parseText(key: TextKey, raw: string): Settings[Key] {
  const text = raw.trim();
  if (key === "downloadDirectory") return text;
  if (key === "fontSize" || key === "zoom" || key === "downloadLimit" || key === "uploadLimit") {
    const value = Number(text);
    const valid = text !== "" && Number.isFinite(value) && (
      key === "fontSize" ? value >= 10 && value <= 48 :
      key === "zoom" ? value >= 25 && value <= 500 :
      Number.isSafeInteger(value) && (value === 0 || value >= 16384)
    );
    if (!valid) throw new Error(key === "fontSize" ? t("Enter a font size from 10 to 48.") :
      key === "zoom" ? t("Enter a zoom percentage from 25 to 500.") :
      t("Enter 0 or a whole number of at least 16384 bytes/s."));
    return key === "zoom" ? value / 100 : value;
  }
  const urls = key === "opdsSources" ? [...new Set(text.split("\n").map((url) => url.trim()).filter(Boolean))] : [text];
  for (const value of urls) {
    try {
      const url = new URL(value);
      if (!["https:", "http:"].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error();
    } catch {
      throw new Error(t("Enter a complete HTTP or HTTPS URL without embedded credentials."));
    }
  }
  return key === "opdsSources" ? urls : text;
}

export function useSettingsAutosave(settings: Settings, handlers: Handlers) {
  useSyncExternalStore(subscribe, snapshot);
  useEffect(() => () => { flushDrafts(); }, []);
  const confirmed = getConfirmedSettings() ?? settings;
  const draft = { ...confirmed };
  for (const [key, edit] of edits)
    if (edit.value !== undefined) Object.assign(draft, { [key]: edit.value });
  function change<K extends Key>(key: K, value: Settings[K]) {
    clearTimeout(edits.get(key)?.timer);
    const edit: Edit = { value, phase: "waiting", handlers };
    edits.set(key, edit);
    commit(key, edit);
  }
  function input(key: TextKey) {
    const value = edits.get(key)?.raw ?? (key === "zoom" ? String(Math.round(draft.zoom * 100)) :
      key === "opdsSources" ? draft.opdsSources.join("\n") : String(draft[key]));
    return {
      value,
      "aria-invalid": edits.get(key)?.phase === "invalid" || undefined,
      onChange: (event: { target: { value: string } }) => {
        clearTimeout(edits.get(key)?.timer);
        const edit: Edit = { raw: event.target.value, phase: "waiting", handlers };
        try { edit.value = parseText(key, edit.raw!); }
        catch (error) { edit.phase = "invalid"; edit.error = error; }
        edits.set(key, edit);
        // A change that asks for confirmation is committed only when the
        // field is left or the form is submitted. A pause while typing is not
        // a finished value: the prompt would open over half a path and take
        // the next keystroke.
        if (edit.phase === "waiting" && !needsConfirmation(key, edit))
          edit.timer = window.setTimeout(() => commit(key, edit), 600);
        publish();
      },
      onBlur: () => {
        const edit = edits.get(key);
        if (edit?.phase === "waiting") commit(key, edit);
      },
      // Enter reviews a change that is waiting for confirmation. The form has
      // no submit control, so the key would otherwise do nothing here.
      onKeyDown: (event: { key: string; isComposing?: boolean; nativeEvent?: { isComposing?: boolean }; preventDefault: () => void }) => {
        const edit = edits.get(key);
        if (event.key !== "Enter" || event.isComposing || event.nativeEvent?.isComposing ||
            !edit || !awaitsReview(key, edit)) return;
        event.preventDefault();
        commit(key, edit);
      },
    };
  }
  return { draft, change, input, flush: flushDrafts };
}

function preferenceLabel(key: Key): string {
  switch (key) {
    case "theme": return t("Application theme");
    case "locale": return t("Interface language");
    case "readerTheme": return t("Reader appearance");
    case "fontFamily": return t("Reader font");
    case "fontSize": return t("Font size");
    case "zoom": return t("Page zoom (%)");
    case "expandSections": return t("Automatically expand article sections");
    case "linkPreviews": return t("Show article previews for links");
    case "restoreSession": return t("Restore open tabs when the app starts");
    case "openNewTabInBackground": return t("Open links in a background tab");
    case "historyEnabled": return t("Remember reading history and position");
    case "spellingEnabled": return t("Offer spelling suggestions for searches (opt-in)");
    case "externalLinks": return t("External links");
    case "completionNotifications": return t("Notify when an archive is verified and added to the library");
    case "allowMeteredDownloads": return t("Allow cellular and metered downloads");
    case "backupArchives": return t("Include app-owned archives in iOS backups");
    case "downloadDirectory": return t("Download directory");
    case "trashDeletedFiles": return t("Move deleted archive files to Trash instead of deleting permanently");
    case "monitorDirectories": return t("Monitored folders");
    case "catalogUrl": return t("Archive catalog URL");
    case "mapCatalogUrl": return t("StreetZim catalog URL");
    case "mapProxyUrl": return t("Map proxy URL");
    case "showTechnicalErrors": return t("Show technical error details");
    case "privateMode": return t("Private reading mode");
    case "readOnly": return t("Strict read-only mode");
    case "agentConnector": return t("Agent connector");
    case "downloadLimit": return t("Download bandwidth limit (bytes/s)");
    case "uploadLimit": return t("Upload bandwidth limit (bytes/s)");
    case "autoResumeDownloads": return t("Automatically resume unfinished downloads at startup");
    case "opdsSources": return t("Additional OPDS catalog sources, one URL per line");
    case "diagnosticsEnabled": return t("Enable local diagnostic logging");
    case "logLevel": return t("Diagnostic detail");
    case "sharingName": return t("Device name");
    default: return t("Settings");
  }
}

export function SettingsSaveStatus() {
  useSyncExternalStore(subscribe, snapshot);
  const failuresByKey = new Map([...failedEdits].filter(([, edit]) => edit.phase === "error"));
  for (const [key, edit] of edits)
    if (edit.phase === "invalid" || edit.phase === "error") failuresByKey.set(key, edit);
  const failures = [...failuresByKey];
  // A typed change that needs review is not being saved: nothing is sent
  // until the person leaves the field (or presses Enter) and confirms.
  const pending = [...edits, ...failedEdits]
    .some(([key, edit]) => edit.phase === "saving" || (edit.phase === "waiting" && !needsConfirmation(key, edit)));
  const confirming = [...edits.values()].some((edit) => edit.phase === "confirming");
  const unreviewed = [...edits].some(([key, edit]) => awaitsReview(key, edit));
  const state = pending ? "saving" : failures.length ? "error" : confirming || unreviewed ? "confirm" : hasSaved ? "saved" : "auto";
  return <div className="settings-save">
    <span role="status" data-state={state}>{pending ? t("Saving…") : failures.length ? t("Some preferences have not been saved.") :
      confirming ? t("Review the preference change to save it.") :
      unreviewed ? t("This change is not saved yet. Leave the field or press Enter to review it.") :
      hasSaved ? t("Preferences saved") : t("Changes are saved automatically.")}</span>
    {failures.length > 0 && <div role="alert">
      {failures.map(([key, edit]) => <p key={key}>{preferenceLabel(key)}: {errorText(edit.error)}</p>)}
      {failedEdits.size > 0 &&
        <button type="button" onClick={retryFailedSettings}>{t("Retry saving")}</button>}
    </div>}
  </div>;
}

// Mount in App, not Settings: navigation or a delayed flush must never bypass consent.
export function SettingsPolicyConfirmation({ available }: { available: boolean }) {
  const revision = useSyncExternalStore(subscribe, snapshot);
  const pending = [...edits.entries()].find(([, edit]) => edit.phase === "confirming");
  // Synchronous with the commit that hides the shell, so a flush issued by
  // the same Quit or close already sees the prompt as unavailable.
  useLayoutEffect(() => {
    confirmationAvailable = available;
    return () => { confirmationAvailable = false; };
  }, [available]);
  useEffect(() => {
    if (available) return;
    let changed = false;
    for (const [key, edit] of edits) {
      if (edit.phase !== "confirming") continue;
      edits.delete(key);
      changed = true;
    }
    if (changed) publish();
  }, [available, revision]);
  useEffect(() => {
    const background = () => {
      if (document.visibilityState === "hidden") flushDrafts();
    };
    document.addEventListener("visibilitychange", background);
    window.addEventListener("pagehide", flushDrafts);
    return () => {
      document.removeEventListener("visibilitychange", background);
      window.removeEventListener("pagehide", flushDrafts);
    };
  }, []);
  if (!pending || !available) return null;
  const [key, edit] = pending;
  function finish(accepted: boolean) {
    if (edits.get(key) !== edit || edit.phase !== "confirming") return;
    if (accepted) {
      edit.consented = true;
      edit.phase = "waiting";
      commit(key, edit);
    } else {
      edits.delete(key);
      publish();
    }
  }
  const message = key === "downloadDirectory"
    ? t("Change the download directory? Existing downloads will stay in their current folder unless you choose to move them.")
    : key === "privateMode"
      ? edit.value
        ? t("Enable private reading mode? New reading history, positions and sessions will not be saved. Existing history is not removed.")
        : t("Turn off private reading mode? Reading history, positions and sessions can be saved again according to your preferences.")
      : edit.value
        ? t("Enable strict read-only mode? Archive changes, downloads and persistent reading changes will be blocked. You can turn this off from Settings.")
        : t("Turn off strict read-only mode? Archive changes, downloads and persistent reading changes will be allowed again according to your preferences.");
  return <Modal title={t("Review preference change")} onClose={() => finish(false)}>
    <p>{message}</p>
    {key === "downloadDirectory" && <p className="file-path">{String(edit.value)}</p>}
    <div className="modal-actions">
      <button autoFocus type="button" onClick={() => finish(false)}>{t("Cancel")}</button>
      <button type="button" className="primary" onClick={(event) => {
        if (event.isTrusted) finish(true);
      }}>{t("Apply change")}</button>
    </div>
  </Modal>;
}

export function SettingsDirectoryMigration({ available }: { available: boolean }) {
  useSyncExternalStore(subscribe, snapshot);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<MigrationResult | null>(null);
  const migration = directoryChanges[0];
  const confirming = [...edits.values()].some((edit) => edit.phase === "confirming");
  function close() {
    if (busy) return;
    directoryChanges.shift();
    publish();
  }
  async function moveDownloads() {
    if (!migration || busy || !available) return;
    setBusy(true);
    try {
      const settings = getConfirmedSettings();
      if (!settings || settings.readOnly || settings.privateMode ||
          settings.downloadDirectory !== migration.to)
        throw new Error(t("The download directory or privacy settings changed. Previous downloads have not been moved."));
      setResult(await api<MigrationResult>("move_archives", {
        fromDirectory: migration.from,
        destination: migration.to,
        confirmed: true,
      }));
      migration.onRefresh();
    } catch (error) {
      setResult({ moved: [], errors: [{
        id: "migration", title: t("Moving previous downloads"), error: errorText(error),
      }], skipped: 0 });
    } finally {
      directoryChanges.shift();
      publish();
      setBusy(false);
    }
  }
  if (!available || confirming) return null;
  if (result) return <Modal title={t("Previous downloads")} onClose={() => setResult(null)}>
    <div role={result.errors.length ? "alert" : "status"}>
      <p>{t("{count} archives moved.", { count: result.moved.length })}</p>
      {result.skipped > 0 && <p>{t("{count} imported or unavailable archives left unchanged.", { count: result.skipped })}</p>}
      {result.moved.length > 0 && <ul>{result.moved.map((item) =>
        <li key={item.id}>{item.title}{!isIOS && <> · <span className="file-path">{item.to}</span></>}</li>)}</ul>}
      {result.errors.length > 0 && <>
        <p>{t("Some archives could not be moved. Their data has been preserved; unfinished journaled moves are checked at the next startup.")}</p>
        <ul>{result.errors.map((item) => <li key={item.id}>{item.title}: {item.error}</li>)}</ul>
      </>}
    </div>
    <div className="modal-actions"><button type="button" onClick={() => setResult(null)}>{t("Close")}</button></div>
  </Modal>;
  if (!migration) return null;
  return <Modal title={t("Move previous downloads?")} onClose={close}>
    <p>{t("The new download directory is saved. Move archives downloaded by this app from the previous directory? Imported files, monitored folders and unavailable archives will stay where they are.")}</p>
    {!isIOS && <p className="file-path">{migration.from} → {migration.to}</p>}
    <p>{t("Existing destination files are never overwritten. Keep the app open until the move finishes.")}</p>
    <div className="modal-actions">
      <button type="button" disabled={busy} onClick={close}>{t("Leave previous downloads in place")}</button>
      <button type="button" className="primary" disabled={busy} onClick={(event) => {
        if (event.isTrusted) void moveDownloads();
      }}>{busy ? t("Moving previous downloads…") : t("Move previous downloads")}</button>
    </div>
  </Modal>;
}
