import { useId } from "react";
import { isIOS } from "../api";
import { displayPath } from "../display-path";
import { formatBytes, formatNumber, t } from "../i18n";
import type { MapJob } from "../types";
import { pathFileName, terminalStatus } from "./UI";

export function MapBlockProgress({ job }: { job: MapJob }) {
  const total = job.blocksTotal;
  const ready = job.blocksReady;
  const downloaded = job.blocksDownloaded;
  // Older journals have no block accounting. Do not infer it from bytes:
  // source block sizes vary and verified local/cache blocks require no download.
  if (total == null || ready == null || total <= 0) return null;
  return (
    <p className="muted" role="status">
      {t("{completed} of {total} blocks ready", {
        completed: formatNumber(ready),
        total: formatNumber(total),
      })}
      {downloaded != null && (
        <small>
          {" · "}
          {t("{downloaded} downloaded · {reused} reused", {
            downloaded: formatNumber(downloaded),
            reused: formatNumber(ready - downloaded),
          })}
        </small>
      )}
    </p>
  );
}

/** The network rate of the block being fetched. It is 0 while blocks come
 * from the cache or a local map, so nothing is shown then; there is no ETA. */
export function MapTransferRate({ job }: { job: MapJob }) {
  const speed = job.speed ?? 0;
  if (terminalStatus(job.status) || job.status === "paused" || !Number.isFinite(speed) || speed <= 0) return null;
  return (
    <p className="muted map-transfer-rate">
      {t("{speed}/s", { speed: formatBytes(speed) })}
    </p>
  );
}

/** Files a Delete kept. Delete retries those it can prove are its own; the
 * others are only reported. Dismissing one removes only its warning; the file
 * is never touched. */
export function MapRecoveryPaths({ job, disabled, onDismiss }: {
  job: MapJob;
  disabled: boolean;
  onDismiss: (path: string) => void;
}) {
  const id = useId();
  const paths = job.recoveryPaths ?? [];
  if (!paths.length) return null;
  return (
    <div className="map-recovery">
      <p className="muted" id={`${id}-note`}>
        {t("Kiwix kept these files. Press Delete to retry the ones it can prove are its own; remove any others yourself, then dismiss this warning.")}
      </p>
      <ul>
        {paths.map((path, index) => (
          <li key={path}>
            <span className="file-path" id={`${id}-${index}`} dir="ltr">
              <bdi>{isIOS ? pathFileName(path) : displayPath(path)}</bdi>
            </span>
            <button type="button" disabled={disabled}
              aria-describedby={`${id}-${index} ${id}-note`}
              onClick={() => onDismiss(path)}>
              {t("Dismiss (keeps the file)")}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
