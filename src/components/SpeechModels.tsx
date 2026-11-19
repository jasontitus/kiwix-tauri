import { useEffect, useState } from "react";
import { useVoiceModelStatus, type RetainedVoiceFiles } from "./useVoiceModelStatus";
import { Download, RefreshCw, Trash2 } from "lucide-react";
import { api, isNative } from "../api";
import { formatBytes, t } from "../i18n";

/**
 * The standard edition downloads the pinned model on request. An offline
 * Android edition can install the same files from its signed package.
 */
export default function SpeechModels() {
  const { status, readError, refresh } = useVoiceModelStatus(isNative);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const preparing = status?.downloading;
    if ((!status?.available || status.performance !== "unknown" || status.performanceError) && !preparing) return;
    const timer = window.setInterval(() => { void refresh(); }, 1500);
    return () => window.clearInterval(timer);
  }, [refresh, status?.available, status?.downloading, status?.performance, status?.performanceError]);

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
      await refresh();
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
      const preserved = await api<RetainedVoiceFiles>("kokoro_delete");
      await refresh(preserved);
    } catch (value) {
      await refresh();
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  }

  if (!isNative) return null;
  if (!status) return (
    <div className="speech-models">
      {readError ? (
        <>
          <p className="inline-error" role="alert">{readError}</p>
          <button type="button" className="secondary" onClick={() => void refresh()}>{t("Try again")}</button>
        </>
      ) : <p className="muted" role="status">{t("Checking natural voices…")}</p>}
    </div>
  );
  const message = error || readError || status?.error;
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
      {message && <p className="inline-error" role="alert">{message}</p>}
    </div>
  );
}
