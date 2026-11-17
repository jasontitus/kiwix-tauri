import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Download, X } from "lucide-react";
import { api, isNative } from "../api";
import { formatBytes, t } from "../i18n";
import { errorText } from "./UI";

interface KokoroStatus {
  available: boolean;
  downloading: boolean;
  downloaded: number;
  total: number;
  error?: string | null;
  bundled?: boolean;
}

interface ModelEvent {
  state: "downloading" | "installed" | "error";
  downloaded?: number;
  total?: number;
  error?: string | null;
}

interface Props {
  storageKey: string;
  enabled: boolean;
}

/** One-time, non-blocking offer for the optional natural-voice pack. */
export default function KokoroDownloadPrompt({ storageKey, enabled }: Props) {
  const [status, setStatus] = useState<KokoroStatus | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [online, setOnline] = useState(() => typeof navigator === "undefined" || navigator.onLine);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isNative || !enabled) return;
    try { setDismissed(localStorage.getItem(storageKey) === "1"); } catch { /* unavailable storage */ }
    const updateOnline = () => setOnline(navigator.onLine);
    window.addEventListener("online", updateOnline);
    window.addEventListener("offline", updateOnline);
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    void api<KokoroStatus>("kokoro_status").then((next) => {
      if (!disposed) { setStatus(next); if (next.error) setError(next.error); }
    }).catch(() => { /* Settings still exposes the manual control. */ });
    void listen<ModelEvent>("kiwix-speech-model", ({ payload }) => {
      if (disposed) return;
      setStatus((previous) => ({
        available: payload.state === "installed" ? true : previous?.available ?? false,
        downloading: payload.state === "downloading",
        downloaded: payload.downloaded ?? previous?.downloaded ?? 0,
        total: payload.total ?? previous?.total ?? 0,
        error: payload.error ?? null,
        bundled: previous?.bundled ?? false,
      }));
      if (payload.state === "error") setError(payload.error || t("The voice model download failed."));
      if (payload.state === "installed") setError("");
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    });
    return () => {
      disposed = true;
      window.removeEventListener("online", updateOnline);
      window.removeEventListener("offline", updateOnline);
      for (const unlisten of unlisteners) unlisten();
    };
  }, [enabled, storageKey]);

  if (!isNative || !enabled || dismissed || !online || status === null || status.available || status.bundled) return null;

  function dismiss() {
    try { localStorage.setItem(storageKey, "1"); } catch { /* unavailable storage */ }
    setDismissed(true);
  }

  async function download() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("kokoro_download");
      setStatus((previous) => previous ? { ...previous, downloading: true } : previous);
    } catch (value) {
      const message = errorText(value);
      if (!/cancel/i.test(String(value))) setError(message);
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    if (busy) return;
    setBusy(true);
    try { await api("kokoro_cancel"); }
    catch (value) { setError(errorText(value)); }
    finally { setBusy(false); }
  }

  if (status.downloading) {
    const percent = status.total ? Math.min(100, Math.round((status.downloaded / status.total) * 100)) : 0;
    return (
      <aside className="panel speech-prompt" role="status">
        <div>
          <strong>{t("Downloading natural voices… {downloaded} of {total}", {
            downloaded: formatBytes(status.downloaded),
            total: formatBytes(status.total || 181_886_349),
          })}</strong>
          <progress value={status.downloaded} max={status.total || 1} aria-label={t("Natural voice download")} />
          <span className="muted">{percent}%</span>
        </div>
        <div className="speech-prompt-actions">
          <button type="button" className="secondary" onClick={() => void cancel()} disabled={busy}>
            {t("Cancel download")}
          </button>
        </div>
        {error && <p className="inline-error" role="alert">{error}</p>}
      </aside>
    );
  }

  return (
    <aside className="panel speech-prompt" role="status">
      <div>
        <strong>{t("Download more natural-sounding voices?")}</strong>
        <p className="muted">
          {t("A one-time {size} download from GitHub, then it works entirely offline. System voices remain available. Use Wi-Fi or allow metered downloads.", {
            size: formatBytes(status.total || 181_886_349),
          })}
        </p>
      </div>
      <div className="speech-prompt-actions">
        <button type="button" className="primary" onClick={() => void download()} disabled={busy}>
          <Download size={16} /> {t("Download natural voices")}
        </button>
        <button type="button" className="secondary" onClick={dismiss} disabled={busy}>
          <X size={16} /> {t("Not now")}
        </button>
      </div>
      {error && <p className="inline-error" role="alert">{error}</p>}
    </aside>
  );
}
