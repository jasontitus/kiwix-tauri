import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  ClipboardCopy,
  ExternalLink,
  FolderOpen,
  Info,
  Plus,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import {
  api,
  capturePrivacyContext,
  chooseDirectory,
  flushPendingPreferences,
  isAndroid,
  isIOS,
  isMobile,
  isPrivacyContextCurrent,
} from "../api";
import { t, useLocale, getLocale, formatBytes } from "../i18n";
import { DIAGNOSTIC_LIMIT, diagnostics } from "../diagnostics";
import type { Book, DownloadAdmission, DownloadTarget, ImportResult, Settings as AppSettings, SharingStatus, Snapshot, TabLocation } from "../types";
import { Modal, PageHeader } from "./UI";
import AdvancedSettings from "./AdvancedSettings";
import SharingControls from "./SharingControls";
import UpdatesSettings from "./UpdatesSettings";
import { SettingsSaveStatus, useSettingsAutosave } from "./SettingsAutosave";
import PrivateReadingSync from "./PrivateReadingSync";
import SpeechModels from "./SpeechModels";
import zimruLicense from "../../src-tauri/licenses/zimru/LICENSE?raw";
import glassLicense from "../../src-tauri/licenses/glass/LICENSE?raw";
interface Props {
  settings: AppSettings;
  books: Book[];
  sharing: SharingStatus;
  notifications: Snapshot["notifications"];
  privacyGeneration: number;
  privateSyncIntent?: number;
  onPrivateSyncShown?: () => void;
  onOpen: (location: TabLocation) => void;
  webSharingIntent?: number;
  onWebSharingShown?: () => void;
  onRefresh: () => void;
  onError: (error: unknown) => void;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
}
interface StorageSummary {
  libraryBytes: number;
  archiveCount: number;
  directory: string;
  freeBytes: number | null;
  totalBytes: number | null;
}
interface StorageLocation {
  path: string;
  label: string;
  kind: "internal" | "external" | "custom";
  removable: boolean;
  freeBytes: number | null;
  totalBytes: number | null;
  active: boolean;
  available: boolean;
}
interface LibraryRootResult {
  copied: number;
  parts: number;
  bytes: number;
  from: string;
  to: string;
  removedOld: boolean;
  removedParts: number;
  keptOld: boolean;
  skipped: number;
}
interface AboutInfo {
  appVersion: string;
  tauriVersion: string;
  zimruVersion: string;
  glassVersion: string;
  zimruRevision: string | null;
  glassRevision: string | null;
  spellingIndexVersion: string;
  platform: string;
  architecture: string;
}
function DiagnosticsPanel() {
  const entries = useSyncExternalStore(
    diagnostics.subscribe,
    diagnostics.snapshot,
  );
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  async function copy() {
    try {
      await navigator.clipboard.writeText(diagnostics.text());
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }
  return (
    <div className="diagnostics-panel">
      <p className="muted">
        {t(
          "The last {count} failures are kept in memory for this session only. Nothing is written to disk or sent anywhere.",
          { count: DIAGNOSTIC_LIMIT },
        )}
      </p>
      {entries.length === 0 ? (
        <p className="muted">{t("No errors in this session.")}</p>
      ) : (
        <>
          <ol className="diagnostic-list">
            {[...entries].reverse().map((entry) => (
              <li key={entry.id}>
                <span
                  className={`diagnostic-severity diagnostic-${entry.severity}`}
                >
                  {entry.severity === "blocking"
                    ? t("Blocking")
                    : t("Recoverable")}
                </span>
                <time dateTime={entry.at}>
                  {new Date(entry.at).toLocaleTimeString(getLocale(), {
                    hour: "2-digit",
                    minute: "2-digit",
                    second: "2-digit",
                  })}
                </time>
                <span className="diagnostic-message">{entry.message}</span>
              </li>
            ))}
          </ol>
          <div className="modal-actions">
            <button type="button" onClick={() => void copy()}>
              <ClipboardCopy size={16} />
              {copyState === "copied" ? t("Copied") : t("Copy diagnostics")}
            </button>
            <button
              type="button"
              onClick={() => {
                diagnostics.clear();
                setCopyState("idle");
              }}
            >
              <Trash2 size={16} />
              {t("Clear diagnostics")}
            </button>
          </div>
          {copyState === "failed" && (
            <p className="muted" role="status">
              {t(
                "Could not copy diagnostics. Select the entries above instead.",
              )}
            </p>
          )}
        </>
      )}
    </div>
  );
}

export default function Settings({
  settings,
  books,
  sharing,
  notifications,
  privacyGeneration,
  privateSyncIntent = 0,
  onPrivateSyncShown,
  onOpen,
  webSharingIntent = 0,
  onWebSharingShown,
  onRefresh,
  onError,
  onDownload,
}: Props) {
  const { locales, setLocale } = useLocale();
  const webSharingDisclosure = useRef<HTMLDetailsElement>(null);
  const consumedWebSharingIntent = useRef(0);
  const privateSyncDisclosure = useRef<HTMLDetailsElement>(null);
  const consumedPrivateSyncIntent = useRef(0);
  useEffect(() => {
    if (!privateSyncIntent || consumedPrivateSyncIntent.current === privateSyncIntent) return;
    const disclosure = privateSyncDisclosure.current;
    if (!disclosure) return;
    consumedPrivateSyncIntent.current = privateSyncIntent;
    disclosure.open = true;
    const summary = disclosure.querySelector("summary");
    summary?.focus({ preventScroll: true });
    summary?.scrollIntoView({ block: "start" });
    onPrivateSyncShown?.();
  }, [privateSyncIntent, onPrivateSyncShown]);
  useEffect(() => {
    if (sharing.running && webSharingDisclosure.current) webSharingDisclosure.current.open = true;
  }, [sharing.running]);
  useEffect(() => {
    if (!webSharingIntent || consumedWebSharingIntent.current === webSharingIntent) return;
    const disclosure = webSharingDisclosure.current;
    if (!disclosure) return;
    consumedWebSharingIntent.current = webSharingIntent;
    disclosure.open = true;
    const summary = disclosure.querySelector("summary");
    summary?.focus({ preventScroll: true });
    summary?.scrollIntoView({ block: "start" });
    onWebSharingShown?.();
  }, [webSharingIntent, onWebSharingShown]);
  const portableReason = settings.portableMode
    ? t("Portable mode keeps downloads and monitored archives in the data folder beside the app. Folder paths cannot be changed.")
    : undefined;
  const [help, setHelp] = useState(false);
  const [storage, setStorage] = useState<StorageSummary | null>(null);
  const [storageUnavailable, setStorageUnavailable] = useState(false);
  const [storageRevision, setStorageRevision] = useState(0);
  const [locations, setLocations] = useState<StorageLocation[] | null>(null);
  const [locationsUnavailable, setLocationsUnavailable] = useState(false);
  const [pendingLocation, setPendingLocation] = useState<StorageLocation | null>(null);
  const [removeOldCopies, setRemoveOldCopies] = useState(false);
  const [locationBusy, setLocationBusy] = useState(false);
  const [locationNotice, setLocationNotice] = useState<string | null>(null);
  const [rootResult, setRootResult] = useState<LibraryRootResult | null>(null);
  const [about, setAbout] = useState<AboutInfo | null>(null);
  const [aboutUnavailable, setAboutUnavailable] = useState(false);
  const [requestingNotifications, setRequestingNotifications] = useState(false);
  const [notificationsDenied, setNotificationsDenied] = useState(false);
  const [externalUrl, setExternalUrl] = useState<string | null>(null);
  const [openingExternal, setOpeningExternal] = useState(false);
  const { draft, change, input, flush } = useSettingsAutosave(settings, {
    onRefresh,
    onError,
    onCommitted: (key, saved) => {
      if (key === "locale") setLocale(saved.locale);
    },
  });
  const activeLocation =
    locations?.find(
      (location) => location.path === (storage?.directory ?? draft.downloadDirectory),
    ) ?? locations?.find((location) => location.active);
  useEffect(() => {
    let cancelled = false;
    // Both inventories are local reads. They never require a permission,
    // network request or archive open, so Settings can always show what the
    // running build actually knows.
    setStorage(null);
    setStorageUnavailable(false);
    void api<StorageSummary>("storage_summary")
      .then((value) => {
        if (!cancelled) setStorage(value);
      })
      .catch(() => {
        if (!cancelled) setStorageUnavailable(true);
      });
    void api<AboutInfo>("about_info")
      .then((value) => {
        if (!cancelled) setAbout(value);
      })
      .catch(() => {
        if (!cancelled) setAboutUnavailable(true);
      });
    return () => {
      cancelled = true;
    };
  }, [storageRevision]);
  useEffect(() => {
    if (!isAndroid) return;
    let cancelled = false;
    setLocations(null);
    setLocationsUnavailable(false);
    const privacy = capturePrivacyContext();
    if (!privacy) {
      setLocationsUnavailable(true);
      return;
    }
    void api<{ locations: StorageLocation[] }>("mobile_storage_locations", {
      privacyGeneration: privacy.generation,
    })
      .then((value) => {
        if (!cancelled) setLocations(value.locations);
      })
      .catch(() => {
        if (!cancelled) {
          setLocations([]);
          setLocationsUnavailable(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [storageRevision]);
  async function switchLibraryLocation() {
    if (!pendingLocation || locationBusy || draft.privateMode || draft.readOnly)
      return;
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.privateMode || privacy.readOnly) return;
    setLocationBusy(true);
    try {
      const result = await api<LibraryRootResult>("mobile_library_root_set", {
        root: pendingLocation.path,
        removeOld: removeOldCopies,
        privacyGeneration: privacy.generation,
      });
      setPendingLocation(null);
      setRootResult(result);
      setLocationNotice(
        result.copied
          ? t("Library copied to {location}.", { location: pendingLocation.label })
          : t("Library location saved."),
      );
      setStorageRevision((value) => value + 1);
      onRefresh();
    } catch (error) {
      onError(error);
    } finally {
      setLocationBusy(false);
    }
  }
  async function enableCompletionNotifications() {
    if (requestingNotifications || !notifications.available || draft.privateMode || draft.readOnly) return;
    const context = capturePrivacyContext();
    if (!context || context.privateMode || context.readOnly) return;
    setRequestingNotifications(true);
    setNotificationsDenied(false);
    try {
      await flushPendingPreferences();
      if (!isPrivacyContextCurrent(context)) return;
      const result = await api<{ granted: boolean }>(
        "request_completion_notifications",
        { privacyGeneration: context.generation },
      );
      setNotificationsDenied(!result.granted);
      onRefresh();
    } catch (error) {
      onError(error);
    } finally {
      setRequestingNotifications(false);
    }
  }
  async function choose(key: "downloadDirectory" | "monitorDirectories") {
    try {
      if (settings.portableMode) return;
      if (isMobile && key === "downloadDirectory") return;
      if (isMobile) {
        const privacy = capturePrivacyContext();
        if (!privacy) return;
        const result = await api<ImportResult | null>("mobile_add_external_archives", {
          folder: true, privacyGeneration: privacy.generation,
        });
        if (!result) return;
        onRefresh();
        if (result.errors.length)
          onError(result.errors.map((item) => isIOS ? item.error : `${item.path}: ${item.error}`).join("\n"));
        return;
      }
      const path = await chooseDirectory();
      if (path)
        key === "downloadDirectory"
          ? change(key, path)
          : change(key, [...new Set([...draft.monitorDirectories, path])]);
    } catch (e) {
      onError(e);
    }
  }
  function external(url: string) {
    setHelp(false);
    setExternalUrl(url);
  }
  function closeExternal() {
    if (openingExternal) return;
    setExternalUrl(null);
    setHelp(true);
  }
  async function confirmExternal() {
    if (!externalUrl || openingExternal) return;
    setOpeningExternal(true);
    try {
      await api("open_external", { url: externalUrl, confirmed: true });
      setExternalUrl(null);
      setHelp(true);
    } catch (e) {
      onError(e);
    } finally {
      setOpeningExternal(false);
    }
  }
  return (
    <section className="page settings-page">
      <PageHeader
        title={t("Settings")}
        actions={
          <button onClick={() => setHelp(true)}>
            <Info size={16} />
            {t("Help & about")}
          </button>
        }
      />
      <SettingsSaveStatus />
      <form onSubmit={(event) => { event.preventDefault(); flush(); }}>
        <div className="settings-layout">
          <section className="settings-section panel">
            <h2>{t("Appearance & language")}</h2>
            <div className="form-grid">
              <label className="field">
                {t("Application theme")}
                <select
                  value={draft.theme}
                  onChange={(e) =>
                    change("theme", e.target.value as AppSettings["theme"])
                  }
                >
                  <option value="system">{t("Follow system")}</option>
                  <option value="light">{t("Light")}</option>
                  <option value="dark">{t("Dark")}</option>
                </select>
              </label>
              <label className="field">
                {t("Interface language")}
                <select
                  value={draft.locale}
                  onChange={(e) => change("locale", e.target.value)}
                >
                  <option value="system">{t("Follow system")}</option>
                  {locales.map((locale) => (
                    <option key={locale.code} value={locale.code}>
                      {locale.name}
                      {" · " +
                        new Intl.NumberFormat(getLocale(), {
                          style: "percent",
                          maximumFractionDigits: 0,
                        }).format(locale.coverage)}
                      {locale.coverage < 1
                        ? " · " + t("Partial translation · English fallback")
                        : ""}
                      {locale.machineTranslated
                        ? " · " + t("Machine translated · unreviewed")
                        : ""}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                {t("Reader appearance")}
                <select
                  value={draft.readerTheme}
                  onChange={(e) =>
                    change(
                      "readerTheme",
                      e.target.value as AppSettings["readerTheme"],
                    )
                  }
                >
                  <option value="original">{t("Archive original")}</option>
                  <option value="light">{t("Light")}</option>
                  <option value="dark">{t("Dark")}</option>
                  <option value="sepia">{t("Sepia")}</option>
                </select>
              </label>
              <label className="field">
                {t("Reader font")}
                <select
                  value={draft.fontFamily}
                  onChange={(e) =>
                    change(
                      "fontFamily",
                      e.target.value as AppSettings["fontFamily"],
                    )
                  }
                >
                  <option value="original">{t("Archive original")}</option>
                  <option value="serif">{t("Serif")}</option>
                  <option value="sans">{t("Sans serif")}</option>
                </select>
              </label>
              <label className="field">
                {t("Font size")}
                <input
                  type="number"
                  min={10}
                  max={48}
                  {...input("fontSize")}
                />
              </label>
              <label className="field">
                {t("Page zoom (%)")}
                <input
                  type="number"
                  min={25}
                  max={500}
                  step={5}
                  {...input("zoom")}
                />
              </label>
              <details className="secondary-controls">
                <summary>{t("Read aloud")}</summary>
                <p className="muted">
                  {t("Choose read-aloud language, voice and speed in the reader. Each language remembers its own preferences.")}
                </p>
                <SpeechModels />
              </details>
            </div>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.expandSections}
                onChange={(e) => change("expandSections", e.target.checked)}
              />
              <span>{t("Automatically expand article sections")}</span>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.linkPreviews}
                onChange={(e) => change("linkPreviews", e.target.checked)}
              />
              <span>{t("Show article previews for links")}</span>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.restoreSession}
                onChange={(e) => change("restoreSession", e.target.checked)}
              />
              <span>{t("Restore open tabs when the app starts")}</span>
            </label>
            <p className="muted">
              {t("Windows, reader tabs and reading positions reopen at launch. File → Restore previous session reopens the last saved session on demand; private and read-only profiles are never restored.")}
            </p>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.openNewTabInBackground}
                onChange={(e) => change("openNewTabInBackground", e.target.checked)}
              />
              <span>{t("Open links in a background tab")}</span>
            </label>
            <p className="muted">
              {t("When a link in an article opens a new tab, keep reading the current page and switch to the new tab later.")}
            </p>
          </section>
          <section className="settings-section panel">
            <h2>{t("Privacy & reading")}</h2>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.historyEnabled}
                onChange={(e) => change("historyEnabled", e.target.checked)}
              />
              <span>{t("Remember reading history and position")}</span>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.spellingEnabled}
                onChange={(e) => change("spellingEnabled", e.target.checked)}
              />
              <span>
                {t("Offer spelling suggestions for searches (opt-in)")}
              </span>
            </label>
            <label className="field">
              {t("External links")}
              <select
                value={draft.externalLinks}
                onChange={(e) =>
                  change(
                    "externalLinks",
                    e.target.value as AppSettings["externalLinks"],
                  )
                }
              >
                <option value="ask">
                  {t("Ask before opening in a browser")}
                </option>
                <option value="deny">{t("Block external links")}</option>
              </select>
            </label>
            <div className="privacy-note">
              <ShieldCheck size={22} />
              <p>
                {t(
                  "Camera and location are blocked until you explicitly allow an archive to use them. You can revoke access in the reader. No microphone access, telemetry or tracking.",
                )}
              </p>
            </div>
          </section>
          {notifications.available && (
            <section className="settings-section panel">
              <h2>{t("Notifications")}</h2>
              <label className="check-field">
                <input
                  type="checkbox"
                  checked={draft.completionNotifications}
                  disabled={requestingNotifications || draft.privateMode || draft.readOnly}
                  aria-describedby="completion-notifications-help"
                  onChange={(event) => {
                    setNotificationsDenied(false);
                    if (event.target.checked) void enableCompletionNotifications();
                    else change("completionNotifications", false);
                  }}
                />
                <span>{t("Notify when an archive is verified and added to the library")}</span>
              </label>
              <p className="muted" id="completion-notifications-help">
                {t("Off by default. Enabling notifications asks for system permission. Notifications are unavailable in private or read-only mode.")}
              </p>
              {requestingNotifications && (
                <p className="muted" role="status">{t("Requesting notification permission…")}</p>
              )}
              {notificationsDenied && (
                <p className="muted" role="status">
                  {t("Notifications were not enabled. You can change notification permissions in system settings.")}
                </p>
              )}
            </section>
          )}
          {isMobile && (
            <section className="settings-section panel">
              <h2>{t("Mobile transfers & backup")}</h2>
              <label className="check-field">
                <input
                  type="checkbox"
                  checked={draft.allowMeteredDownloads}
                  aria-describedby="mobile-network-policy-help"
                  onChange={(e) => change("allowMeteredDownloads", e.target.checked)}
                />
                <span>{t("Allow cellular and metered downloads")}</span>
              </label>
              <p className="muted" id="mobile-network-policy-help">
                {t("Off by default. Internet archive, torrent and map transfers stop when the operating system reports a cellular or metered connection. Nearby sharing does not use cellular data.")}
              </p>
              <p className="muted">
                {t("Interrupted archive downloads follow your automatic-resume preference when an allowed connection returns. Maps must be restarted. System background and battery restrictions still apply.")}
              </p>
              <h3>{t("Archive backup")}</h3>
              {isIOS ? (
                <>
                  <label className="check-field">
                    <input
                      type="checkbox"
                      checked={draft.backupArchives}
                      aria-describedby="mobile-backup-policy-help"
                      onChange={(e) => change("backupArchives", e.target.checked)}
                    />
                    <span>{t("Include app-owned archives in iOS backups")}</span>
                  </label>
                  <p className="muted" id="mobile-backup-policy-help">
                    {t("Off by default. Applies only to completed archive copies stored by this app, including imports and maps. Provider originals, temporary downloads, caches and credentials are not enabled for backup by this setting. Large archives may use significant backup storage; iOS and your backup settings determine whether a backup occurs.")}
                  </p>
                </>
              ) : (
                <p className="muted">
                  {t("Android backup is excluded by this app’s manifest and backup rules, not a runtime switch. Archives and app data are excluded from supported Android cloud backup and device-transfer mechanisms. Device manufacturers may apply their own transfer policies. Export files explicitly to keep a separate copy.")}
                </p>
              )}
            </section>
          )}
          <details className="settings-section panel secondary-controls" onInvalidCapture={(event) => { event.currentTarget.open = true; }}>
            <summary>{t("Files & downloads")}</summary>
            <h3>{t("Storage")}</h3>
            {storage ? (
              <>
                <dl className="facts">
                  <div>
                    <dt>{t("Library archives")}</dt>
                    <dd>
                      {formatBytes(storage.libraryBytes)}
                      {" · "}
                      {t("Archives: {count}", { count: storage.archiveCount })}
                    </dd>
                  </div>
                  <div>
                    <dt>{t("Free space")}</dt>
                    <dd>
                      {storage.freeBytes === null
                        ? t("Not reported by this platform")
                        : formatBytes(storage.freeBytes)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t("Total space")}</dt>
                    <dd>
                      {storage.totalBytes === null
                        ? t("Not reported by this platform")
                        : formatBytes(storage.totalBytes)}
                    </dd>
                  </div>
                  <div>
                    <dt>{t("Location")}</dt>
                    <dd>
                      {activeLocation
                        ? `${activeLocation.kind === "internal" ? t("Internal app storage") : activeLocation.label || t("External app storage")}${activeLocation.removable ? ` · ${t("Removable volume")}` : ""}`
                        : t("Selected library location")}
                      {" · "}
                      <span className="file-path">{storage.directory}</span>
                    </dd>
                  </div>
                </dl>
                <p className="muted">
                  {t("Library size is the sum of archive sizes from when they were added. Free and total space describe the volume holding the active library location; this platform may not expose them.")}
                </p>
              </>
            ) : storageUnavailable ? (
              <p className="muted">
                {t("Storage information is unavailable in this session.")}
              </p>
            ) : (
              <p className="muted" role="status">{t("Reading storage information…")}</p>
            )}
            {isMobile ? (
              <>
                <h3>{t("Kiwix storage")}</h3>
                <p>{t("Archives and downloads are stored in this app. Imported files are copied; the originals are not changed. Removing the app may remove its stored files.")}</p>
                {!isIOS && <p className="file-path">{draft.downloadDirectory}</p>}
                {isAndroid && (
                  <div className="storage-location-picker">
                    <h4>{t("Library location")}</h4>
                    <p className="muted">
                      {t("Archives are served from Kiwix's own app storage. A removable card is used through this app's own folder on that card; a raw SD-card path is never served directly.")}
                    </p>
                    {draft.privateMode || draft.readOnly ? (
                      <p className="privacy-note">
                        {t("The library location cannot be changed in private or read-only mode.")}
                      </p>
                    ) : locationsUnavailable ? (
                      <p className="muted">
                        {t("Storage locations are unavailable in this session.")}
                      </p>
                    ) : locations === null ? (
                      <p className="muted" role="status">
                        {t("Reading storage locations…")}
                      </p>
                    ) : (
                      <ul className="storage-location-list">
                        {locations.map((location) => (
                          <li key={location.path}>
                            <label className="check-field">
                              <input
                                type="radio"
                                name="library-location"
                                checked={location.active}
                                disabled={
                                  locationBusy ||
                                  (!location.available && !location.active)
                                }
                                onChange={() => {
                                  if (location.active) return;
                                  setRemoveOldCopies(false);
                                  setPendingLocation(location);
                                }}
                              />
                              <span>
                                <strong>
                                  {location.kind === "internal"
                                    ? t("Internal app storage")
                                    : location.label || t("External app storage")}
                                </strong>
                                {location.removable && (
                                  <span className="badge">{t("Removable")}</span>
                                )}
                                {location.active && (
                                  <span className="badge">{t("Current")}</span>
                                )}
                                {!location.available && (
                                  <span className="badge">{t("Unavailable")}</span>
                                )}
                                <span className="file-path">{location.path}</span>
                                <span className="muted">
                                  {location.freeBytes === null
                                    ? t("Free space not reported by this platform")
                                    : t("{free} free of {total}", {
                                        free: formatBytes(location.freeBytes),
                                        total:
                                          location.totalBytes === null
                                            ? t("an unknown total")
                                            : formatBytes(location.totalBytes),
                                      })}
                                </span>
                              </span>
                            </label>
                          </li>
                        ))}
                      </ul>
                    )}
                    {locationNotice && (
                      <p className="muted" role="status">
                        {locationNotice}
                      </p>
                    )}
                  </div>
                )}
                <p>{t("Deleted archive copies are permanently deleted after confirmation. Trash is not available on this device.")}</p>
                <button type="button" onClick={() => choose("monitorDirectories")}>
                  <FolderOpen size={16} />
                  {t("Add folder")}
                </button>
                <p>{t("Read archives from their original folder without copying them. Keep the storage connected and its provider available while reading.")}</p>
              </>
            ) : (
              <>
            {portableReason && (
              <p className="privacy-note" id="portable-path-reason">{portableReason}</p>
            )}
            <label className="field">
              {t("Download directory")}
              <div className="input-action">
                <input
                  disabled={settings.portableMode}
                  aria-describedby={portableReason ? "portable-path-reason" : undefined}
                  {...input("downloadDirectory")}
                  placeholder={t("Default downloads folder")}
                />
                <button
                  type="button"
                  disabled={settings.portableMode}
                  title={portableReason}
                  aria-describedby={portableReason ? "portable-path-reason" : undefined}
                  onClick={() => choose("downloadDirectory")}
                  aria-label={t("Choose download directory")}
                >
                  <FolderOpen size={17} />
                </button>
              </div>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={draft.trashDeletedFiles}
                onChange={(e) => change("trashDeletedFiles", e.target.checked)}
              />
              <span>
                {t(
                  "Move deleted archive files to Trash instead of deleting permanently",
                )}
              </span>
            </label>
            <div className="section-heading">
              <div>
                <h3>{t("Monitored folders")}</h3>
                <p>
                  {t(
                    "Archives in these folders are discovered automatically. Your files are never moved.",
                  )}
                </p>
              </div>
              <button
                type="button"
                disabled={settings.portableMode}
                title={portableReason}
                aria-describedby={portableReason ? "portable-path-reason" : undefined}
                onClick={() => choose("monitorDirectories")}
              >
                <Plus size={16} />
                {t("Add")}
              </button>
            </div>
            {draft.monitorDirectories.length ? (
              <ul className="directory-list">
                {draft.monitorDirectories.map((path) => (
                  <li key={path}>
                    <FolderOpen size={16} />
                    <span className="file-path">{path}</span>
                    <button
                      type="button"
                      className="icon-button danger"
                      disabled={settings.portableMode}
                      title={portableReason}
                      aria-describedby={portableReason ? "portable-path-reason" : undefined}
                      onClick={() =>
                        change(
                          "monitorDirectories",
                          draft.monitorDirectories.filter((p) => p !== path),
                        )
                      }
                      aria-label={t("Stop monitoring {path}", { path })}
                    >
                      <Trash2 size={16} />
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">{t("No folders are being monitored.")}</p>
            )}
              </>
            )}
          </details>
          <details className="settings-section panel secondary-controls" onInvalidCapture={(event) => { event.currentTarget.open = true; }}>
            <summary>{t("Catalog & map sources")}</summary>
            <p className="muted">
              {t(
                "Change these only when using a trusted, compatible catalog or map proxy.",
              )}
            </p>
            <label className="field">
              {t("Archive catalog URL")}
              <input
                type="url"
                required
                {...input("catalogUrl")}
              />
            </label>
            <label className="field">
              {t("StreetZim catalog URL")}
              <input
                type="url"
                {...input("mapCatalogUrl")}
              />
            </label>
            <label className="field">
              {t("Map proxy URL")}
              <input
                type="url"
                {...input("mapProxyUrl")}
              />
            </label>
          </details>
        </div>
      </form>
      <h2 className="settings-group-heading">{t("Sharing & devices")}</h2>
      <details className="secondary-controls" ref={privateSyncDisclosure}>
        <summary>{t("Connect my devices")}</summary>
        <PrivateReadingSync
          privateMode={settings.privateMode}
          readOnly={settings.readOnly}
          privacyGeneration={privacyGeneration}
          onOpen={onOpen}
          onRefresh={onRefresh}
        />
      </details>
      <details className="secondary-controls" ref={webSharingDisclosure}>
        <summary>
          {t("Web sharing (browsers and OPDS)")} · {sharing.running ? t("Sharing is on") : t("Sharing is off")}
        </summary>
      <SharingControls
        settings={settings}
        books={books}
        sharing={sharing}
        onError={onError}
        onRefresh={onRefresh}
      />
      </details>
      <UpdatesSettings
        currentVersion={about?.appVersion ?? null}
        onError={onError}
      />
      <h2 className="settings-group-heading">{t("Advanced & diagnostics")}</h2>
      <AdvancedSettings
        settings={settings}
        books={books}
        onError={onError}
        onRefresh={onRefresh}
        onDownload={onDownload}
      />
      <details className="settings-section panel secondary-controls">
        <summary>{t("Troubleshooting")}</summary>
        <label className="check-field">
          <input
            type="checkbox"
            checked={draft.showTechnicalErrors}
            onChange={(e) => change("showTechnicalErrors", e.target.checked)}
          />
          <span>{t("Show technical error details")}</span>
        </label>
        <p className="muted">
          {t(
            "When off, only failures that stop you from reading are shown. Other failures are still recorded here for support.",
          )}
        </p>
        <DiagnosticsPanel />
      </details>
      {help && (
        <Modal
          title={t("About Kiwix Workbench")}
          onClose={() => setHelp(false)}
        >
          <p>
            {t(
              "Kiwix reads ZIM archives locally. Import an archive or download one from Discover, then read it without an internet connection.",
            )}
          </p>
          <h3>{t("Build inventory")}</h3>
          {about ? (
            <dl className="facts">
              <div>
                <dt>{t("Application")}</dt>
                <dd>
                  {about.appVersion} · {about.platform}/{about.architecture}
                </dd>
              </div>
              <div>
                <dt>{t("Tauri runtime")}</dt>
                <dd>{about.tauriVersion}</dd>
              </div>
              <div>
                <dt>Zimru</dt>
                <dd>{about.zimruVersion}</dd>
              </div>
              <div>
                <dt>zimfile-xapian</dt>
                <dd>{about.glassVersion}</dd>
              </div>
              <div>
                <dt>{t("Spelling suggestions")}</dt>
                <dd>{about.spellingIndexVersion}</dd>
              </div>
            </dl>
          ) : aboutUnavailable ? (
            <p className="muted">
              {t("Build information is unavailable in this session.")}
            </p>
          ) : (
            <p className="muted" role="status">
              {t("Reading build information…")}
            </p>
          )}
          <p className="muted">
            {t(
              "Version values are read locally from the running build. When a bundled library does not expose its version, this panel says so instead of guessing.",
            )}
          </p>
          <h3>{t("Getting started")}</h3>
          <ol className="help-list">
            <li>
              {t(
                "Import ZIM files or add a folder from Library. Missing files can be located again without losing your reading list.",
              )}
            </li>
            <li>
              {t(
                "Use Discover for downloadable archives, and Maps for locally assembled StreetZim maps.",
              )}
            </li>
            <li>
              {t(
                "Open archives in tabs. Search, bookmark, print and read aloud from the reader toolbar.",
              )}
            </li>
            <li>
              {t(
                "Use the reading list to keep notes and tags, and export it as a portable backup.",
              )}
            </li>
          </ol>
          <h3>{t("Keyboard shortcuts")}</h3>
          <dl className="shortcut-list">
            <div>
              <dt>{t("Import archives")}</dt>
              <dd>
                <kbd>{t("⌘ / Ctrl + O")}</kbd>
              </dd>
            </div>
            <div>
              <dt>{t("New tab")}</dt>
              <dd>
                <kbd>{t("⌘ / Ctrl + T")}</kbd>
              </dd>
            </div>
            <div>
              <dt>{t("Close tab")}</dt>
              <dd>
                <kbd>{t("⌘ / Ctrl + W")}</kbd>
              </dd>
            </div>
            <div>
              <dt>{t("Reopen closed tab")}</dt>
              <dd>
                <kbd>{t("⌘ / Ctrl + Shift + T")}</kbd>
              </dd>
            </div>
            <div>
              <dt>{t("Switch tabs")}</dt>
              <dd>
                <kbd>{t("Ctrl + Tab")}</kbd>
              </dd>
            </div>
            <div>
              <dt>{t("New window")}</dt>
              <dd>
                <kbd>{t("⌘ / Ctrl + Shift + N")}</kbd>
              </dd>
            </div>
          </dl>
          <h3>{t("License")}</h3>
          <p>
            {t(
              "Kiwix Workbench is free software, licensed under the GNU General Public License, version 3 or later. Archive content and map imagery retain their respective licenses.",
            )}
          </p>
          <p>
            {t(
              "No account is required. No usage analytics or telemetry are collected. Network access occurs when you request downloads, catalogs, external links or local sharing.",
            )}
          </p>
          {[
            { name: "Zimru · MIT", text: zimruLicense, revision: about?.zimruRevision, repo: "https://github.com/jasontitus/zimru" },
            { name: "zimfile-xapian · GPL-3.0-or-later", text: glassLicense, revision: about?.glassRevision, repo: "https://github.com/jasontitus/zimru-xapian" },
          ].map(({ name, text, revision, repo }) => (
            <details key={name}>
              <summary>{name}</summary>
              <pre style={{ whiteSpace: "pre-wrap" }}>{text}</pre>
              {revision && (
                <button onClick={() => external(`${repo}/tree/${revision}`)}>
                  <ExternalLink size={16} />
                  {t("Source code & support")}
                </button>
              )}
            </details>
          ))}
          <div className="modal-actions">
            <button onClick={() => external("https://www.kiwix.org/")}>
              <ExternalLink size={16} />
              {t("Kiwix project")}
            </button>
            <button onClick={() => external("https://kiwix.org/en/donate/")}>
              <ExternalLink size={16} />
              {t("Donate to Kiwix")}
            </button>
            <button
              onClick={() => external("https://github.com/kiwix/kiwix-desktop")}
            >
              <ExternalLink size={16} />
              {t("Upstream source code & support")}
            </button>
            <button
              onClick={() =>
                external("https://www.gnu.org/licenses/gpl-3.0.html")
              }
            >
              <ExternalLink size={16} />
              {t("License")}
            </button>
          </div>
        </Modal>
      )}
      {pendingLocation && (
        <Modal
          title={t("Change library location?")}
          onClose={() => {
            if (!locationBusy) setPendingLocation(null);
          }}
        >
          <p>
            {t("Copy this app's managed archives to {location} and make it the library location? Each archive is copied, verified, and only then becomes active.", {
              location:
                pendingLocation.kind === "internal"
                  ? t("Internal app storage")
                  : pendingLocation.label || t("External app storage"),
            })}
          </p>
          <p className="file-path">
            {draft.downloadDirectory} → {pendingLocation.path}
          </p>
          <label className="check-field">
            <input
              type="checkbox"
              checked={removeOldCopies}
              disabled={locationBusy}
              onChange={(event) => setRemoveOldCopies(event.target.checked)}
            />
            <span>
              {t("Remove the old copies after every copied archive is verified")}
            </span>
          </label>
          <p className="muted">
            {t("Off by default: the previous copies are kept. If copying or verification fails, the previous library location stays active. Keep the app open until the copy finishes.")}
          </p>
          {!pendingLocation.available && (
            <p className="privacy-note">
              {t("This location is currently unavailable. Reconnect the volume before copying the library.")}
            </p>
          )}
          <div className="modal-actions">
            <button
              type="button"
              disabled={locationBusy}
              onClick={() => setPendingLocation(null)}
            >
              {t("Cancel")}
            </button>
            <button
              type="button"
              className="primary"
              disabled={locationBusy || !pendingLocation.available}
              onClick={(event) => {
                if (event.isTrusted) void switchLibraryLocation();
              }}
            >
              {locationBusy
                ? t("Copying and verifying…")
                : t("Copy library here")}
            </button>
          </div>
        </Modal>
      )}
      {rootResult && (
        <Modal title={t("Library location changed")} onClose={() => setRootResult(null)}>
          <p role="status">
            {rootResult.copied
              ? t("{count} archives copied and verified.", {
                  count: rootResult.copied,
                })
              : t("The library location was saved. There were no managed archives to copy.")}
          </p>
          <p className="muted">
            {rootResult.keptOld
              ? t("The previous copies were kept. You can delete them manually if you no longer need them.")
              : t("The previous copies were removed after the new copies passed verification.")}
          </p>
          {rootResult.skipped > 0 && (
            <p className="muted">
              {t("{count} imported or unavailable archives were left unchanged.", {
                count: rootResult.skipped,
              })}
            </p>
          )}
          <p className="file-path">{rootResult.to}</p>
          <div className="modal-actions">
            <button type="button" onClick={() => setRootResult(null)}>
              {t("Close")}
            </button>
          </div>
        </Modal>
      )}
      {externalUrl && (
        <Modal title={t("Open external link?")} onClose={closeExternal}>
          <p>{t("Open this link in your default browser?\n{url}", { url: externalUrl })}</p>
          <div className="modal-actions">
            <button autoFocus disabled={openingExternal} onClick={closeExternal}>
              {t("Cancel")}
            </button>
            <button className="primary" disabled={openingExternal} onClick={() => void confirmExternal()}>
              <ExternalLink size={16} />
              {openingExternal ? t("Opening…") : t("Open in browser")}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
