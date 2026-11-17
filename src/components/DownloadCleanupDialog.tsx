import { useEffect, useRef } from "react";
import { t } from "../i18n";
import { Modal } from "./UI";

export function DownloadCleanupDialog({ name, title, busy, error, description, archiveNames, removeFromListDisabled = false, removeFromListLabel, onCancel, onRemoveFromList, onDeleteFiles }: {
  name: string;
  title?: string;
  busy: boolean;
  error?: string;
  description?: string;
  archiveNames?: string[];
  removeFromListDisabled?: boolean;
  removeFromListLabel?: string;
  onCancel: () => void;
  onRemoveFromList: () => void;
  onDeleteFiles: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus({ preventScroll: true }); }, []);
  return (
    <Modal title={title ?? t("Delete download “{name}”?", { name })} presentation="confirmation" onClose={() => { if (!busy) onCancel(); }}>
      <p>{description ?? t("Remove this download from the list and keep its files, or delete the download and its files to free storage.")}</p>
      <p>{t("Deleting files also removes completed archives from your library.")}</p>
      {!!archiveNames?.length && (
        <div>
          <p>{t("Completed archives to delete:")}</p>
          <ul>{archiveNames.map((archive, index) => <li key={index}><bdi>{archive}</bdi></li>)}</ul>
        </div>
      )}
      {busy && <p role="status">{t("Working… Large archives may take time to prepare.")}</p>}
      {error && <p className="inline-error" role="alert">{error}</p>}
      <div className="modal-actions download-cleanup-actions">
        <button ref={cancel} type="button" disabled={busy} onClick={onCancel}>{t("Cancel")}</button>
        <button type="button" disabled={busy || removeFromListDisabled} onClick={(event) => { if (event.nativeEvent.isTrusted) onRemoveFromList(); }}>{removeFromListLabel ?? t("Remove from list")}</button>
        <button type="button" className="danger" disabled={busy} onClick={(event) => { if (event.nativeEvent.isTrusted) onDeleteFiles(); }}>{t("Delete download and files")}</button>
      </div>
    </Modal>
  );
}
