import { useEffect, useId, useRef, useState } from "react";
import { BookOpen, Copy, Link, RefreshCw, ShieldCheck, Unlink } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { capturePrivacyContext, isPrivacyContextCurrent, subscribeReaderPreferences, type PrivacyContext } from "../api";
import { privateSyncRequest } from "../private-sync-api";
import { t, formatDate } from "../i18n";
import type { PrivateSyncConflict, PrivateSyncStatus } from "../private-sync-types";
import type { TabLocation } from "../types";
import { confirmAction, errorText } from "./UI";
import "./private-reading-sync.css";

interface Props {
  privateMode: boolean;
  readOnly: boolean;
  privacyGeneration: number;
  presentation?: "settings" | "reading";
  onConnect?: () => void;
  onOpen: (location: TabLocation) => void;
  onRefresh: () => void;
}

type OwnedSession = { id: string; privacy: PrivacyContext };

function ConflictChoice({ conflict, disabled, onChoose, onCopy }: {
  conflict: PrivateSyncConflict;
  disabled: boolean;
  onChoose: (alternativeId: string) => void;
  onCopy: (value: string) => void;
}) {
  const id = useId();
  const [choice, setChoice] = useState("");
  const selected = conflict.alternatives.some((alternative) => alternative.id === choice);
  return <fieldset className="private-sync-conflict" disabled={disabled}>
    <legend>{conflict.title || t("Untitled article")}</legend>
    <p>{conflict.field === "note" ? t("Note") : conflict.field === "comment" ? t("Highlight comment") : t("Saved reading data")}</p>
    <p className="muted">{t("Compare or copy these versions before choosing. Saving one version discards the other versions of this note or comment.")}</p>
    {conflict.alternatives.map((alternative) => <div key={alternative.id}>
      <label className="private-sync-alternative">
        <input type="radio" name={id} checked={choice === alternative.id} onChange={() => setChoice(alternative.id)} />
        <span><strong>{alternative.deviceName || t("Another device")}</strong><span className="private-sync-version" dir="auto">{alternative.value || t("Empty version")}</span></span>
      </label>
      <button disabled={disabled} onClick={() => onCopy(alternative.value)}><Copy size={16} aria-hidden="true" />{t("Copy this version")}</button>
    </div>)}
    <button disabled={disabled || !selected} onClick={() => onChoose(choice)}>{t("Use selected version")}</button>
  </fieldset>;
}

/** Private profile data never shares the public archive-sharing surface or authority. */
export default function PrivateReadingSync({
  privateMode, readOnly, privacyGeneration, presentation = "settings", onConnect, onOpen, onRefresh,
}: Props) {
  const id = useId();
  const [status, setStatus] = useState<PrivateSyncStatus | null>(null);
  const [invitation, setInvitation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [statusError, setStatusError] = useState("");
  const [notice, setNotice] = useState("");
  const [foreground, setForeground] = useState(() => document.visibilityState === "visible" && document.hasFocus());
  const alive = useRef(false);
  const pending = useRef(false);
  const revision = useRef(0);
  const ownedSession = useRef<OwnedSession | null>(null);
  const statusPrivacy = useRef<PrivacyContext | null>(null);
  const observedGeneration = useRef(privacyGeneration);
  const callbacks = useRef({ onOpen, onRefresh });
  callbacks.current = { onOpen, onRefresh };
  const blocked = privateMode || readOnly;

  function context(): PrivacyContext | null {
    const privacy = capturePrivacyContext();
    return alive.current && privacy && !privacy.privateMode && !privacy.readOnly && document.visibilityState === "visible" && document.hasFocus() ? privacy : null;
  }
  function current(privacy: PrivacyContext, operation: number): boolean {
    return alive.current && revision.current === operation && isPrivacyContextCurrent(privacy) && !!context();
  }
  function cancelOwned() {
    const owned = ownedSession.current;
    ownedSession.current = null;
    if (owned && isPrivacyContextCurrent(owned.privacy)) {
      void privateSyncRequest("private_sync_pair_cancel", { sessionId: owned.id }, owned.privacy).catch(() => {
        // Native expiry and lifecycle retirement also revoke this bounded session.
      });
    }
  }

  useEffect(() => {
    alive.current = true;
    function retire(cancelSession = true) {
      revision.current++;
      pending.current = false;
      setBusy(false);
      setStatus(null);
      statusPrivacy.current = null;
      setInvitation("");
      setNotice("");
      setError("");
      setStatusError("");
      if (cancelSession) cancelOwned();
    }
    function activity() {
      const active = document.visibilityState === "visible" && document.hasFocus();
      setForeground(active);
      if (!active) retire(false);
    }
    const unsubscribe = subscribeReaderPreferences(() => {
      if (statusPrivacy.current && !isPrivacyContextCurrent(statusPrivacy.current)) retire();
    });
    document.addEventListener("visibilitychange", activity);
    window.addEventListener("focus", activity);
    window.addEventListener("blur", activity);
    return () => {
      alive.current = false;
      revision.current++;
      cancelOwned();
      unsubscribe();
      document.removeEventListener("visibilitychange", activity);
      window.removeEventListener("focus", activity);
      window.removeEventListener("blur", activity);
    };
  }, []);

  useEffect(() => {
    revision.current++;
    setStatus(null);
    setInvitation("");
    setError("");
    setStatusError("");
    setNotice("");
    pending.current = false;
    setBusy(false);
    if (blocked || observedGeneration.current !== privacyGeneration) cancelOwned();
    observedGeneration.current = privacyGeneration;
    if (blocked || !foreground) return;
    let active = true;
    let reading = false;
    async function refresh() {
      if (!active || reading || pending.current) return;
      const privacy = context();
      if (!privacy) return;
      const operation = revision.current;
      reading = true;
      try {
        const next = await privateSyncRequest("private_sync_status", {}, privacy);
        if (active && current(privacy, operation) && !pending.current) {
          statusPrivacy.current = privacy;
          setStatus(next);
          setStatusError("");
        }
      } catch (cause) {
        if (active && current(privacy, operation)) setStatusError(errorText(cause));
      } finally { reading = false; }
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => { active = false; window.clearInterval(timer); };
  }, [blocked, foreground, privacyGeneration]);

  async function action(work: (privacy: PrivacyContext) => Promise<PrivateSyncStatus>, success = "", pairing = false) {
    const privacy = context();
    if (pending.current || !privacy || blocked) return;
    const operation = ++revision.current;
    pending.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const next = await work(privacy);
      if (pairing && next.session) {
        if (!current(privacy, operation)) {
          if (isPrivacyContextCurrent(privacy)) void privateSyncRequest("private_sync_pair_cancel", { sessionId: next.session.id }, privacy).catch(() => {});
          return;
        }
        ownedSession.current = { id: next.session.id, privacy };
      }
      if (!current(privacy, operation)) return;
      statusPrivacy.current = privacy;
      setStatus(next);
      setNotice(success);
      callbacks.current.onRefresh();
    } catch (cause) {
      if (current(privacy, operation)) setError(errorText(cause));
    } finally {
      if (alive.current && revision.current === operation) { pending.current = false; setBusy(false); }
    }
  }

  async function resume(positionId: string) {
    const privacy = context();
    if (!privacy || pending.current || blocked) return;
    const operation = ++revision.current;
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      const location = await privateSyncRequest("private_sync_resume", { positionId }, privacy);
      if (current(privacy, operation)) callbacks.current.onOpen(location);
    } catch (cause) {
      if (current(privacy, operation)) setError(errorText(cause));
    } finally {
      if (alive.current && revision.current === operation) { pending.current = false; setBusy(false); }
    }
  }

  async function copyInvitation() {
    const privacy = context();
    const value = status?.session?.invitation;
    const operation = revision.current;
    if (!privacy || !value) return;
    try {
      await navigator.clipboard.writeText(value);
      if (current(privacy, operation)) setNotice(t("Invitation copied. Send it only to your other device."));
    } catch (cause) {
      if (current(privacy, operation)) setError(errorText(cause));
    }
  }
  async function copyVersion(value: string) {
    const privacy = context();
    const operation = revision.current;
    if (!privacy) return;
    try {
      await navigator.clipboard.writeText(value);
      if (current(privacy, operation)) setNotice(t("Version copied to the clipboard."));
    } catch (cause) {
      if (current(privacy, operation)) setError(errorText(cause));
    }
  }

  async function resolveConflict(conflict: PrivateSyncConflict, alternativeId: string) {
    const privacy = context();
    const operation = revision.current;
    if (!privacy || !(await confirmAction(
      t("Use the selected version and discard the other versions of this note or comment? Copy anything you want to keep before continuing."),
      { confirmLabel: t("Use selected version"), danger: true },
    )) || !current(privacy, operation)) return;
    void action((captured) => privateSyncRequest("private_sync_resolve", {
      conflictId: conflict.id,
      alternativeId,
      alternativeIds: conflict.alternatives.map((alternative) => alternative.id),
    }, captured), t("Selected version saved. The other versions were discarded."));
  }

  const visibleStatus = !blocked && foreground && statusPrivacy.current && isPrivacyContextCurrent(statusPrivacy.current) ? status : null;
  const session = visibleStatus?.session;
  const disabled = busy || blocked || !foreground;
  return <section className={`private-reading-sync ${presentation === "settings" ? "settings-section panel" : "panel"}`} aria-labelledby={`${id}-heading`}>
    <div className="private-sync-heading">
      <ShieldCheck size={22} aria-hidden="true" />
      <div><h2 id={`${id}-heading`}>{presentation === "settings" ? t("Connect my devices") : t("Reading across my devices")}</h2>
      <p className="muted">{t("Private bookmarks, notes, highlights and reading positions. Separate from public archive sharing.")}</p></div>
    </div>
    {blocked ? <p className="privacy-note">{t("Private device sync is unavailable in private or read-only mode.")}</p> : !foreground ? <p className="muted">{t("Return to this app to manage private device sync.")}</p> : <>
      {error && <p className="error-text" role="alert">{error}</p>}
      {statusError && <p className="error-text" role="alert">{statusError}</p>}
      {notice && <p className="success-text" role="status">{notice}</p>}
      {!visibleStatus && !error && !statusError && <p role="status">{t("Reading private device status…")}</p>}
      {!visibleStatus && (error || statusError) && <p className="muted">{t("Device status will refresh while this app is active.")}</p>}
      {visibleStatus && <>
        {visibleStatus.error && <p className="error-text" role="alert">{visibleStatus.error}</p>}
        {presentation === "reading" ? <div className="toolbar"><button onClick={onConnect}><Link size={16} aria-hidden="true" />{t("Connect my devices")}</button></div> : <>
          <p>{t("Keep both apps open on the same local network. No cloud account, archive files or browsing tabs are transferred.")}</p>
          <label className="check-field"><input type="checkbox" checked={visibleStatus.enabled} disabled={disabled} onChange={(event) => void action((privacy) => privateSyncRequest("private_sync_configure", { enabled: event.target.checked, categories: visibleStatus.categories }, privacy))} /><span>{t("Enable private device sync")}</span></label>
          <fieldset className="private-sync-categories" disabled={disabled}>
            <legend>{t("Choose what to sync")}</legend>
            <label className="check-field"><input type="checkbox" checked={visibleStatus.categories.bookmarks} onChange={(event) => void action((privacy) => privateSyncRequest("private_sync_configure", { enabled: visibleStatus.enabled, categories: { ...visibleStatus.categories, bookmarks: event.target.checked } }, privacy))} /><span>{t("Bookmarks, notes and highlights")}</span></label>
            <label className="check-field"><input type="checkbox" checked={visibleStatus.categories.positions} onChange={(event) => void action((privacy) => privateSyncRequest("private_sync_configure", { enabled: visibleStatus.enabled, categories: { ...visibleStatus.categories, positions: event.target.checked } }, privacy))} /><span>{t("Reading positions from each device")}</span></label>
          </fieldset>
          <p className="muted">{t("Turning a category off stops its transfer; it does not delete saved reading data.")}</p>
          {visibleStatus.enabled && <div className="private-sync-pairing">
            <h3>{t("Add my other device")}</h3>
            <p>{t("Create an invitation here, then paste it into Connect my devices on your other device. Only continue when the verification code matches on both screens.")}</p>
            {!session && <>
              <button disabled={disabled} onClick={() => void action((privacy) => privateSyncRequest("private_sync_pair_start", {}, privacy), "", true)}><Link size={16} aria-hidden="true" />{t("Create private invitation")}</button>
              <form onSubmit={(event) => { event.preventDefault(); const value = invitation.trim(); if (value) void action((privacy) => privateSyncRequest("private_sync_pair_join", { invitation: value }, privacy), "", true); }}>
                <label className="field"><span>{t("Invitation from my other device")}</span><textarea maxLength={16384} value={invitation} onChange={(event) => setInvitation(event.target.value)} autoComplete="off" autoCapitalize="off" spellCheck={false} disabled={disabled} /></label>
                <button type="submit" disabled={disabled || !invitation.trim()}>{t("Connect to my other device")}</button>
              </form>
            </>}
            {session && <div className="private-sync-session">
              {session.phase === "waiting" && session.invitation && <>
                <div className="private-sync-qr" role="img" aria-label={t("Private device invitation QR code")}><QRCodeSVG value={session.invitation} size={192} marginSize={4} /></div>
                <p className="muted">{t("Use your other device’s QR reader to copy this invitation, or copy and paste it. An invitation expires automatically.")}</p>
                <label className="field"><span>{t("Private invitation")}</span><textarea readOnly value={session.invitation} rows={3} autoComplete="off" spellCheck={false} /></label>
                <button disabled={disabled} onClick={() => void copyInvitation()}><Copy size={16} aria-hidden="true" />{t("Copy invitation")}</button>
              </>}
              <p className="muted">{t("Invitation expires: {date}", { date: formatDate(session.expiresAt) })}</p>
              {session.phase === "waiting" && <p role="status">{t("Waiting for your other device…")}</p>}
              {session.phase === "verifying" && <>
                <h4>{t("Verify both screens")}</h4>
                <p>{t("Compare this code with the code shown on your other device. A device name alone does not verify ownership.")}</p>
                <p className="private-sync-code" dir="ltr" aria-label={t("Device verification code")}>{session.verificationCode}</p>
                <p>{session.peerName || t("Another device")}</p>
                <details><summary>{t("Device identity")}</summary><p className="private-sync-identity" dir="ltr">{session.peerIdentity}</p></details>
                {session.localConfirmed ? <p role="status">{t("Confirmed here. Confirm the matching code on your other device too.")}</p> : <button className="primary" disabled={disabled || !session.peerIdentity || !session.verificationCode} onClick={() => void action((privacy) => privateSyncRequest("private_sync_pair_confirm", { sessionId: session.id, peerIdentity: session.peerIdentity! }, privacy))}>{t("Codes match — confirm this device")}</button>}
              </>}
              {session.phase === "confirmed" && <p className="success-text" role="status">{t("Both devices confirmed. Your device is connected.")}</p>}
              {session.phase === "error" && !session.error && <p className="error-text" role="alert">{t("The private connection failed. Cancel this invitation and try again.")}</p>}
              {session.error && <p className="error-text" role="alert">{session.error}</p>}
              <button disabled={disabled} onClick={() => void action((privacy) => privateSyncRequest("private_sync_pair_cancel", { sessionId: session.id }, privacy))}>{session.phase === "confirmed" ? t("Close invitation") : t("Cancel connection")}</button>
            </div>}
          </div>}
          <h3>{t("My connected devices")}</h3>
          {!visibleStatus.peers.length && <p className="muted">{t("No private devices connected yet.")}</p>}
          <ul className="private-sync-devices">{visibleStatus.peers.map((peer) => <li key={peer.id}>
            <h4>{peer.name || t("Another device")}</h4>
            <details><summary>{t("Device identity")}</summary><p className="private-sync-identity" dir="ltr">{peer.identity}</p></details>
            <label className="check-field"><input type="checkbox" checked={peer.enabled} disabled={disabled} onChange={(event) => void action((privacy) => privateSyncRequest("private_sync_peer_update", { peerId: peer.id, enabled: event.target.checked }, privacy))} /><span>{t("Allow sync with this device")}</span></label>
            <label className="check-field"><input type="checkbox" checked={peer.automatic} disabled={disabled || !peer.enabled} onChange={(event) => void action((privacy) => privateSyncRequest("private_sync_peer_update", { peerId: peer.id, automatic: event.target.checked }, privacy))} /><span>{t("Automatically sync while both apps are open")}</span></label>
            <p className="muted" role="status">{peer.lastSync ? t("Last synced: {date}", { date: formatDate(peer.lastSync) }) : t("Not synced yet")}</p>
            {peer.state === "syncing" && <p role="status">{t("Syncing with this device…")}</p>}
            {peer.error && <p className="error-text" role="alert">{peer.error}</p>}
            <div className="toolbar"><button disabled={disabled || !visibleStatus.enabled || !peer.enabled || peer.state === "syncing" || (!visibleStatus.categories.bookmarks && !visibleStatus.categories.positions)} onClick={() => void action((privacy) => privateSyncRequest("private_sync_now", { peerId: peer.id }, privacy), t("Sync started. Keep both apps open."))}><RefreshCw size={16} aria-hidden="true" />{t("Sync now")}</button>
            <button className="danger" disabled={disabled} onClick={async () => {
              const privacy = context();
              if (!privacy || !(await confirmAction(t("Disconnect this device? It will no longer receive private reading data. Data already saved on either device is kept."), { confirmLabel: t("Disconnect device"), danger: true })) || !isPrivacyContextCurrent(privacy)) return;
              void action((captured) => privateSyncRequest("private_sync_unpair", { peerId: peer.id }, captured));
            }}><Unlink size={16} aria-hidden="true" />{t("Disconnect device")}</button></div>
          </li>)}</ul>
          <p className="muted">{t("Automatic sync works only while both apps are active and reachable nearby. Use Sync now for a one-time transfer.")}</p>
          <details>
            <summary>{t("Reset private sync history")}</summary>
            <p>{t("Use this if private sync reports that its history is full. Resolve conflicts first. All connected devices will be disconnected, and each must also reset private sync history before reconnecting. Your reading list and this device’s reading positions stay here; positions received from other devices are removed.")}</p>
            <p className="muted">{t("Reset may free old sync history, but active saved-data limits still apply.")}</p>
            <button className="danger" disabled={disabled} onClick={async () => {
              const privacy = context();
              if (!privacy || !(await confirmAction(t("Reset private sync history and disconnect every device? Your reading list and this device’s reading positions are kept. Positions received from other devices are removed. Reset private sync history on your other devices too before reconnecting."), { confirmLabel: t("Reset private sync history"), danger: true })) || !isPrivacyContextCurrent(privacy)) return;
              void action((captured) => privateSyncRequest("private_sync_rebootstrap", { confirmed: true }, captured), t("Private sync history reset. Reset your other devices too, then connect them again."));
            }}>{t("Reset private sync history")}</button>
            <h4>{t("If saved sync positions fill storage")}</h4>
            <p>{t("You can forget all saved sync position candidates and reset device connections. This does not delete your reading history, reading list or open tabs. Existing history and tabs will not restore the forgotten sync candidates; new reading-position changes can create new candidates. Your other devices must also reset private sync history before reconnecting.")}</p>
            <button className="danger" disabled={disabled} onClick={async () => {
              const privacy = context();
              if (!privacy || !(await confirmAction(t("Forget all saved sync positions and disconnect every device? Reading history, reading list and open tabs are kept. Saved sync position candidates are removed and will not be restored from unchanged history or tabs. Reset your other devices too before reconnecting."), { confirmLabel: t("Clear saved sync positions and reset connections"), danger: true })) || !isPrivacyContextCurrent(privacy)) return;
              void action((captured) => privateSyncRequest("private_sync_clear_positions", { confirmed: true }, captured), t("Saved sync positions cleared and devices disconnected. Reading history, reading list and open tabs are unchanged."));
            }}>{t("Clear saved sync positions and reset connections")}</button>
          </details>
        </>}
        {visibleStatus.conflicts.length > 0 && <div className="private-sync-conflicts"><h3>{t("Versions needing your choice")}</h3>{visibleStatus.conflicts.map((conflict) => <ConflictChoice key={conflict.id} conflict={conflict} disabled={disabled} onCopy={(value) => void copyVersion(value)} onChoose={(alternativeId) => void resolveConflict(conflict, alternativeId)} />)}</div>}
        <h3>{t("Continue from another device")}</h3>
        <p className="muted">{t("Open a saved position in a new reading tab. Your other tabs are kept.")}</p>
        {!visibleStatus.positions.length && <p className="muted">{t("No reading positions received yet.")}</p>}
        <ul className="private-sync-positions">{visibleStatus.positions.map((position) => <li key={position.id}><div><strong dir="auto">{position.title || t("Untitled article")}</strong><p className="muted">{position.deviceName || t("Another device")}</p>{!position.available && <p className="muted">{t("Unavailable — add this archive to your library to continue reading.")}</p>}</div><button disabled={disabled || !position.available} onClick={() => void resume(position.id)}><BookOpen size={16} aria-hidden="true" />{t("Continue reading")}</button></li>)}</ul>
      </>}
    </>}
  </section>;
}
