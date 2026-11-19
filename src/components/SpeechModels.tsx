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
  bundleCheckFailed?: boolean;
  removalPending?: boolean;
  repairAvailable?: boolean;
  legacyRetained?: boolean;
  legacyAudioRetained?: boolean;
}

interface ModelEvent {
  state: "downloading" | "installed" | "error";
  downloaded?: number;
  total?: number;
  error?: string | null;
}

/**
 * The standard edition downloads the pinned model on request. An offline
 * Android edition can install the same files from its signed package.
 */
export default function SpeechModels() {
  const [status, setStatus] = useState<KokoroStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function refresh(preserved?: Pick<KokoroStatus, "legacyRetained" | "legacyAudioRetained">) {
    try {
      const next = await api<KokoroStatus>("kokoro_status");
      setStatus({
        ...next,
        legacyRetained: next.legacyRetained || preserved?.legacyRetained,
        legacyAudioRetained: next.legacyAudioRetained || preserved?.legacyAudioRetained,
      });
      setError(next.error || "");
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
        bundleCheckFailed: previous?.bundleCheckFailed ?? false,
        removalPending: previous?.removalPending ?? false,
        repairAvailable: previous?.repairAvailable ?? false,
        legacyRetained: previous?.legacyRetained,
        legacyAudioRetained: previous?.legacyAudioRetained,
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
    const preparingBundled = status?.bundled === true && status.downloading;
    if ((!status?.available || status.performance !== "unknown" || status.performanceError) && !preparingBundled) return;
    const timer = window.setInterval(() => { void refresh(); }, 1500);
    return () => window.clearInterval(timer);
  }, [status?.available, status?.bundled, status?.downloading, status?.performance, status?.performanceError]);

  async function download() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await api("kokoro_download");
      await refresh();
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
      await api("kokoro_probe");
      await refresh();
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
      const preserved = await api<Pick<KokoroStatus, "legacyRetained" | "legacyAudioRetained">>("kokoro_delete");
      await refresh(preserved);
    } catch (value) {
      await refresh();
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  if (!isNative) return null;
  const percent = status?.total ? Math.min(100, Math.round((status.downloaded / status.total) * 100)) : 0;
  return (
    <div className="speech-models">
      {status && !status.bundled && <p className="muted">
        {t("Natural voices sound clearer than the system voices and work entirely offline. The English voice model is a one-time download of about {size}.", {
          size: formatBytes(status?.total || 181_886_349),
        })}
      </p>}
      {status?.removalPending ? (
        <>
          <p className="muted" role="status">{t("Natural voice removal was interrupted.")}</p>
          <button type="button" className="secondary" onClick={() => void remove()} disabled={busy}>
            <Trash2 size={16} /> {t("Retry removal")}
          </button>
        </>
      ) : status?.bundleCheckFailed && (!status.available || status.repairAvailable) ? (
        <button type="button" className="secondary" onClick={() => void refresh()}>{t("Try again")}</button>
      ) : status?.repairAvailable && !status.downloading ? (
        <button type="button" className="primary" onClick={() => void download()} disabled={busy}>
          {status.bundled ? t("Repair natural voices") : t("Download and repair natural voices")}
        </button>
      ) : status?.available && !status.downloading ? (
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
      ) : status?.bundled ? (
        status.downloading ? (
          <>
            <p className="muted" role="status">{t("Preparing bundled natural voices…")}</p>
            <button type="button" className="secondary" onClick={() => void cancel()} disabled={busy}>{t("Cancel")}</button>
          </>
        ) : (
          <button type="button" className="primary" onClick={() => void download()} disabled={busy}>
            {status.error ? t("Try again") : t("Install natural voices")}
          </button>
        )
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
      ) : (
        <button type="button" className="primary" onClick={() => void download()} disabled={busy}>
          {busy ? <RefreshCw size={16} className="spin" /> : <Download size={16} />}
          {t("Download natural voices")}
        </button>
      )}
      {status?.legacyRetained && <p className="muted" role="status">{t("Files from an earlier voice model remain in app storage.")}</p>}
      {status?.legacyAudioRetained && <p className="muted" role="status">{t("Some cached voice audio remains in app storage because its ownership could not be verified.")}</p>}
      {error && <p className="inline-error" role="alert">{error}</p>}
    </div>
  );
}
