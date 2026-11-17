import { useId, useState } from "react";
import { FolderOpen } from "lucide-react";
import { api, capturePrivacyContext, isAndroid, isPrivacyContextCurrent } from "../api";
import { formatBytes, t } from "../i18n";
import type { Book } from "../types";
import { Modal } from "./UI";

interface Props {
  books: Book[];
  disabled?: boolean;
  onChanged: () => Promise<void> | void;
  onError: (error: unknown) => void;
}
interface Location {
  path: string;
  label: string;
  kind: string;
  available: boolean;
  freeBytes: number | null;
}
interface Result {
  cleanupPending?: string | null;
}

/** Selected installations only; the default download location is untouched. */
export default function SelectedStorageActions({ books, disabled = false, onChanged, onError }: Props) {
  const fieldId = useId();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [locations, setLocations] = useState<Location[] | null>(null);
  const [target, setTarget] = useState("");
  const [removeOld, setRemoveOld] = useState(false);
  const [notice, setNotice] = useState("");
  if (!isAndroid || books.length === 0) return null;
  const unavailable = disabled || busy || books.some((book) => !book.available || (!book.managed && !book.sourceReference));

  async function chooseLocation() {
    const privacy = capturePrivacyContext();
    if (unavailable || !privacy || privacy.privateMode || privacy.readOnly) return;
    setOpen(true);
    setBusy(true);
    setLocations(null);
    setTarget("");
    setRemoveOld(false);
    setNotice("");
    try {
      const result = await api<{ locations: Location[] }>("mobile_storage_locations", { privacyGeneration: privacy.generation });
      if (isPrivacyContextCurrent(privacy)) setLocations(result.locations);
      else setOpen(false);
    } catch (error) {
      setOpen(false);
      onError(error);
    } finally {
      setBusy(false);
    }
  }

  async function relocate() {
    const privacy = capturePrivacyContext();
    if (!target || unavailable || !privacy || privacy.privateMode || privacy.readOnly) return;
    setBusy(true);
    try {
      const result = await api<Result>("library_files_relocate", {
        bookIds: books.map((book) => book.id), target, removeOld, confirm: true,
        privacyGeneration: privacy.generation,
      });
      setOpen(false);
      if (isPrivacyContextCurrent(privacy)) {
        setNotice(result.cleanupPending
          ? t("The managed copies are ready. Some originals were kept; restart Kiwix to finish cleanup.")
          : removeOld ? t("Selected archives moved.") : t("Selected archives copied. Originals were kept."));
        await onChanged();
        if (result.cleanupPending) {
          // Selection refresh can unmount this control, so retain the cleanup
          // warning in the shell as well as its local status.
          onError(t("The managed copies are ready. Some originals were kept; restart Kiwix to finish cleanup."));
        }
      }
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  }

  return <>
    <button disabled={unavailable} onClick={() => void chooseLocation()}>
      <FolderOpen size={16} aria-hidden="true" />{t("Copy or move selected archives")}
    </button>
    {notice && <p className="muted" role="status">{notice}</p>}
    {open && <Modal title={t("Copy or move selected archives")} onClose={() => { if (!busy) setOpen(false); }}>
      <p>{t("Only the selected archives will change location. Other archives and the download location stay where they are.")}</p>
      <label className="field" htmlFor={fieldId}>
        <span>{t("Storage location")}</span>
        <select id={fieldId} value={target} disabled={busy || disabled || locations === null} onChange={(event) => setTarget(event.target.value)}>
          <option value="">{t("Choose a storage location")}</option>
          {locations?.map((location) => <option key={location.path} value={location.path} disabled={!location.available}>
            {location.kind === "internal" ? t("Internal app storage") : location.label || t("External app storage")}
            {location.freeBytes === null ? "" : ` · ${t("{free} free", { free: formatBytes(location.freeBytes) })}`}
          </option>)}
        </select>
      </label>
      {locations === null && <p className="muted" role="status">{t("Reading storage locations…")}</p>}
      <label className="check-field">
        <input type="checkbox" checked={removeOld} disabled={busy || disabled} onChange={(event) => setRemoveOld(event.target.checked)} />
        <span>{t("Move: remove originals after verifying the managed copies")}</span>
      </label>
      <p className="muted">{removeOld
        ? t("Provider originals are removed only when the provider grants delete permission. If permission is unavailable, choose Copy instead.")
        : t("Copy keeps the originals and opens the selected archives from their new managed copies.")}</p>
      <div className="dialog-actions">
        <button disabled={busy} onClick={() => setOpen(false)}>{t("Cancel")}</button>
        <button className="primary" disabled={unavailable || !target} onClick={() => void relocate()}>
          {busy ? t("Copying and verifying…") : removeOld ? t("Move selected archives") : t("Copy selected archives")}
        </button>
      </div>
    </Modal>}
  </>;
}
