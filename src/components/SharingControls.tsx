import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Check, Copy, ExternalLink, RefreshCw } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { api, capturePrivacyContext, isAndroid, isIOS, isPrivacyContextCurrent } from "../api";
import { t } from "../i18n";
import { errorText } from "./UI";
import { isSuspendedError } from "../error-visibility";
import { validSharingName } from "../sharing-name";
import { SettingsSaveStatus, useSettingsAutosave } from "./SettingsAutosave";
import type {
  Book,
  LocalSwarmHotspot,
  Settings,
  SharingBinding,
  SharingInterface,
  SharingScope,
  SharingStatus,
} from "../types";
import "./sharing-controls.css";

interface Props {
  settings: Settings;
  books: Book[];
  sharing: SharingStatus;
  onError: (error: unknown) => void;
  onRefresh: () => void;
}

type BindingMode = SharingBinding["mode"];

export default function SharingControls({
  settings,
  books,
  sharing,
  onError,
  onRefresh,
}: Props) {
  const [mode, setMode] = useState<BindingMode>(
    sharing.binding?.mode ?? "loopback",
  );
  const [address, setAddress] = useState(
    sharing.binding?.mode === "interface" ? sharing.binding.address : "",
  );
  const [port, setPort] = useState(String(sharing.port || 0));
  const [scope, setScope] = useState<SharingScope>("library");
  const [pin, setPin] = useState("");
  const [localError, setLocalError] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [publisherName, setPublisherName] = useState(settings.sharingName);
  const nameEdited = useRef(false);
  const [ids, setIds] = useState(
    sharing.running ? sharing.bookIds : settings.sharingBookIds,
  );
  const [interfaces, setInterfaces] = useState<SharingInterface[]>([]);
  const [hotspot, setHotspot] = useState<LocalSwarmHotspot | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [qrSelection, setQrSelection] = useState("");
  const [visibleQrUrl, setVisibleQrUrl] = useState<string | null>(null);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const qrId = useId();
  const optionsDetails = useRef<HTMLDetailsElement>(null);
  const errorHandler = useRef(onError);
  errorHandler.current = onError;
  const refreshHandler = useRef(onRefresh);
  refreshHandler.current = onRefresh;
  // The status that arrived with the page describes the network at that
  // moment. While sharing is on and this view is visible, ask again so the
  // links follow an address change and a dead link is not shown as working.
  const [observed, setObserved] = useState<SharingStatus | null>(null);
  const bindingTouched = useRef(false);
  const hotspotChosen = useRef(false);
  const alive = useRef(false);
  const pending = useRef(false);
  const namePreferences = useSettingsAutosave(settings, { onRefresh });
  useEffect(() => {
    if (!nameEdited.current && settings.sharingName === namePreferences.draft.sharingName)
      setPublisherName(settings.sharingName);
  }, [settings.sharingName, namePreferences.draft.sharingName]);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  // A browser-restored open state has no toggle event; read it after mount so
  // the archive rows exist whenever the section is actually visible.
  useEffect(() => {
    if (optionsDetails.current) setOptionsOpen(optionsDetails.current.open);
  }, []);
  useEffect(() => {
    setPin("");
    setLocalError("");
    setVisibleQrUrl(null);
  }, [settings.privateMode, settings.readOnly]);

  useEffect(() => {
    if (!sharing.running) setVisibleQrUrl(null);
  }, [sharing.running]);

  useEffect(() => {
    setObserved(null);
    if (!sharing.running) return;
    let active = true;
    let reported = false;
    let asking = false;
    let timer: number | undefined;
    async function poll() {
      // One chain only: a request in flight schedules the next one itself.
      if (!active || asking || document.hidden) return;
      asking = true;
      try {
        const value = await api<SharingStatus>("sharing_status");
        if (!active) return;
        if (value.running && value.sessionId === sharing.sessionId) setObserved(value);
        else if (!reported) {
          // Sharing ended or was replaced elsewhere; let the page catch up.
          reported = true;
          setObserved(null);
          refreshHandler.current();
        }
      } catch {
        // The page's own status stays on screen; the next poll asks again.
      } finally {
        asking = false;
      }
      window.clearTimeout(timer);
      if (active && !document.hidden) timer = window.setTimeout(() => void poll(), 4000);
    }
    function visibility() {
      window.clearTimeout(timer);
      if (!document.hidden) void poll();
    }
    document.addEventListener("visibilitychange", visibility);
    void poll();
    return () => {
      active = false;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [sharing.running, sharing.sessionId]);

  useEffect(() => {
    let active = true;
    api<SharingInterface[]>("sharing_interfaces")
      .then((items) => {
        if (active) setInterfaces(items);
      })
      .catch((error) => {
        if (active) errorHandler.current(error);
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!isAndroid) return;
    let active = true;
    api<LocalSwarmHotspot | null>("hotspot_status")
      .then((value) => {
        if (active) setHotspot(value);
      })
      .catch(() => {
        // The hotspot card owns its own error surface; a share must not fail
        // because the read-only probe did.
        if (active) setHotspot(null);
      });
    return () => {
      active = false;
    };
  }, [sharing.running]);

  // An existing owned hotspot is an optional Web binding. Never replace an
  // interface the user explicitly selected.
  useEffect(() => {
    if (
      hotspotChosen.current ||
      bindingTouched.current ||
      sharing.running ||
      mode !== "loopback"
    )
      return;
    if (hotspot?.phase === "active") {
      hotspotChosen.current = true;
      setMode("hotspot");
    }
  }, [hotspot, mode, sharing.running]);

  useEffect(() => {
    setIds(sharing.running ? sharing.bookIds : settings.sharingBookIds);
    if (sharing.running) {
      setScope(sharing.scope);
      if (sharing.binding) {
        setMode(sharing.binding.mode);
        setAddress(
          sharing.binding.mode === "interface" ? sharing.binding.address : "",
        );
        setPort(String(sharing.port));
      }
    }
    setCopied(null);
  }, [
    sharing.running,
    sharing.scope,
    sharing.bookIds,
    sharing.binding,
    sharing.port,
    settings.sharingBookIds,
  ]);

  async function refreshInterfaces() {
    setLoading(true);
    try {
      const [items, current] = await Promise.all([
        api<SharingInterface[]>("sharing_interfaces"),
        isAndroid
          ? api<LocalSwarmHotspot | null>("hotspot_status")
          : Promise.resolve(null),
      ]);
      setInterfaces(items);
      if (isAndroid) setHotspot(current);
    } catch (error) {
      onError(error);
    } finally {
      setLoading(false);
    }
  }

  async function selectBook(id: string, selected: boolean) {
    const previous = ids;
    const next = selected
      ? [...new Set([...ids, id])]
      : ids.filter((value) => value !== id);
    setIds(next);
    setBusy(true);
    try {
      await api("save_settings", { settings: { sharingBookIds: next } });
      onRefresh();
    } catch (error) {
      setIds(previous);
      onError(error);
    } finally {
      setBusy(false);
    }
  }

  function savePublisherName(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (settings.privateMode || settings.readOnly || pending.current || !capturePrivacyContext()) return;
    const name = publisherName.trim();
    if (!validSharingName(name)) {
      setLocalError(t("Use a shorter device name without control or direction-override characters."));
      return;
    }
    setLocalError("");
    nameEdited.current = false;
    setPublisherName(name);
    // Only submitted names enter the shell-owned retry mechanism; typing is local.
    namePreferences.change("sharingName", name);
  }

  async function toggleSharing() {
    const context = capturePrivacyContext();
    if (settings.privateMode || settings.readOnly || pending.current || !context) return;
    if (!sharing.running && !validSelection) return;
    pending.current = true;
    setBusy(true);
    setLocalError("");
    try {
      const binding: SharingBinding =
        mode === "interface" ? { mode, address } : { mode };
      const request = {
        scope, ...(scope === "selected" ? { bookIds: ids } : {}),
        ...(pin ? { pin } : {}),
        ...(advanced ? { port: Number(port), binding } : {}),
      };
      await api(sharing.running ? "sharing_stop" : "sharing_start",
        { ...(sharing.running ? {} : request), privacyGeneration: context.generation });
      if (alive.current && isPrivacyContextCurrent(context)) {
        setPin("");
        setCopied(null);
        onRefresh();
      }
    } catch (error) {
      if (alive.current && isPrivacyContextCurrent(context) && !isSuspendedError(error))
        setLocalError(errorText(error));
    } finally {
      pending.current = false;
      if (alive.current) setBusy(false);
    }
  }

  async function openInBrowser(url: string) {
    try {
      await api("open_external", { url, confirmed: true });
    } catch (error) {
      onError(error);
    }
  }

  async function copy(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(url);
    } catch (error) {
      onError(error);
    }
  }

  const locked = settings.privateMode || settings.readOnly || busy || sharing.running;
  const numericPort = Number(port);
  const validPort =
    port.trim() !== "" &&
    Number.isInteger(numericPort) &&
    numericPort >= 0 &&
    numericPort <= 65535;
  const bookById = useMemo(
    () => new Map(books.map((book) => [book.id, book])),
    [books],
  );
  const selectedIds = useMemo(() => new Set(ids), [ids]);
  const availableBooks = useMemo(
    () => books.filter((book) => book.available),
    [books],
  );
  const availableIds = useMemo(
    () => new Set(availableBooks.map((book) => book.id)),
    [availableBooks],
  );
  const missingIds = useMemo(
    () => ids.filter((id) => !bookById.has(id)),
    [ids, bookById],
  );
  const validSelection =
    scope === "library"
      ? availableBooks.length > 0
      : ids.length > 0 && ids.every((id) => availableIds.has(id));
  const hotspotActive =
    hotspot?.phase === "active" ||
    (sharing.running && sharing.binding?.mode === "hotspot");
  const hotspotAddress =
    hotspot?.phase === "active"
      ? hotspot.address
      : sharing.running && sharing.binding?.mode === "hotspot" && sharing.url
        ? new URL(sharing.url).hostname
        : undefined;
  const validInterface =
    (mode !== "interface" ||
      interfaces.some((item) => item.address === address)) &&
    (mode !== "hotspot" || hotspotActive);
  const shown =
    observed && sharing.running && observed.sessionId === sharing.sessionId
      ? observed
      : sharing;
  const qrUrl = sharing.running
    ? shown.urls.includes(qrSelection)
      ? qrSelection
      : (shown.urls[0] ?? "")
    : "";
  const qrVisible = qrUrl !== "" && visibleQrUrl === qrUrl;

  return (
    <section
      className="settings-section panel sharing-section sharing-controls"
      aria-busy={busy}
    >
      <div className="section-heading">
        <div>
          <h2>{t("Web sharing")}</h2>
          <p>{t("Let browsers and OPDS readers access this library. This is separate from Nearby.")}</p>
        </div>
      </div>
      <p className="sharing-web-warning" role="note">{t("Web sharing is unencrypted. Enable it only on a trusted network. Anyone on that network can browse and download an open share.")}</p>
      {isIOS && <p className="muted">{t("Keep this app open on the iPhone or iPad while sharing.")}</p>}
      <div className="toolbar sharing-primary-actions">
        <span className={`badge ${sharing.running ? "success" : ""}`} role="status">
          {sharing.running ? t("Sharing is on") : t("Sharing is off")}
        </span>
        <button type="button" className={sharing.running ? "danger" : "primary"}
          disabled={settings.privateMode || settings.readOnly || busy || (!sharing.running &&
            (!validSelection || (pin !== "" && !/^[0-9]{6,12}$/.test(pin)) ||
              (advanced && (!validPort || !validInterface))))}
          onClick={() => void toggleSharing()}>
          {busy ? t("Working…") : sharing.running ? t("Stop web sharing") : t("Enable web sharing")}
        </button>
        {sharing.running && <span className="muted">{sharing.locked ? t("PIN protected") : t("Open share")}</span>}
      </div>
      {localError && <p className="inline-error" role="alert">{localError}</p>}
      {sharing.running && shown.addressLost && (
        <p className="inline-error" role="alert">
          {t(
            "This device is no longer connected at the address this share uses, so its link and QR code will not open. Reconnect to that network, or stop web sharing and enable it again to share on the current one.",
          )}
        </p>
      )}
      {sharing.running && shown.localOnly && (
        <p className="inline-error" role="alert">
          {t(
            "No network address is available for this share, so only this device can open the links below. Connect to a trusted Wi-Fi or Ethernet network, or stop web sharing and choose a different “Listen on” option. Sharing continues on a new connection without restarting.",
          )}
        </p>
      )}
      {!sharing.running && availableBooks.length === 0 && <p className="muted">{t("Import an available archive before starting sharing.")}</p>}
      <details className="sharing-secondary">
        <summary>{t("Device name")}</summary>
      <form onSubmit={(event) => void savePublisherName(event)}>
        <label className="field">
          {t("Device name")}
          <input aria-label={t("Device name")} value={publisherName} maxLength={160} disabled={settings.privateMode || settings.readOnly || busy}
            placeholder={t("Automatic platform name")} autoComplete="off"
            onChange={(event) => { nameEdited.current = true; setPublisherName(event.target.value); }} />
          <span className="muted">{t("Leave blank to use the platform name. Changes apply to new shares; names are not proof of identity.")}</span>
        </label>
        <button type="submit" disabled={settings.privateMode || settings.readOnly || busy || publisherName.trim() === namePreferences.draft.sharingName}>
          {t("Save device name")}
        </button>
      </form>
      <SettingsSaveStatus />
      </details>
      <details className="sharing-options" ref={optionsDetails}
        onToggle={(event) => setOptionsOpen(event.currentTarget.open)}>
        <summary>{t("Choose archives or add a PIN")}</summary>
      <div className="form-grid">
        <label className="field">
          {t("What to serve")}
          <select
            value={scope}
            disabled={locked}
            onChange={(event) => setScope(event.target.value === "selected" ? "selected" : "library")}
          >
            <option value="selected">{t("Selected archives")}</option>
            <option value="library">{t("Entire library")}</option>
          </select>
        </label>
        <label className="field">
          {t("Optional share PIN")}
          <input type="password" inputMode="numeric" pattern="[0-9]{6,12}" minLength={6} maxLength={12} value={pin} disabled={locked} autoComplete="new-password" spellCheck={false} onChange={(event) => setPin(event.target.value)} />
          <span className="muted">{t("Use 6–12 digits. PINs are not saved; share the PIN separately.")}</span>
        </label>
      </div>
      <label className="check-field sharing-advanced-toggle">
        <input type="checkbox" checked={advanced} disabled={locked} onChange={(event) => setAdvanced(event.target.checked)} />
        <span>{t("Advanced network settings")}</span>
      </label>
      {advanced && <>
      <div className="form-grid">
        <label className="field">
          {t("Listen on")}
          <select
            value={mode}
            disabled={locked}
            onChange={(event) => {
              bindingTouched.current = true;
              const value = event.target.value;
              if (value === "loopback" || value === "hotspot" || value === "ipv4All" || value === "ipv6All" || value === "dualStackAll" || value === "interface") setMode(value);
            }}
          >
            <option value="loopback">{t("This device only (loopback)")}</option>
            {isAndroid && (
              <option value="hotspot" disabled={!hotspotActive}>
                {hotspotAddress
                  ? t("This phone's Wi-Fi hotspot ({address})", {
                      address: hotspotAddress,
                    })
                  : t("This phone's Wi-Fi hotspot")}
              </option>
            )}
            <option value="ipv4All">{t("All IPv4 interfaces")}</option>
            <option value="ipv6All">{t("All IPv6 interfaces")}</option>
            <option value="dualStackAll">
              {t("All IPv4 and IPv6 interfaces")}
            </option>
            <option value="interface">{t("A selected local interface")}</option>
          </select>
        </label>
        <label className="field">
          {t("Index port")}
          <input
            type="number"
            min={0}
            max={65535}
            step={1}
            value={port}
            disabled={locked}
            aria-describedby="sharing-port-note"
            onChange={(event) => setPort(event.target.value)}
          />
          <button
            type="button"
            className="sharing-auto-port"
            disabled={locked}
            onClick={() => setPort("0")}
          >
            {t("Use an available port")}
          </button>
        </label>
      </div>
      <p className="muted" id="sharing-port-note">
        {t(
          "Port 0 chooses an available port. Each archive also gets its own available port so article links and assets keep working.",
        )}
      </p>
      {mode === "interface" && (
        <div className="sharing-interface-picker">
          <label className="field">
            {t("Local interface address")}
            <select
              disabled={locked || loading}
              value={address}
              onChange={(event) => setAddress(event.target.value)}
            >
              <option value="">{t("Choose an interface")}</option>
              {address &&
                !interfaces.some((item) => item.address === address) && (
                  <option value={address}>
                    {address} — {t("Unavailable")}
                  </option>
                )}
              {interfaces.map((item) => (
                <option
                  key={`${item.name}/${item.address}`}
                  value={item.address}
                >
                  {item.name} · {item.address} ·{" "}
                  {item.family === "ipv4" ? "IPv4" : "IPv6"}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={locked || loading}
            onClick={refreshInterfaces}
          >
            <RefreshCw size={14} aria-hidden="true" /> {t("Refresh interfaces")}
          </button>
          <p className="muted">
            {t(
              "Only usable local IP addresses are listed. Scoped and link-local addresses are excluded.",
            )}
          </p>
        </div>
      )}
      {mode === "hotspot" && (
        <p className="muted" id="sharing-hotspot-note">
          {hotspotAddress
            ? t(
                "Devices that join this phone's hotspot reach this server at {address}:{port}. Open the access link below in their browser. Stopping either the hotspot or sharing releases both.",
                { address: hotspotAddress, port },
              )
            : t(
                "Connect this device to a trusted network, then choose a local interface or all interfaces.",
              )}
        </p>
      )}
      </>}
      {scope === "selected" ? (
        <fieldset className="sharing-selection" disabled={locked}>
          <legend>{t("Archives to share")}</legend>
          {optionsOpen && <>
            {books.map((book) => (
              <label className="check-field" key={book.id}>
                <input
                  type="checkbox"
                  checked={selectedIds.has(book.id)}
                  disabled={!book.available && !selectedIds.has(book.id)}
                  onChange={(event) =>
                    selectBook(book.id, event.target.checked)
                  }
                />
                <span>
                  {book.title}
                  {!book.available && <> — {t("Unavailable")}</>}
                </span>
              </label>
            ))}
            {missingIds.map((id) => (
              <label className="check-field" key={id}>
                <input
                  type="checkbox"
                  checked
                  onChange={() => selectBook(id, false)}
                />
                <span>
                  {id} — {t("Unavailable")}
                </span>
              </label>
            ))}
          </>}
          {!availableBooks.length && (
            <p className="muted">
              {t("Import an available archive before starting sharing.")}
            </p>
          )}
        </fieldset>
      ) : (
        <div className="sharing-selection">
          <h3>{t("Entire library")}</h3>
          <p>
            {t(
              "Every available archive is served from this device. Archives added while the server is running appear on the index automatically.",
            )}
          </p>
          <p className="muted">
            {t("{count} archives are available now.", {
              count: availableBooks.length,
            })}
          </p>
          {availableBooks.length === 0 && (
            <p className="muted">
              {t("Import an available archive before starting sharing.")}
            </p>
          )}
        </div>
      )}
      {sharing.running && (
        <p className="muted">
          {t(
            "Stop sharing to change what is served, the interface, or the port.",
          )}
        </p>
      )}
      {(settings.privateMode || settings.readOnly) && (
        <p className="muted">{t("Web sharing is unavailable in private or read-only mode.")}</p>
      )}
      {!sharing.running && <div className="toolbar">
        <button type="button" className="primary"
          disabled={locked || !validSelection || (pin !== "" && !/^[0-9]{6,12}$/.test(pin)) || (advanced && (!validPort || !validInterface))}
          onClick={() => void toggleSharing()}>
          {t("Enable web sharing")}
        </button>
      </div>}
      </details>
      {sharing.running && !settings.privateMode && !settings.readOnly && (
        <div className="sharing-live-status">
          {sharing.scope === "library" && (
            <p className="muted">
              {t("Serving {count} available archives.", { count: sharing.libraryCount })}
            </p>
          )}
          <details className="sharing-secondary">
          <summary>{t("Manual connection details")}</summary>
          <h3>
            {sharing.scope === "library"
              ? t("Library access links")
              : t("Collection access links")}
          </h3>
          <p className="muted">
            {t(
              "Choose the address reachable from the other device. IPv6 addresses appear in brackets.",
            )}
          </p>
          {shown.urls.map((url) => (
            <div className="sharing-address" key={url}>
              <code>{url}</code>
              <button type="button" onClick={() => copy(url)}>
                {copied === url ? (
                  <Check size={16} aria-hidden="true" />
                ) : (
                  <Copy size={16} aria-hidden="true" />
                )}
                {copied === url ? t("Copied") : t("Copy access link")}
              </button>
              <button type="button" onClick={() => openInBrowser(url)}>
                <ExternalLink size={16} aria-hidden="true" />{" "}
                {t("Open in browser")}
              </button>
            </div>
          ))}
          {sharing.scope === "library" && shown.catalogUrl && (
            <div className="sharing-address">
              <code>{shown.catalogUrl}</code>
              <button type="button" onClick={() => copy(shown.catalogUrl!)}>
                {copied === shown.catalogUrl ? (
                  <Check size={16} aria-hidden="true" />
                ) : (
                  <Copy size={16} aria-hidden="true" />
                )}
                {copied === shown.catalogUrl
                  ? t("Copied")
                  : t("Copy catalog link")}
              </button>
              <button
                type="button"
                onClick={() => openInBrowser(shown.catalogUrl!)}
              >
                <ExternalLink size={16} aria-hidden="true" />{" "}
                {t("Open catalog in browser")}
              </button>
            </div>
          )}
          {qrUrl && (
            <div className="sharing-qr">
              <h4>{t("QR code access")}</h4>
              <label className="field">
                {t("Access link for QR code")}
                <select
                  value={qrUrl}
                  onChange={(event) => {
                    setQrSelection(event.target.value);
                    setVisibleQrUrl(null);
                  }}
                >
                  {shown.urls.map((url) => (
                    <option key={url} value={url}>
                      {url}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                aria-expanded={qrVisible}
                aria-controls={qrId}
                onClick={() => setVisibleQrUrl(qrVisible ? null : qrUrl)}
              >
                {qrVisible ? t("Hide QR code") : t("Show QR code")}
              </button>
              <div id={qrId} hidden={!qrVisible}>
                {qrVisible && (
                  <>
                    <QRCodeSVG
                      className="sharing-qr-code"
                      value={qrUrl}
                      size={256}
                      level="M"
                      marginSize={4}
                      bgColor="#ffffff"
                      fgColor="#000000"
                      role="img"
                      title={t("Collection access QR code")}
                      aria-label={t("Collection access QR code")}
                    />
                    <p className="warning-text">
                      {t(
                        "Scan this code on a device on the same trusted network. Anyone with this code can access the items you share.",
                      )}
                    </p>
                  </>
                )}
              </div>
            </div>
          )}
          <p className="muted">
            {t("Listening sockets")}: <code>{sharing.bindings.join(", ")}</code>
          </p>
          {sharing.scope === "selected" && (
            <details>
              <summary>{t("Individual archive access links")}</summary>
              {shown.books.map((book) => (
                <div className="sharing-book-links" key={book.id}>
                  <h4>
                    {bookById.get(book.id)?.title ||
                      book.id}
                  </h4>
                  {book.urls.map((url) => (
                    <div className="sharing-address" key={url}>
                      <code>{url}</code>
                      <button type="button" onClick={() => copy(url)}>
                        {copied === url ? (
                          <Check size={16} aria-hidden="true" />
                        ) : (
                          <Copy size={16} aria-hidden="true" />
                        )}
                        {copied === url ? t("Copied") : t("Copy access link")}
                      </button>
                    </div>
                  ))}
                </div>
              ))}
            </details>
          )}
          </details>
        </div>
      )}
    </section>
  );
}
