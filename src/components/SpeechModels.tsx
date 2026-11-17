import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw, Trash2 } from "lucide-react";
import { api, isNative } from "../api";
import { formatBytes, t } from "../i18n";

interface KokoroStatus {
  available: boolean;
  downloading: boolean;
  downloaded: number;
  total: number;
  voices: unknown[];
  error?: string | null;
  performance?: "unknown" | "realtime" | "slow";
  probing?: boolean;
  performanceRatio?: number | null;
  performanceError?: string | null;
  bundled?: boolean;
}

interface ModelEvent {
  state: "downloading" | "installed" | "error";
  downloaded?: number;
  total?: number;
  error?: string | null;
}

/**
 * Explicit model installation for offline neural read-aloud. The model is not
 * bundled: it is a large pinned download, and the platform voice path remains
 * usable until the user chooses it.
 */
export default function SpeechModels() {
  const [status, setStatus] = useState<KokoroStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function refresh() {
    try {
      const next = await api<KokoroStatus>("kokoro_status");
      setStatus(next);
      if (next.error) setError(next.error);
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    }
  }

  useEffect(() => {
    // The headless browser preview has no Tauri event bridge or model store.
    if (!isNative) return;
    void refresh();
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    void listen<ModelEvent>("kiwix-speech-model", (event) => {
      if (disposed) return;
      const payload = event.payload;
      setStatus((previous) => ({
        available: payload.state === "installed" ? true : previous?.available ?? false,
        downloading: payload.state === "downloading",
        downloaded: payload.downloaded ?? previous?.downloaded ?? 0,
        total: payload.total ?? previous?.total ?? 0,
        voices: previous?.voices ?? [],
        error: payload.error ?? null,
        performance: previous?.performance ?? "unknown",
        probing: previous?.probing ?? false,
        performanceRatio: previous?.performanceRatio ?? null,
        performanceError: previous?.performanceError ?? null,
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
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  useEffect(() => {
    const preparingBundled = status?.bundled === true && !status.available;
    if ((!status?.available || status.performance !== "unknown" || status.performanceError) && !preparingBundled) return;
    const timer = window.setInterval(() => { void refresh(); }, 1500);
    return () => window.clearInterval(timer);
  }, [status?.available, status?.bundled, status?.performance, status?.performanceError]);

  async function download() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("kokoro_download");
      setStatus((previous) => previous ? { ...previous, downloading: true } : previous);
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    if (busy) return;
    setBusy(true);
    try {
      await api("kokoro_cancel");
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  async function checkAgain() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      const next = await api<KokoroStatus>("kokoro_probe");
      setStatus(next);
      if (next.error) setError(next.error);
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("kokoro_delete");
      await refresh();
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  if (!isNative) return null;
  const percent = status?.total ? Math.min(100, Math.round((status.downloaded / status.total) * 100)) : 0;
  return (
    <div className="speech-models">
      <p className="muted">
        {t("Natural voices sound clearer than the system voices and work entirely offline. The English voice model is a one-time download of about {size}.", {
          size: formatBytes(status?.total || 181_886_349),
        })}
      </p>
      {status?.available ? (
        <>
          {status.performance === "slow" ? (
            <p className="muted" role="status">
              {t("Natural voices are installed but disabled on this device because synthesis cannot keep up in real time. System voices will be used.")}
            </p>
          ) : status.performance === "unknown" ? (
            <p className="muted" role="status">
              {status.probing
                ? t("Checking whether this device can synthesize natural voices in real time…")
                : t("Natural voice performance could not be checked. System voices will be used.")}
            </p>
          ) : (
            <p className="muted" role="status">
              {t("Natural read-aloud is installed. Reopen the article, then choose a voice beginning with “Natural” in the reader’s Read aloud panel.")}
            </p>
          )}
          {status.performance !== "realtime" && (
            <button type="button" className="secondary" onClick={() => void checkAgain()} disabled={busy || status.probing}>
              <RefreshCw size={16} className={busy ? "spin" : undefined} /> {t("Check this device again")}
            </button>
          )}
          <button type="button" className="secondary" onClick={() => void remove()} disabled={busy}>
            <Trash2 size={16} /> {t("Remove natural voice model")}
          </button>
        </>
      ) : status?.downloading ? (
        <>
          <p role="status">
            {t("Downloading natural voices… {downloaded} of {total}", {
              downloaded: formatBytes(status.downloaded),
              total: formatBytes(status.total),
            })}
          </p>
          <progress value={status.downloaded} max={status.total || 1} aria-label={t("Natural voice download")} />
          <p className="muted">{percent}%</p>
          <button type="button" className="secondary" onClick={() => void cancel()} disabled={busy}>
            {t("Cancel download")}
          </button>
        </>
      ) : status?.bundled ? (
        <p className="muted" role="status">
          {t("Preparing bundled natural voices…")}
        </p>
      ) : (
        <button type="button" className="primary" onClick={() => void download()} disabled={busy}>
          {busy ? <RefreshCw size={16} className="spin" /> : <Download size={16} />}
          {t("Download natural voices")}
        </button>
      )}
      {error && <p className="inline-error" role="alert">{error}</p>}
    </div>
  );
}
