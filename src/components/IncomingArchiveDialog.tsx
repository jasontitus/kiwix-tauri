import { formatBytes, t } from "../i18n";
import { Modal } from "./UI";

export type IncomingArchiveRequest = {
  requestId: string;
  count: number;
  fileNames?: string[];
  canReadOriginals?: boolean;
};

export default function IncomingArchiveDialog({
  request,
  sessionOnly,
  busy,
  copiedBytes,
  onCancel,
  onOpenOriginals,
  onSaveCopy,
  onChooseFiles,
}: {
  request: IncomingArchiveRequest;
  sessionOnly: boolean;
  busy: boolean;
  copiedBytes: number | null;
  onCancel: () => void;
  onOpenOriginals: () => void;
  onSaveCopy: () => void;
  onChooseFiles: () => void;
}) {
  // A hint may route the user to Files; it never grants authority. Native
  // registration still checks the actual handoff and current privacy policy.
  const chooseOriginals = request.canReadOriginals !== true;
  const name = request.count === 1 ? request.fileNames?.[0] : undefined;
  const title = sessionOnly
    ? name ? t("Open {name} for this session?", { name })
      : t("Open {count} archives for this session?", { count: request.count })
    : name ? t("Add {name} to your library?", { name })
      : t("Add {count} archives to your library?", { count: request.count });
  return (
    <Modal title={title} onClose={onCancel}>
      <p>{sessionOnly
        ? t("Nothing will be added to your saved library. Access ends when this session closes.")
        : t("Your files will stay in their current location. No additional copy will be saved.")}</p>
      {chooseOriginals && <p>{t("To keep files in their current location, choose them in Files instead.")}</p>}
      {!sessionOnly && (
        <details>
          <summary>{t("Storage options")}</summary>
          <p>{t("Kiwix will save a copy for offline reading. Your original files will not be changed.")}</p>
          <button disabled={busy} onClick={(event) => {
            if (event.isTrusted) onSaveCopy();
          }}>{t("Save a copy in Kiwix")}</button>
        </details>
      )}
      {copiedBytes !== null && <p role="status">{t("Copied {size}", { size: formatBytes(copiedBytes) })}</p>}
      <div className="modal-actions">
        <button disabled={busy} onClick={onCancel}>{t("Cancel")}</button>
        <button className="primary" disabled={busy} onClick={(event) => {
          if (event.isTrusted) {
            if (chooseOriginals) onChooseFiles();
            else onOpenOriginals();
          }
        }}>{busy ? t("Adding…") : chooseOriginals ? t("Choose files…") : sessionOnly ? t("Open for this session") : t("Add to library")}</button>
      </div>
    </Modal>
  );
}
