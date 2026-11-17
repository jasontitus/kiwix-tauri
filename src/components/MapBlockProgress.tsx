import { formatNumber, t } from "../i18n";
import type { MapJob } from "../types";

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
