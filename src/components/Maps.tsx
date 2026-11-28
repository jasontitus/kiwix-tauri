import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine,
  BookOpen,
  Map,
  Pause,
  Play,
  Search,
  Trash2,
} from "lucide-react";
import {
  api,
  capturePrivacyContext,
  confirmDiskSpaceWarning,
  isIOS,
  isPrivacyContextCurrent,
  parseDiskSpaceWarning,
  type PrivacyContext,
} from "../api";
import { diagnostics, reportError } from "../diagnostics";
import {
  t,
  formatBytes,
  formatNumber,
  languageName,
  useLocale,
  getLocale,
} from "../i18n";
import type { Book, DownloadAdmission, DownloadTarget, MapCatalog, MapJob, MapPreparation, MapRecipeMetadata, MapRegion, MapStartRequest, MapVersion } from "../types";
import { cleanupDownloadBooks, mapDownloadConfiguration, readyDownloadBooks } from "../download-state";
import { displayPath } from "../display-path";
import { MapBlockProgress, MapRecoveryPaths, MapTransferRate } from "./MapBlockProgress";
import { DownloadCleanupDialog } from "./DownloadCleanupDialog";
import { SwipeRow } from "./SwipeRow";
import areaGroups from "../map-data/area-groups.json";
import regionLanguages from "../map-data/region-languages.json";
import {
  Empty,
  ErrorNotice,
  Loading,
  Modal,
  PageHeader,
  PagedResults,
  confirmAction,
  errorText,
  jobStatus,
  terminalStatus,
  useDocumentVisible,
  type SurfaceProps,
} from "./UI";
const groups: Record<string, string> = areaGroups;
const localLanguages: Record<string, string[]> = regionLanguages;
// Region menus repeat a handful of language tags across hundreds of areas; a
// process-wide memo keeps Intl.Locale construction off the render path.
const canonicalTags = new globalThis.Map<string, string>();
function canonical(code: string) {
  const cached = canonicalTags.get(code);
  if (cached !== undefined) return cached;
  let result: string;
  try {
    result = new Intl.Locale(code.replaceAll("_", "-")).toString();
  } catch {
    result = code;
  }
  if (canonicalTags.size < 512) canonicalTags.set(code, result);
  return result;
}
function preferredVersion(versions: MapVersion[], locale: string) {
  const code = canonical(locale);
  const exact = versions.find(
    (v) => canonical(v.lang).toLowerCase() === code.toLowerCase(),
  );
  if (exact) return exact.lang;
  try {
    const requested = new Intl.Locale(code);
    const minimal = requested.minimize().toString();
    const maximal = requested.maximize().toString();
    let best: MapVersion | undefined;
    let bestScore = 0;
    for (const version of versions) {
      try {
        const candidate = new Intl.Locale(canonical(version.lang));
        if (candidate.language !== requested.language) continue;
        const score =
          candidate.minimize().toString() === minimal
            ? 4
            : candidate.maximize().toString() === maximal
              ? 3
              : candidate.script === requested.script
                ? 2
                : 1;
        if (score > bestScore) {
          best = version;
          bestScore = score;
        }
      } catch {
        /* An invalid catalog tag must not hide other offered versions. */
      }
    }
    if (best) return best.lang;
  } catch {
    /* Fall back to English or the first offered language. */
  }
  return (
    versions.find((v) => baseLanguage(v.lang) === "en")?.lang ||
    versions[0]?.lang ||
    ""
  );
}
function baseLanguage(code: string) {
  try {
    return new Intl.Locale(canonical(code)).language;
  } catch {
    return code.replaceAll("_", "-").toLowerCase().split("-")[0];
  }
}
function versionLabel(version: MapVersion) {
  const translated = languageName(version.lang);
  return !version.label || translated === version.label
    ? translated
    : `${translated} (${version.label})`;
}
function savedMapLabel(book: Book, regions: MapRegion[]) {
  let title: string | undefined;
  let language: string | undefined;
  for (const region of regions) {
    const version = region.menu.find((v) => v.name === book.name);
    if (version) {
      title = region.title;
      language = versionLabel(version);
      break;
    }
  }
  if (!title) {
    const parts = /^osm_([^_]+)_(.+)$/.exec(book.name);
    if (parts) {
      const words = parts[2].split(/[_-]+/).filter(Boolean);
      if (words.length) {
        title = words
          .map(
            (word) =>
              word.charAt(0).toLocaleUpperCase() +
              word.slice(1).toLocaleLowerCase(),
          )
          .join(" ");
        language = languageName(parts[1]);
      }
    }
  }
  title ??= book.title;
  language ??= book.language
    .split(",")
    .map((code) => languageName(code.trim()))
    .filter(Boolean)
    .join(", ");
  return {
    title,
    subtitle: [language, title === book.title ? "" : book.title]
      .filter(Boolean)
      .join(" · "),
  };
}
type MapSelectionRequest = Omit<MapStartRequest, "decisionToken" | "replaceExisting">;
interface MapSelection {
  language: string;
  version: MapVersion | undefined;
  key: string;
}
interface LanguageOption {
  lang: string;
  label: string;
}
interface MapDecision {
  request: MapSelectionRequest;
  preparation: MapPreparation;
  title: string;
}
type MapCleanup = (
  | { kind: "job"; job: MapJob; books: Book[] }
  | { kind: "book"; book: Book }
) & { privacy: PrivacyContext };
const resumableMapStatuses = ["paused", "failed", "error", "cancelled", "canceled"];
interface Props extends SurfaceProps {
  privacyGeneration: number;
  choices: Record<string, string>;
  onChoice: (region: string, language: string) => void;
  onDownload: (target: DownloadTarget, admission: DownloadAdmission) => void;
  onDismissTarget: (target: DownloadTarget) => void;
}
export default function Maps({
  books,
  settings,
  privacyGeneration,
  onOpen,
  onRefresh,
  onError,
  choices,
  onChoice,
  onDownload,
  onDismissTarget,
}: Props) {
  const { locale } = useLocale();
  const visible = useDocumentVisible();
  const [catalog, setCatalog] = useState<MapCatalog | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [statusError, setStatusError] = useState("");
  const [query, setQuery] = useState("");
  const [group, setGroup] = useState("all");
  const [openedRegion, setDetail] = useState<MapRegion | null>(null);
  const openedRef = useRef(openedRegion);
  openedRef.current = openedRegion;
  const detail = openedRegion && catalog?.regions.find((region) => region.id === openedRegion.id);
  const [recipeMetadata, setRecipeMetadata] = useState<{ key: string; value?: MapRecipeMetadata; error?: string; stale?: boolean } | null>(null);
  const [metadataRetry, setMetadataRetry] = useState(0);
  const [job, setJob] = useState<MapJob | null>(null);
  const [pollEpoch, setPollEpoch] = useState(0);
  const [working, setWorking] = useState(false);
  const [startError, setStartError] = useState("");
  const [decision, setDecision] = useState<MapDecision | null>(null);
  const decisionRef = useRef<MapDecision | null>(null);
  const selectionRequest = useRef<MapSelectionRequest | null>(null);
  const startPending = useRef(false);
  const confirmationPending = useRef(false);
  const mapActionPending = useRef(false);
  const [cleanup, setCleanup] = useState<MapCleanup | null>(null);
  const cleanupRef = useRef<MapCleanup | null>(null);
  cleanupRef.current = cleanup;
  const [cleanupError, setCleanupError] = useState("");
  const cleanupRemovedInstallations = useRef(new Set<string>());
  const alive = useRef(true);
  const jobRevision = useRef(0);
  const statusPayload = useRef("");
  const previous = useRef("");
  const catalogRequest = useRef(0);
  const mapRequest = useRef<(MapStartRequest & { jobId: string }) | null>(null);
  const downloadConfiguration = mapDownloadConfiguration(settings, books);
  const configurationRef = useRef(downloadConfiguration);
  configurationRef.current = downloadConfiguration;
  const jobRef = useRef(job);
  jobRef.current = job;
  const booksRef = useRef(books);
  booksRef.current = books;
  const blockedRef = useRef(settings.privateMode || settings.readOnly);
  blockedRef.current = settings.privateMode || settings.readOnly;
  const load = useCallback(async () => {
    const id = ++catalogRequest.current;
    setLoading(true);
    setError("");
    dismissDecision();
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.generation !== privacyGeneration) {
      setError(t("Browsing session expired after a privacy transition"));
      setLoading(false);
      return;
    }
    try {
      const result = await api<MapCatalog>("map_catalog", {
        url: settings.mapCatalogUrl || undefined,
        privacyGeneration: privacy.generation,
      });
      if (!alive.current || id !== catalogRequest.current) return;
      if (!isPrivacyContextCurrent(privacy)) {
        setError(t("Browsing session expired after a privacy transition"));
        return;
      }
      setCatalog(result);
      setMetadataRetry((n) => n + 1);
    } catch (e) {
      if (alive.current && id === catalogRequest.current) {
        setError(isPrivacyContextCurrent(privacy)
          ? errorText(e) : t("Browsing session expired after a privacy transition"));
      }
    } finally {
      if (alive.current && id === catalogRequest.current) setLoading(false);
    }
  }, [settings.mapCatalogUrl, privacyGeneration]);
  useEffect(() => {
    setDetail(null);
    setCatalog(null);
    setRecipeMetadata(null);
  }, [settings.mapCatalogUrl, privacyGeneration]);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    void load();
    return () => { catalogRequest.current++; };
  }, [load]);
  useEffect(() => {
    if (!visible) return;
    let stopped = false;
    let timer: number | undefined;
    async function tick() {
      if (stopped || document.visibilityState === "hidden") return;
      const privacy = capturePrivacyContext();
      const revision = jobRevision.current;
      let next = jobRef.current;
      try {
        if (!privacy || privacy.generation !== privacyGeneration) return;
        const result = await api<MapJob | null>("map_status");
        if (stopped || !isPrivacyContextCurrent(privacy) || revision !== jobRevision.current) return;
        next = result;
        const payload = JSON.stringify(next);
        if (payload !== statusPayload.current) {
          statusPayload.current = payload;
          setJob(next);
        }
        setStatusError("");
        if (
          next &&
          ["completed", "complete"].includes(next.status) &&
          previous.current !== next.id + next.status + (next.importState || "")
        )
          onRefresh();
        previous.current = next
          ? next.id + next.status + (next.importState || "")
          : "";
      } catch (e) {
        if (!stopped && privacy && isPrivacyContextCurrent(privacy) && revision === jobRevision.current)
          setStatusError(errorText(e));
      } finally {
        // Retain a slower idle probe to discover a job started by another window.
        const progresses = next && ((!terminalStatus(next.status) && next.status !== "paused") ||
          ["queued", "verifying", "ready"].includes(next.importState || ""));
        if (!stopped && !document.hidden)
          timer = window.setTimeout(tick, progresses ? 1200 : 10000);
      }
    }
    void tick();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [visible, onRefresh, privacyGeneration, pollEpoch]);
  function noteJobUpdate() {
    jobRevision.current++;
    statusPayload.current = "";
    setPollEpoch((value) => value + 1);
  }
  const installed = books.filter(
    (book) =>
      /^osm_/i.test(book.name) ||
      book.tags.some((tag) => /^(map|maps|_category:maps)$/i.test(tag)),
  );
  const readyMaps = job ? readyDownloadBooks(job, books) : [];
  const checkingImport = !!job && ["queued", "verifying", "ready"].includes(job.importState || "");
  const regions = useMemo(
    () =>
      catalog?.regions
        .filter(
          (region) =>
            // A region the catalog adds that this build has no group for must
            // stay visible under every filter rather than silently vanish.
            (group === "all" || groups[region.id] === undefined || groups[region.id] === group) &&
            (!query ||
              `${region.title} ${region.id}`
                .toLocaleLowerCase()
                .includes(query.toLocaleLowerCase())),
        )
        .sort((a, b) => a.title.localeCompare(b.title)) || [],
    [catalog, query, group],
  );
  const resolvedLocale = locale === "system" ? getLocale() : locale;
  /** The selected language and version of a region, with the identity that
   * request revalidation compares. Recomputed once per catalog/locale/choice. */
  function selectRegion(region: MapRegion): MapSelection {
    const language = region.menu.some((v) => v.lang === choices[region.id])
      ? choices[region.id]
      : preferredVersion(region.menu, resolvedLocale);
    const version = region.menu.find((entry) => entry.lang === language);
    return {
      language,
      version,
      key: JSON.stringify([language, version?.name, version?.md5, version?.recipe, version?.size]),
    };
  }
  const selections = useMemo(() => {
    const index = new globalThis.Map<string, MapSelection>();
    for (const region of catalog?.regions || []) index.set(region.id, selectRegion(region));
    return index;
  }, [catalog, resolvedLocale, choices]);
  function selectionFor(region: MapRegion) {
    return selections.get(region.id) ?? selectRegion(region);
  }
  function requestCurrent(request: MapSelectionRequest) {
    return alive.current && !blockedRef.current &&
      request.configuration === configurationRef.current &&
      request.selection === selections.get(request.regionId)?.key &&
      isPrivacyContextCurrent(request.privacy) &&
      !request.privacy.privateMode && !request.privacy.readOnly;
  }
  /** The open area dialog shows its failure beside Get map. A card or the job
   * panel can be scrolled anywhere in the page, so their failures go to the
   * shell's always-visible surface instead of the top of the scrolled page. */
  function reportStartError(e: unknown) {
    if (!openedRef.current) {
      onError(e);
      return;
    }
    if (reportError(diagnostics, e, "blocking", true)) setStartError(errorText(e));
  }
  function dismissDecision() {
    selectionRequest.current = null;
    decisionRef.current = null;
    setDecision(null);
    if (!startPending.current && !confirmationPending.current && !mapActionPending.current && alive.current) setWorking(false);
  }
  function chooseLanguage(region: string, language: string) {
    dismissDecision();
    if (mapRequest.current?.regionId === region) mapRequest.current = null;
    onChoice(region, language);
  }
  function openDetail(region: MapRegion) {
    dismissDecision();
    setStartError("");
    setDetail(region);
  }
  function closeDetail() {
    dismissDecision();
    setDetail(null);
  }
  useEffect(() => {
    // A snapshot may retire privacy authority without changing the visible settings.
    if (selectionRequest.current && !requestCurrent(selectionRequest.current)) dismissDecision();
    if (mapRequest.current && !requestCurrent(mapRequest.current)) mapRequest.current = null;
    if (cleanup && !isPrivacyContextCurrent(cleanup.privacy) && !mapActionPending.current) setCleanup(null);
  });
  // Labels and grouping depend only on the region menu and the resolved locale;
  // caching them per region keeps Intl and the sort off the per-card path.
  const languageOptionsByRegion = useMemo(
    () => new globalThis.Map<string, { local: LanguageOption[]; other: LanguageOption[] }>(),
    [resolvedLocale, catalog],
  );
  function languageOptions(region: MapRegion) {
    let options = languageOptionsByRegion.get(region.id);
    if (!options) {
      const promoted = new Set(
        (localLanguages[region.id] || []).map(baseLanguage),
      );
      const decorated = region.menu.map((version) => ({ version, label: versionLabel(version) }));
      decorated.sort(
        (a, b) =>
          a.label.localeCompare(b.label, getLocale(), {
            sensitivity: "base",
          }) || a.version.lang.localeCompare(b.version.lang),
      );
      const local: LanguageOption[] = [];
      const other: LanguageOption[] = [];
      for (const entry of decorated) {
        const option = { lang: entry.version.lang, label: entry.label };
        (promoted.has(baseLanguage(entry.version.lang)) ? local : other).push(option);
      }
      options = { local, other };
      languageOptionsByRegion.set(region.id, options);
    }
    const localOptions = options.local.map((option) => (
      <option value={option.lang} key={option.lang}>
        {option.label}
      </option>
    ));
    const otherOptions = options.other.map((option) => (
      <option value={option.lang} key={option.lang}>
        {option.label}
      </option>
    ));
    return options.local.length ? (
      <>
        <optgroup label={t("Local languages")}>{localOptions}</optgroup>
        {!!options.other.length && (
          <optgroup label={t("Other languages")}>{otherOptions}</optgroup>
        )}
      </>
    ) : (
      otherOptions
    );
  }
  async function submitMapRequest(
    request: MapStartRequest,
    diskSpaceAcknowledgement?: string,
    resumeJobId?: string,
  ) {
    if (startPending.current || !requestCurrent(request))
      return;
    startPending.current = true;
    setStartError("");
    setWorking(true);
    try {
      const next = await api<MapJob>(resumeJobId ? "map_resume" : "map_start", {
        ...(resumeJobId ? { jobId: resumeJobId } : {
          regionId: request.regionId,
          language: request.language,
          decisionToken: request.decisionToken,
          replaceExisting: request.replaceExisting,
        }),
        privacyGeneration: request.privacy.generation,
        diskSpaceAcknowledgement,
      });
      const present = alive.current && isPrivacyContextCurrent(request.privacy);
      const retained = requestCurrent(request) ? request : undefined;
      onDownload({ kind: "map", id: next.id, request: retained }, { privacy: request.privacy, focus: present });
      if (present) {
        mapRequest.current = retained ? { ...retained, jobId: next.id } : null;
        noteJobUpdate();
        setJob(next);
        setDetail(null);
      }
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(request.privacy)) reportStartError(e);
    } finally {
      startPending.current = false;
      dismissDecision();
      if (alive.current) setWorking(false);
    }
  }
  async function start(region: MapRegion) {
    if (loading || error || confirmationPending.current || startPending.current || selectionRequest.current) return;
    const selection = selectionFor(region);
    const { language, version } = selection;
    if (!version) return;
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.privateMode || privacy.readOnly) return;
    const request: MapSelectionRequest = {
      regionId: region.id,
      language,
      configuration: downloadConfiguration,
      selection: selection.key,
      privacy,
    };
    selectionRequest.current = request;
    setStartError("");
    setWorking(true);
    try {
      const preparation = await api<MapPreparation>("map_prepare", {
        regionId: request.regionId,
        language: request.language,
        privacyGeneration: request.privacy.generation,
      });
      if (selectionRequest.current !== request || !requestCurrent(request)) return;
      if (preparation.regionId !== request.regionId || preparation.language !== request.language ||
        preparation.name !== version.name || preparation.md5 !== version.md5) {
        setError(t("Map catalog changed. Reload the map registry before downloading."));
        return;
      }
      if (preparation.installed) {
        const next = { request, preparation, title: region.title };
        decisionRef.current = next;
        setDecision(next);
      } else {
        await submitMapRequest({ ...request, decisionToken: preparation.token, replaceExisting: false });
      }
    } catch (e) {
      if (selectionRequest.current === request) {
        if (requestCurrent(request)) reportStartError(e);
        dismissDecision();
      }
    } finally {
      if (selectionRequest.current === request && !decisionRef.current) selectionRequest.current = null;
      if (alive.current && !startPending.current &&
        (!selectionRequest.current || selectionRequest.current === request)) setWorking(false);
    }
  }
  async function replaceMap(retained: MapDecision) {
    if (decisionRef.current !== retained || selectionRequest.current !== retained.request ||
      !requestCurrent(retained.request)) return;
    decisionRef.current = null;
    setDecision(null);
    await submitMapRequest({
      ...retained.request,
      decisionToken: retained.preparation.token,
      replaceExisting: true,
    });
  }
  async function confirmAndRetry() {
    const currentJob = jobRef.current;
    const warning = currentJob && parseDiskSpaceWarning(currentJob.error);
    const request = mapRequest.current;
    if (
      confirmationPending.current || startPending.current || !alive.current ||
      blockedRef.current || !warning || !request ||
      request.jobId !== currentJob?.id ||
      !isPrivacyContextCurrent(request.privacy) ||
      request.privacy.privateMode || request.privacy.readOnly
    )
      return;
    if (request.configuration !== configurationRef.current) {
      mapRequest.current = null;
      setError(t("Download settings changed. Start this map download again."));
      return;
    }
    confirmationPending.current = true;
    setWorking(true);
    try {
      if (!await confirmDiskSpaceWarning(warning)) return;
      const latest = jobRef.current;
      if (!alive.current || !latest || blockedRef.current || mapRequest.current !== request ||
        latest.id !== request.jobId || !isPrivacyContextCurrent(request.privacy) ||
        parseDiskSpaceWarning(latest.error)?.acknowledgement !== warning.acknowledgement) return;
      if (request.configuration !== configurationRef.current) {
        mapRequest.current = null;
        setError(t("Download settings changed. Start this map download again."));
        return;
      }
      await submitMapRequest(request, warning.acknowledgement, latest.resumable ? latest.id : undefined);
    } finally {
      confirmationPending.current = false;
      if (alive.current) setWorking(false);
    }
  }
  async function changeMapState(command: "map_pause" | "map_resume") {
    const privacy = capturePrivacyContext();
    const latest = jobRef.current;
    if (!alive.current || !latest || mapActionPending.current || blockedRef.current ||
      !privacy || !isPrivacyContextCurrent(privacy)) return;
    if (command === "map_resume"
      ? !resumableMapStatuses.includes(latest.status) || latest.resumable !== true
      : terminalStatus(latest.status) || latest.status === "paused") return;
    mapActionPending.current = true;
    setWorking(true);
    setStartError("");
    try {
      const next = await api<MapJob>(command, {
        jobId: latest.id,
        privacyGeneration: privacy.generation,
      });
      const present = alive.current && isPrivacyContextCurrent(privacy) && jobRef.current?.id === latest.id;
      if (command === "map_resume") {
        const retained = mapRequest.current?.jobId === latest.id && requestCurrent(mapRequest.current)
          ? mapRequest.current : undefined;
        onDownload({ kind: "map", id: next.id, request: retained }, { privacy, focus: present });
      }
      if (present) {
        noteJobUpdate();
        setJob(next);
      }
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(privacy)) reportStartError(e);
    } finally {
      mapActionPending.current = false;
      if (alive.current) setWorking(false);
    }
  }
  function requestJobCleanup() {
    const privacy = capturePrivacyContext();
    const latest = jobRef.current;
    if (!alive.current || !latest || mapActionPending.current || blockedRef.current ||
      !privacy || !isPrivacyContextCurrent(privacy) ||
      (!terminalStatus(latest.status) && latest.status !== "paused") ||
      ["queued", "verifying", "ready"].includes(latest.importState || "")) return;
    setCleanupError("");
    cleanupRemovedInstallations.current.clear();
    setCleanup({ kind: "job", job: latest, books: cleanupDownloadBooks(latest, booksRef.current).map((book) => ({ ...book })), privacy });
  }
  function requestBookCleanup(book: Book) {
    const privacy = capturePrivacyContext();
    if (!alive.current || mapActionPending.current || blockedRef.current ||
      !privacy || !isPrivacyContextCurrent(privacy)) return;
    setCleanupError("");
    cleanupRemovedInstallations.current.clear();
    setCleanup({ kind: "book", book, privacy });
  }
  async function confirmCleanup(deleteFiles: boolean) {
    const retained = cleanup;
    if (!alive.current || !retained || mapActionPending.current || blockedRef.current ||
      !isPrivacyContextCurrent(retained.privacy)) return;
    if (!deleteFiles && retained.kind === "job" && retained.job.canRemoveFromList !== true) {
      setCleanupError(t("Resume this download or delete its files before removing it from the list."));
      return;
    }
    const retainedBooks = retained.kind === "job" ? retained.books : [retained.book];
    const current = () => {
      if (!alive.current || cleanupRef.current !== retained || blockedRef.current ||
        !isPrivacyContextCurrent(retained.privacy)) return false;
      let currentBooks: Book[];
      if (retained.kind === "job") {
        const latest = jobRef.current;
        if (!latest || latest.id !== retained.job.id || latest.path !== retained.job.path ||
          latest.status !== retained.job.status ||
          (!terminalStatus(latest.status) && latest.status !== "paused") ||
          ["queued", "verifying", "ready"].includes(latest.importState || "")) return false;
        if (!deleteFiles) return latest.canRemoveFromList === true;
        currentBooks = cleanupDownloadBooks(latest, booksRef.current);
      } else {
        currentBooks = booksRef.current.filter((book) => book.id === retained.book.id);
      }
      const expected = retainedBooks.filter((book) => !cleanupRemovedInstallations.current.has(book.installationId));
      const remaining = currentBooks.filter((book) => !cleanupRemovedInstallations.current.has(book.installationId));
      return remaining.length === expected.length &&
        remaining.every((book) => expected.some((captured) =>
          captured.id === book.id && captured.installationId === book.installationId &&
          (retained.kind !== "job" || captured.path === book.path))) &&
        !retainedBooks.some((book) => cleanupRemovedInstallations.current.has(book.installationId) &&
          booksRef.current.some((candidate) => candidate.id === book.id && candidate.installationId !== book.installationId));
    };
    if (!current()) {
      setCleanupError(t("The download or saved map changed. Close this dialog and try again."));
      return;
    }
    mapActionPending.current = true;
    setWorking(true);
    setCleanupError("");
    try {
      if (deleteFiles && retained.kind === "job") {
        await api("prepare_download_cleanup", {
          kind: "map",
          id: retained.job.id,
          privacyGeneration: retained.privacy.generation,
          installations: retainedBooks.filter((book) => !cleanupRemovedInstallations.current.has(book.installationId)).map((book) => ({
            id: book.id,
            installationId: book.installationId,
            path: book.path,
          })),
        });
        if (!current()) {
          throw new Error(t("The download or saved map changed. Close this dialog and try again."));
        }
      }
      if (deleteFiles || retained.kind === "book") {
        for (const book of retainedBooks) {
          if (cleanupRemovedInstallations.current.has(book.installationId)) continue;
          if (!current()) throw new Error(t("The download or saved map changed. Close this dialog and try again."));
          await api("remove_book", {
            id: book.id,
            installationId: book.installationId,
            deleteFile: deleteFiles,
            privacyGeneration: retained.privacy.generation,
            ...(retained.kind === "job"
              ? { cleanupSource: { kind: "map", id: retained.job.id } } : {}),
          });
          cleanupRemovedInstallations.current.add(book.installationId);
        }
      }
      if (retained.kind === "job") {
        if (!current()) throw new Error(t("The download or saved map changed. Close this dialog and try again."));
        await api(deleteFiles ? "map_delete_download" : "map_remove_history", {
          ...(deleteFiles ? { jobId: retained.job.id } : { id: retained.job.id }),
          privacyGeneration: retained.privacy.generation,
        });
        onDismissTarget({ kind: "map", id: retained.job.id });
        if (mapRequest.current?.jobId === retained.job.id) mapRequest.current = null;
        if (alive.current) {
          // Delete keeps files it cannot prove are its own; the download then
          // stays listed with them, so read what remains instead of assuming
          // it is gone. A failed read is reported by the restarted poll.
          noteJobUpdate();
          const revision = jobRevision.current;
          const next = await api<MapJob | null>("map_status").catch(() => undefined);
          if (next !== undefined && alive.current && revision === jobRevision.current &&
            isPrivacyContextCurrent(retained.privacy)) {
            statusPayload.current = JSON.stringify(next);
            setJob(next);
          }
        }
      }
      if (alive.current) setCleanup(null);
      onRefresh();
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(retained.privacy)) {
        // The dialog shows the failure; it is recorded, not painted twice.
        setCleanupError(errorText(e));
        onError(e, "blocking", false);
        // A Delete that had to keep a file leaves the download listed with it.
        noteJobUpdate();
        onRefresh();
      }
    } finally {
      mapActionPending.current = false;
      if (alive.current) setWorking(false);
    }
  }
  async function forgetRecovery(path: string) {
    const privacy = capturePrivacyContext();
    const latest = jobRef.current;
    if (!alive.current || !latest || mapActionPending.current || blockedRef.current ||
      !privacy || !isPrivacyContextCurrent(privacy) || !latest.recoveryPaths?.includes(path)) return;
    mapActionPending.current = true;
    setWorking(true);
    try {
      // Only the warning is dismissed; the file stays where it is.
      await api("map_forget_recovery", { path, privacyGeneration: privacy.generation });
      if (alive.current && isPrivacyContextCurrent(privacy)) noteJobUpdate();
    } catch (e) {
      if (alive.current && isPrivacyContextCurrent(privacy)) {
        onError(e);
        noteJobUpdate();
      }
    } finally {
      mapActionPending.current = false;
      if (alive.current) setWorking(false);
    }
  }
  const active = job && !terminalStatus(job.status);
  const diskSpaceWarning =
    job?.status === "failed" ? parseDiskSpaceWarning(job.error) : null;
  const retryRequest =
    mapRequest.current?.jobId === job?.id ? mapRequest.current : null;
  const detailSelection = detail ? selectionFor(detail) : undefined;
  const detailVersion = detailSelection?.version;
  const detailLanguage = detailSelection?.language ?? "";
  const metadataKey = detail && detailVersion
    ? JSON.stringify([settings.mapCatalogUrl, detail.id, detailLanguage, detailVersion.recipe, detailVersion.name, detailVersion.md5])
    : "";
  const currentMetadata = recipeMetadata?.key === metadataKey ? recipeMetadata : null;
  useEffect(() => {
    if (loading || !metadataKey || !detail || !detailVersion) return;
    let stopped = false;
    const privacy = capturePrivacyContext();
    if (!privacy || privacy.generation !== privacyGeneration) return;
    setRecipeMetadata({ key: metadataKey });
    void api<MapRecipeMetadata>("map_recipe_metadata", {
      url: settings.mapCatalogUrl || undefined,
      regionId: detail.id,
      language: detailLanguage,
      privacyGeneration: privacy.generation,
    }).then((value) => {
      if (stopped || !isPrivacyContextCurrent(privacy)) return;
      if (
        value.regionId !== detail.id || value.language !== detailLanguage ||
        value.name !== detailVersion.name || value.md5 !== detailVersion.md5 ||
        (detailVersion.size != null && value.assembledBytes !== detailVersion.size)
      ) {
        setRecipeMetadata({ key: metadataKey, stale: true, error: t("Map catalog changed. Reload the map registry before downloading.") });
        return;
      }
      setRecipeMetadata({ key: metadataKey, value });
    }).catch((value) => {
      if (!stopped && isPrivacyContextCurrent(privacy))
        setRecipeMetadata({ key: metadataKey, error: errorText(value) });
    });
    return () => { stopped = true; };
  }, [metadataKey, detail, detailVersion, detailLanguage, settings.mapCatalogUrl, settings.privateMode, settings.readOnly, privacyGeneration, loading, metadataRetry]);
  return (
    <section className="page maps-page">
      <PageHeader title={t("Maps")} />
      {statusError && <ErrorNotice message={statusError} onDismiss={() => setStatusError("")} />}
      {!!installed.length && (
        <section className="installed-maps">
          <div className="section-heading">
            <h2>{t("Saved maps")}</h2>
            <span className="muted">{formatNumber(installed.length)}</span>
          </div>
          <div className="installed-map-grid">
            {installed.map((book) => {
              const label = savedMapLabel(book, catalog?.regions || []);
              return (
                <SwipeRow key={`${book.id}:${book.installationId}`}
                  actionLabel={t("Remove from library")}
                  disabled={working || !!cleanup || settings.privateMode || settings.readOnly}
                  onAction={() => requestBookCleanup(book)}>
                <article className="panel installed-map">
                  <Map size={25} />
                  <div>
                    <h3>{label.title}</h3>
                    <p>
                      {label.subtitle} <span aria-hidden="true">·</span>{" "}
                      {formatBytes(book.size)}
                    </p>
                    {!book.available && (
                      <span className="badge warning">{t("File missing")}</span>
                    )}
                  </div>
                  <button
                    disabled={!book.available}
                    onClick={() =>
                      onOpen({
                        bookId: book.id,
                        path: "",
                        title: book.title,
                        scroll: 0,
                      })
                    }
                  >
                    <BookOpen size={15} />
                    {t("Open map")}
                  </button>
                  <button className="icon-button" aria-label={t("Remove from library")} title={t("Remove from library")}
                    disabled={working || !!cleanup || settings.privateMode || settings.readOnly}
                    onClick={() => requestBookCleanup(book)}>
                    <Trash2 size={16} aria-hidden="true" />
                  </button>
                </article>
                </SwipeRow>
              );
            })}
          </div>
        </section>
      )}
      {job && (
        <SwipeRow className="swipe-map-job" actionLabel={t("Delete download")}
          disabled={working || !!cleanup || settings.privateMode || settings.readOnly ||
            (!terminalStatus(job.status) && job.status !== "paused") || checkingImport}
          onAction={requestJobCleanup}>
        <section className="panel map-job">
          <div className="section-heading">
            <h2>
              {t("Map: {region}", {
                region:
                  catalog?.regions.find((region) => region.id === job.regionId)
                    ?.title || job.regionId,
              })}
            </h2>
            <span className="badge" role="status">{jobStatus(job.status)}</span>
            {active && job.status !== "paused" && (
              <button disabled={working || settings.privateMode || settings.readOnly}
                onClick={(event) => { if (event.nativeEvent.isTrusted) void changeMapState("map_pause"); }}>
                <Pause size={16} />{t("Pause")}
              </button>
            )}
          </div>
          <p className="muted">{languageName(job.language)}</p>
          {readyMaps.map((book) => (
            <button className="primary download-open" key={book.installationId}
              onClick={() => onOpen({ bookId: book.id, path: "", title: book.title, scroll: 0 })}>
              <BookOpen size={28} />{t("Open {title}", { title: book.title })}
            </button>
          ))}
          {!readyMaps.length && !job.error && !job.importError &&
            (!terminalStatus(job.status) || checkingImport) && (
          <label className="progress-label">
            {t("Local assembly")}
            <progress
              max={checkingImport ? undefined : job.total || undefined}
              value={checkingImport ? undefined : job.total ? job.written : undefined}
            />
            {!checkingImport && job.total > 0 && <span>
              {t("{percent}%", { percent: formatNumber(Math.floor(Math.min(100, Math.max(0, job.written / job.total * 100)))) })}
            </span>}
          </label>
          )}
          <MapBlockProgress job={job} />
          <MapTransferRate job={job} />
          <details className="download-details">
            <summary>{t("Details for {title}", { title: t("Map: {region}", { region: job.regionId }) })}</summary>
            <p className="muted">
              {t("Current block")}: {job.incomingTotal
                ? t("{downloaded} of {total}", {
                    downloaded: formatBytes(job.incoming),
                    total: formatBytes(job.incomingTotal),
                  })
                : formatBytes(job.incoming)}
            </p>
            <p className="muted">
              {t("Local assembly")}: {job.total
                ? t("{written} of {total}", {
                    written: formatBytes(job.written),
                    total: formatBytes(job.total),
                  })
                : formatBytes(job.written)}
            </p>
            {!isIOS && job.path && <p className="file-path muted">{displayPath(job.path)}</p>}
          </details>
          {diskSpaceWarning ? (
            <div role="alert">
              <p className="inline-error">
                {t(
                  "This map download would leave less than 1 GB of free space in {directory}. Projected free space: {remaining}.",
                  {
                    directory: isIOS ? t("Kiwix storage") : diskSpaceWarning.directory,
                    remaining: formatBytes(diskSpaceWarning.projectedFreeBytes),
                  },
                )}
              </p>
              {diskSpaceWarning.recoveryWarning && (
                <p className="inline-error">{errorText(diskSpaceWarning.recoveryWarning)}</p>
              )}
              {retryRequest ? (
                retryRequest.configuration === downloadConfiguration ? (
                  <button
                    onClick={confirmAndRetry}
                    disabled={working || settings.privateMode || settings.readOnly}
                  >
                    {t("Review disk space and continue")}
                  </button>
                ) : (
                  <p className="muted">
                    {t("Download settings changed. Start this map download again.")}
                  </p>
                )
              ) : (
                <p className="muted">
                  {t("Start this map download again to review its disk space requirement.")}
                </p>
              )}
            </div>
          ) : job.error ? (
            <p className="inline-error" role="alert">
              {errorText(job.error)}
            </p>
          ) : null}
          <MapRecoveryPaths job={job}
            disabled={working || !!cleanup || settings.privateMode || settings.readOnly}
            onDismiss={(path) => void forgetRecovery(path)} />
          {job.importError && (
            <p className="inline-error" role="alert">
              {t("Map import failed: {error}", { error: errorText(job.importError) })}
            </p>
          )}
          {["queued", "verifying", "ready"].includes(job.importState || "") && (
            <p role="status" className="muted">
              {t("Verifying")}
            </p>
          )}
          {resumableMapStatuses.includes(job.status) && (
            <>
              <div className="modal-actions">
                <button className="primary"
                  disabled={working || settings.privateMode || settings.readOnly || job.resumable !== true}
                  onClick={(event) => { if (event.nativeEvent.isTrusted) void changeMapState("map_resume"); }}>
                  <Play size={16} />{t("Resume")}
                </button>
              </div>
              {job.resumable !== true && (
                <p className="muted">{t("This download cannot be resumed. Delete this download, then get the map again.")}</p>
              )}
            </>
          )}
          {(terminalStatus(job.status) || job.status === "paused") && !checkingImport && (
            <button disabled={working || settings.privateMode || settings.readOnly} onClick={requestJobCleanup}>
              <Trash2 size={16} />{t("Delete download")}
            </button>
          )}
          {job.imports
            ?.filter(
              (entry) =>
                entry.state === "failed" || entry.state === "cancelled",
            )
            .map((entry) => (
              <button
                key={entry.receipt}
                disabled={working || settings.readOnly || settings.privateMode}
                onClick={async () => {
                  const privacy = capturePrivacyContext();
                  if (!privacy || privacy.privateMode || privacy.readOnly) return;
                  setWorking(true);
                  try {
                    await api("retry_import", { receipt: entry.receipt });
                    const next = await api<MapJob | null>("map_status");
                    if (alive.current && isPrivacyContextCurrent(privacy)) {
                      noteJobUpdate();
                      setJob(next);
                    }
                    onRefresh();
                  } catch (value) {
                    onError(value);
                  } finally {
                    setWorking(false);
                  }
                }}
              >
                {t("Retry archive import")}
              </button>
            ))}
        </section>
        </SwipeRow>
      )}
      <div className="section-heading">
        <h2>{t("Browse map areas")}</h2>
      </div>
      <div className="toolbar compact-toolbar map-area-toolbar">
        <label className="search-field">
          <Search size={17} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("Find a country, region or city…")}
            aria-label={t("Search map areas")}
          />
        </label>
        <select
          value={group}
          onChange={(e) => setGroup(e.target.value)}
          aria-label={t("Geographic group")}
        >
          <option value="all">{t("All areas")}</option>
          <option value="region">{t("Regions")}</option>
          <option value="country">{t("Countries")}</option>
          <option value="local">{t("Local areas")}</option>
        </select>
      </div>
      {!openedRegion && error && (
        <ErrorNotice
          message={t("Could not load map data.") + "\n" + error}
          onRetry={load}
        />
      )}
      {loading && !catalog ? (
        <Loading label={t("Loading map registry…")} />
      ) : !catalog ? (
        <Empty
          title={t("No map areas available")}
          description={t(
            "The map registry did not return any areas. Check the connection, then retry.",
          )}
        >
          <button onClick={() => void load()}>{t("Retry")}</button>
        </Empty>
      ) : !regions.length ? (
        <Empty
          title={t("No matching map areas")}
          description={t("Try another place name or geographic group.")}
        />
      ) : (
        <PagedResults items={regions} criteria={JSON.stringify([query, group, resolvedLocale])}
          label={t("Maps")} className="map-grid">
          {(region) => {
            const { version } = selectionFor(region);
            const language = version ? versionLabel(version) : "";
            return (
              <article className="map-card panel" key={region.id}>
                <div className="map-card-heading">
                  <Map size={23} />
                  <div>
                    <h3>
                      <button
                        className="text-button"
                        onClick={() => openDetail(region)}
                      >
                        {region.title}
                      </button>
                    </h3>
                    <span className="muted">
                      {groups[region.id] === "country"
                        ? t("Country")
                        : groups[region.id] === "local"
                          ? t("Local area")
                          : groups[region.id] === "region"
                            ? t("Region")
                            : t("Map area")}
                    </span>
                  </div>
                </div>
                <p className="map-size muted">
                  {version ? (
                    <>{t("On device")}: <strong>{formatBytes(version.size ?? Number.NaN)}</strong></>
                  ) : t("No language available")}
                </p>
                <div className="compact-toolbar map-card-controls">
                  {version && (
                    <button type="button" className="map-language-button" aria-haspopup="dialog"
                      aria-label={`${t("Map language")}: ${language}`}
                      onClick={() => openDetail(region)}>
                      {language}
                    </button>
                  )}
                  <button
                    disabled={
                      loading || !!error || working || !!active || !version || settings.privateMode || settings.readOnly
                    }
                    onClick={() => void start(region)}
                  >
                    <ArrowDownToLine size={16} />
                    {t("Get map")}
                  </button>
                </div>
              </article>
            );
          }}
        </PagedResults>
      )}
      <details className="secondary-controls map-options">
        <summary>{t("Map options")}</summary>
        <p className="muted">{t("Exact StreetZim area registry")}</p>
        <p className="muted">
          {t("Choose an area and map language. Shared base data is cached, verified and assembled locally into a readable archive.")}
        </p>
        <button
          onClick={async () => {
            if (!(await confirmAction(t("Clear the map assembly cache? Installed maps will not be removed."), {
              confirmLabel: t("Clear map cache"),
              danger: true,
            }))) return;
            setWorking(true);
            try {
              await api("map_clear_cache");
            } catch (e) {
              onError(e);
            } finally {
              setWorking(false);
            }
          }}
          disabled={working || !!active}
        >
          <Trash2 size={16} />
          {t("Clear map cache")}
        </button>
      </details>
      {cleanup && isPrivacyContextCurrent(cleanup.privacy) && !settings.privateMode && !settings.readOnly && (
        <DownloadCleanupDialog
          name={cleanup.kind === "book" ? cleanup.book.title :
            catalog?.regions.find((region) => region.id === cleanup.job.regionId)?.title || cleanup.job.regionId}
          title={cleanup.kind === "book"
            ? t("Remove saved map “{title}”?", { title: cleanup.book.title }) : undefined}
          busy={working || mapActionPending.current}
          error={cleanupError}
          archiveNames={(cleanup.kind === "book" ? [cleanup.book] : cleanup.books).map((book) => book.title)}
          removeFromListDisabled={cleanup.kind === "job" && cleanup.job.canRemoveFromList !== true}
          description={cleanup.kind === "book"
            ? t("Keep the map file when removing it from your library, or delete the map and its file to free storage.")
            : cleanup.job.canRemoveFromList !== true
              ? t("Resume this download or delete its files before removing it from the list.")
              : undefined}
          onCancel={() => setCleanup(null)}
          onRemoveFromList={() => void confirmCleanup(false)}
          onDeleteFiles={() => void confirmCleanup(true)}
        />
      )}
      {openedRegion && !decision && (
        <Modal title={detail?.title ?? openedRegion.title} onClose={closeDetail}>
          {startError && !loading && <ErrorNotice message={startError} onDismiss={() => setStartError("")} />}
          {loading ? <Loading label={t("Loading map registry…")} /> : error ? (
            <ErrorNotice message={t("Could not load map data.") + "\n" + error} onRetry={load} />
          ) : !detail ? (
            <ErrorNotice message={t("This area is no longer in the map registry.")} onRetry={load} />
          ) : <>
          <label className="field">
            {t("Map language")}
            <select
              value={detailSelection?.language ?? ""}
              onChange={(e) => chooseLanguage(detail.id, e.target.value)}
            >
              {languageOptions(detail)}
            </select>
          </label>
          <dl className="facts">
            <div>
              <dt>{t("On device")}</dt>
              <dd>{formatBytes(currentMetadata?.value?.assembledBytes ?? detailVersion?.size ?? Number.NaN)}</dd>
            </div>
          </dl>
          <div className="muted">
            {currentMetadata?.error ? (
              <ErrorNotice message={currentMetadata.error}
                onRetry={currentMetadata.stale ? load : () => setMetadataRetry((n) => n + 1)} />
            ) : currentMetadata?.value ? (
              <dl className="facts">
                <div>
                  <dt>{t("Download estimate")}</dt>
                  <dd>{formatBytes(currentMetadata.value.coldDownloadBytes)}</dd>
                </div>
              </dl>
            ) : detailVersion ? <p role="status">{t("Loading map transfer estimate…")}</p> : null}
          </div>
          <div className="modal-actions">
            <button
              className="primary"
              disabled={
                working || !!active || !detailVersion || currentMetadata?.stale || settings.privateMode || settings.readOnly
              }
              onClick={() => start(detail)}
            >
              <ArrowDownToLine size={17} />
              {working ? t("Starting…") : t("Get map")}
            </button>
          </div>
          <details className="download-details">
            <summary>{t("Details for {title}", { title: detail.title })}</summary>
            <dl className="facts">
              <div>
                <dt>{t("Shared base size")}</dt>
                <dd>{formatBytes(detail.base.size ?? Number.NaN)}</dd>
              </div>
              {currentMetadata?.value && <div>
                <dt>{t("With reusable map data")}</dt>
                <dd>{formatBytes(currentMetadata.value.reusableBaseDownloadBytes)}</dd>
              </div>}
            </dl>
            <p className="muted">{t("Saved map data can reduce downloads. Metadata and retries can add data.")}</p>
            <p className="muted">{t("Cached base data is reused when available. Keep enough free space for the map and its assembly cache.")}</p>
          </details>
          </>}
        </Modal>
      )}
      {decision && requestCurrent(decision.request) && decision.preparation.installed && (
        <Modal title={t("Map already saved")} onClose={dismissDecision}>
          <p>
            {decision.preparation.sameVersion
              ? t("This version of {region} in {language} is already saved.", {
                  region: decision.title,
                  language: languageName(decision.request.language),
                })
              : t("A map of {region} in {language} is already saved.", {
                  region: decision.title,
                  language: languageName(decision.request.language),
                })}
          </p>
          <dl className="facts">
            <div>
              <dt>{t("Saved version")}</dt>
              <dd>{decision.preparation.installed.date || decision.preparation.installed.name}</dd>
            </div>
            <div>
              <dt>{t("Selected version")}</dt>
              <dd>{decision.preparation.version}</dd>
            </div>
          </dl>
          {!decision.preparation.installed.available && (
            <p className="muted">{t("File missing")}</p>
          )}
          <p className="muted">
            {t("Replace downloads the selected version. Your saved map stays available until the new map is verified and ready.")}
          </p>
          <div className="modal-actions">
            <button type="button" onClick={dismissDecision}>{t("Cancel")}</button>
            <button type="button" disabled={!decision.preparation.installed.available}
              onClick={() => {
                if (decisionRef.current !== decision || !requestCurrent(decision.request)) return;
                const book = decision.preparation.installed;
                if (!book?.available) return;
                dismissDecision();
                setDetail(null);
                onOpen({ bookId: book.id, path: "", title: book.title, scroll: 0 });
              }}>
              <BookOpen size={17} />{t("Open existing")}
            </button>
            <button type="button" className="primary" disabled={working || !!active}
              onClick={(event) => {
                if (event.nativeEvent.isTrusted) void replaceMap(decision);
              }}>
              <ArrowDownToLine size={17} />{t("Replace")}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
