import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { RefreshCw } from "lucide-react";
import {
  api,
  capturePrivacyContext,
  isAndroid,
  isDesktop,
  isIOS,
} from "../api";
import { formatBytes, getLocale, t } from "../i18n";
import type { UpdateProgress, UpdateStatus } from "../types";

interface Props {
  currentVersion: string | null;
  onError: (error: unknown) => void;
}

function checkedLabel(value: string | null): string {
  if (!value) return t("Not yet");
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return t("Not yet");
  return date.toLocaleString(getLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function progressLabel(progress: UpdateProgress): string {
  if (progress.phase === "verifying") return t("Confirming the update came from Kiwix…");
  if (progress.phase === "installing") return t("Installing update…");
  const total = progress.total;
  return total
    ? t("Downloaded {downloaded} of {total}", {
        downloaded: formatBytes(progress.downloaded),
        total: formatBytes(total),
      })
    : t("Downloaded {downloaded}", {
        downloaded: formatBytes(progress.downloaded),
      });
}

/**
 * Signed desktop updates. Mobile builds update through their store, so this
 * panel states that plainly instead of offering a button that cannot work.
 */
export default function UpdatesSettings({ currentVersion, onError }: Props) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api<UpdateStatus>("update_status"));
      setUnavailable(false);
    } catch {
      setUnavailable(true);
    }
  }, []);

  useEffect(() => {
    if (!isDesktop) return;
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    void listen<UpdateProgress | null>("kiwix-update-progress", (event) => {
      if (!disposed) setProgress(event.payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen("kiwix-update-available", () => {
      if (!disposed) void refresh();
    }).then((unlisten) => unlisteners.push(unlisten));
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [refresh]);

  if (isIOS || isAndroid) {
    const store = isIOS ? t("App Store") : t("Google Play");
    return (
      <section className="settings-section panel">
        <h2>{t("Updates")}</h2>
        <p className="muted">
          {isIOS
            ? t(
                "iPhone and iPad apps update through the App Store. This build cannot update itself. Open the App Store to install the latest version.",
              )
            : t(
                "Android apps update through Google Play or the installer that supplied Kiwix. This build cannot update itself.",
              )}
        </p>
        <p className="muted">
          {t("Current version: {version}", {
            version: status?.currentVersion ?? currentVersion ?? t("Unknown"),
          })}
        </p>
        <p className="muted">{store}</p>
      </section>
    );
  }

  if (!isDesktop) return null;

  async function checkNow() {
    if (checking || installing) return;
    const context = capturePrivacyContext();
    if (!context) return;
    setChecking(true);
    setConfirming(false);
    try {
      const next = await api<UpdateStatus>("update_check", {
        privacyGeneration: context.generation,
      });
      setStatus(next);
    } catch (error) {
      onError(error);
      void refresh();
    } finally {
      setChecking(false);
    }
  }

  async function installNow() {
    if (!status?.available || installing) return;
    const context = capturePrivacyContext();
    if (!context) return;
    setInstalling(true);
    setConfirming(false);
    try {
      await api("update_install", {
        version: status.available.version,
        confirmed: true,
        privacyGeneration: context.generation,
      });
    } catch (error) {
      onError(error);
      setInstalling(false);
      void refresh();
    }
  }

  const blocked = status?.blockedReason;
  const checkDisabled =
    checking ||
    installing ||
    !status ||
    !status.configured ||
    !status.supported ||
    !!blocked ||
    status.busy;

  return (
    <section className="settings-section panel">
      <h2>{t("Updates")}</h2>
      {unavailable || !status ? (
        <p className="muted">
          {t("Updates are not available until Kiwix finishes starting. Try again in a moment.")}
        </p>
      ) : (
        <>
          <dl className="facts">
            <div>
              <dt>{t("Current version")}</dt>
              <dd>{status.currentVersion}</dd>
            </div>
            <div>
              <dt>{t("Channel")}</dt>
              <dd>{t("Stable")}</dd>
            </div>
            <div>
              <dt>{t("Last checked")}</dt>
              <dd>{checkedLabel(status.lastChecked)}</dd>
            </div>
            {status.automaticChecks && (
              <div>
                <dt>{t("Next automatic check")}</dt>
                <dd>
                  {status.nextCheckAt
                    ? checkedLabel(status.nextCheckAt)
                    : t("After the next launch")}
                </dd>
              </div>
            )}
          </dl>
          {status.automaticChecks && (
            <p className="muted">
              {t(
                "Kiwix checks at most once a day and only downloads an update when you choose to install it.",
              )}
            </p>
          )}
          {blocked && <p className="warning-text">{blocked}</p>}
          {!status.configured && status.configurationNote && (
            <p className="muted">{status.configurationNote}</p>
          )}
          {!status.supported && <p className="muted">{status.explanation}</p>}
          {status.lastError && (
            <p className="muted">
              {t("The last check did not finish: {reason}", {
                reason: status.lastError,
              })}
            </p>
          )}
          <div className="toolbar">
            <button
              disabled={checkDisabled}
              onClick={() => void checkNow()}
              type="button"
            >
              <RefreshCw size={16} />
              {checking ? t("Checking…") : t("Check for updates")}
            </button>
          </div>
          {progress && (
            <div className="download-progress" role="status" aria-live="polite">
              <progress
                value={progress.total ? progress.downloaded : undefined}
                max={progress.total ?? undefined}
              />
              <span className="muted">{progressLabel(progress)}</span>
            </div>
          )}
          {status.available ? (
            <div className="update-result" role="status">
              <h3>
                {t("Version {version} is available.", {
                  version: status.available.version,
                })}
              </h3>
              {status.available.notes && (
                <>
                  <h4>{t("Release notes")}</h4>
                  <div className="update-notes">{status.available.notes}</div>
                </>
              )}
              {status.installBlockedReason && (
                <p className="warning-text">{status.installBlockedReason}</p>
              )}
              {confirming ? (
                <>
                  <p>
                    {t(
                      "Install version {version}? Kiwix will close and reopen to finish.",
                      { version: status.available.version },
                    )}
                  </p>
                  <div className="modal-actions">
                    <button
                      disabled={installing}
                      onClick={() => void installNow()}
                      type="button"
                    >
                      {installing ? t("Installing…") : t("Install and restart")}
                    </button>
                    <button
                      disabled={installing}
                      onClick={() => setConfirming(false)}
                      type="button"
                    >
                      {t("Cancel")}
                    </button>
                  </div>
                </>
              ) : (
                <button
                  disabled={
                    installing || !!status.installBlockedReason || !!blocked
                  }
                  onClick={() => setConfirming(true)}
                  type="button"
                >
                  {t("Install update")}
                </button>
              )}
            </div>
          ) : (
            status.lastChecked &&
            !checking && (
              <p className="muted">{t("You have the latest version.")}</p>
            )
          )}
          {!status.available && status.lastAvailableVersion && (
            <p className="muted">
              {t("Version {version} was available at the last check.", {
                version: status.lastAvailableVersion,
              })}
            </p>
          )}
        </>
      )}
    </section>
  );
}
