// SPDX-License-Identifier: GPL-3.0-or-later
import {
  api,
  registerPendingPreferenceDrain,
  registerPendingPreferenceCutoverDrain,
} from "./api";
import { t } from "./i18n";

type Binding = {
  origin: string;
  bookId: string;
  privacyGeneration: number;
  retirementUrl: string;
  retirementProof: string;
};
type RegisteredBinding = Binding & {
  source: WeakRef<WindowProxy>;
  onRetire?: () => void | Promise<void>;
  onRestore?: (url: string) => void;
  currentUrl?: string;
  ready: boolean;
  reserve: number;
  documentId?: string;
  failed?: Error;
  retirementReported?: boolean;
  retired?: boolean;
  restoreBlocked?: boolean;
  quarantined?: boolean;
};
type StorageBatch = {
  id: string;
  documentId: string;
  sequence: number;
  operations: Record<string, unknown>[];
};
type Stream = {
  source: WindowProxy;
  binding: RegisteredBinding;
  documentId: string;
  sequence: number;
  available: number;
  closed: boolean;
  hiding: boolean;
  failed: boolean;
  /** Batches of this stream still waiting in `queue`. */
  queued: number;
  queuedBytes: number;
};
type QueuedBatch = { stream: Stream; batch: StorageBatch; bytes: number };
const bindings = new WeakMap<WindowProxy, Set<RegisteredBinding>>();
// Null-source delivery may finish an already-authenticated document stream,
// never admit a new one: detached descendants share the archive origin but are
// not the shell's registered producer. Weak sources do not pin aborted frames.
const origins = new Map<string, Set<RegisteredBinding>>();
const active = new Map<WindowProxy, RegisteredBinding>();
const activeFrames = new Map<WindowProxy, HTMLIFrameElement>();
const currentDocuments = new WeakMap<WindowProxy, string>();
let retiringDocuments: Promise<() => void> | undefined;
let retirementActive = false;
const reservations = new Set<RegisteredBinding>();
// A forced security detach has no final producer proof. Retain its bounded
// authority until the shell replaces the frames and explicitly discards it;
// never recycle that authority for a later document.
const quarantinedBindings = new Set<RegisteredBinding>();
const streams = new Set<Stream>();
const queue: QueuedBatch[] = [];
// A head cursor makes every dequeue O(1); dead entries are compacted
// amortized instead of shifting the whole array per committed batch.
let queueHead = 0;
function queued(): number {
  return queue.length - queueHead;
}
function dequeue(): void {
  queueHead += 1;
  if (queueHead > 1024 && queueHead * 2 >= queue.length) {
    queue.splice(0, queueHead);
    queueHead = 0;
  }
}
// Includes unspent producer credits, detached frames, and every queued batch.
const SOURCE_CREDIT = 2 * 1024 * 1024;
const ROOT_CREDIT = 16 * 1024 * 1024;
let reservedBytes = 0;
let privacyGeneration = 0;
type FreezeLease = {
  id: string;
  privacyGeneration: number;
  retiring: boolean;
  targets: Map<RegisteredBinding, { source: WindowProxy; documentId?: string }>;
  pending: Set<Promise<void>>;
  failures: unknown[];
};
// A lease gates admissions before its first asynchronous drain. A snapshot of
// current frames alone misses pending open_article/sensor activation replies.
const freezeLeases = new Map<string, FreezeLease>();
const deferredRestores = new Set<() => void>();
type NavigationWaiter = { resolve(): void; reject(error: unknown): void };
const navigationWaiters = new Set<NavigationWaiter>();
let suppressFreezeRestore = false;
// GC is a fallback for detached contexts; live, reused contexts settle unused
// reservations at a navigation/message boundary instead.
const abandonedReservations = new FinalizationRegistry<RegisteredBinding>(
  (binding) => {
    reservedBytes -= binding.reserve;
    binding.reserve = 0;
    reservations.delete(binding);
    removeOriginBinding(binding);
  },
);
type DrainMessage = {
  type: "kiwix-storage-drain";
  id: string;
  freeze?: string;
  documentId?: string;
};
const drains = new Map<
  string,
  {
    source: WindowProxy;
    binding: RegisteredBinding;
    message: DrainMessage;
    resolve: () => void;
  }
>();
const fences = new Map<string, () => void>();
const encoder = new TextEncoder();
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
let writing: Promise<void> | undefined;
let flushing: Promise<void> | undefined;
let retry: ReturnType<typeof setTimeout> | undefined;
let reportedFailure = false;

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function fields(value: Record<string, unknown>, names: string[]): boolean {
  const keys = Object.keys(value);
  return keys.length === names.length && names.every((key) => key in value);
}
function boundedBytes(value: unknown, limit: number): number | undefined {
  if (
    typeof value !== "string" ||
    value.length > limit ||
    /[\uD800-\uDFFF]/u.test(value)
  )
    return undefined;
  const size = encoder.encode(value).length;
  return size <= limit ? size : undefined;
}
function validOperation(value: unknown): value is Record<string, unknown> {
  if (!object(value)) return false;
  if (value.kind === "local-clear") return fields(value, ["kind"]);
  if (boundedBytes(value.key, 8192) === undefined) return false;
  if (value.kind === "local-remove" || value.kind === "cookie-remove")
    return fields(value, ["kind", "key"]);
  if (value.kind === "local-set")
    return (
      fields(value, ["kind", "key", "value"]) &&
      boundedBytes(value.value, 1024 * 1024) !== undefined
    );
  if (
    value.kind !== "cookie-set" ||
    !fields(value, ["kind", "key", "cookie"]) ||
    !object(value.cookie)
  )
    return false;
  const cookie = value.cookie;
  // Reuse the bounded sizes: the name/value budget is the same measurement.
  const name = boundedBytes(cookie.name, 4096);
  const secret = boundedBytes(cookie.value, 4096);
  return (
    fields(cookie, ["name", "value", "path", "expires"]) &&
    name !== undefined &&
    secret !== undefined &&
    name + secret <= 4096 &&
    boundedBytes(cookie.path, 8192) !== undefined &&
    typeof cookie.path === "string" &&
    cookie.path.startsWith("/") &&
    typeof cookie.expires === "number" &&
    Number.isFinite(cookie.expires) &&
    value.key === `${cookie.path}\n${cookie.name}`
  );
}
/** Validates a batch and returns the payload size already spent measuring it. */
function validBatch(
  value: unknown,
): { batch: StorageBatch; bytes: number } | undefined {
  if (
    !object(value) ||
    !fields(value, ["id", "documentId", "sequence", "operations"])
  )
    return undefined;
  const { id, documentId, sequence, operations } = value;
  if (
    typeof id !== "string" ||
    !uuid.test(id) ||
    typeof documentId !== "string" ||
    !uuid.test(documentId) ||
    typeof sequence !== "number" ||
    !Number.isSafeInteger(sequence) ||
    sequence < 1 ||
    !Array.isArray(operations) ||
    operations.length < 1 ||
    operations.length > 10000
  )
    return undefined;
  // Measure the aggregate before the per-operation pass so an oversized frame
  // is refused without validating operations the credit can never admit.
  const bytes = encoder.encode(JSON.stringify(value)).length;
  if (bytes > 2 * 1024 * 1024 || !operations.every(validOperation))
    return undefined;
  return { batch: { id, documentId, sequence, operations }, bytes };
}

function report(error: unknown): void {
  window.dispatchEvent(
    new CustomEvent("kiwix-reader-storage-error", { detail: error }),
  );
}
function send(stream: Stream, message: Record<string, unknown>): void {
  stream.source.postMessage(
    { ...message, documentId: stream.documentId },
    stream.binding.origin,
  );
}
function releaseClosed(stream: Stream): void {
  if (!stream.queued && stream.closed) {
    streams.delete(stream);
    releaseBinding(stream.source, stream.binding);
  }
}
function removeOriginBinding(binding: RegisteredBinding): void {
  const registered = origins.get(binding.origin);
  registered?.delete(binding);
  if (!registered?.size) origins.delete(binding.origin);
}
function releaseBinding(source: WindowProxy, binding: RegisteredBinding): void {
  if (binding.reserve || active.get(source) === binding) return;
  for (const stream of streams) if (stream.binding === binding) return;
  // An unconsumed registration can still authenticate a late initial ready
  // after its unused allowance was settled. Its source remains weak.
  if (!binding.documentId && !binding.retired) return;
  bindings.get(source)?.delete(binding);
  removeOriginBinding(binding);
  abandonedReservations.unregister(binding);
}
function closeStream(stream: Stream): void {
  if (stream.closed) return;
  if (stream.binding.documentId === stream.documentId)
    stream.binding.ready = false;
  stream.closed = true;
  reservedBytes -= stream.available;
  stream.available = 0;
  releaseClosed(stream);
}
function failStream(
  stream: Stream,
  error: Error,
  retired = false,
  discardPending = true,
): void {
  const alreadyFailed = stream.failed;
  stream.failed = true;
  if (!retired) stream.binding.failed = error;
  closeStream(stream);
  if (discardPending) {
    // One compaction pass instead of a splice per matching batch.
    let write = queueHead;
    for (let read = queueHead; read < queue.length; read++) {
      const item = queue[read];
      if (item.stream === stream) {
        reservedBytes -= item.bytes;
        stream.queued -= 1;
        stream.queuedBytes -= item.bytes;
      } else queue[write++] = item;
    }
    queue.length = write;
  }
  releaseClosed(stream);
  if (!alreadyFailed) {
    send(stream, { type: "kiwix-storage-failed" });
    report(error);
  }
}

function registeredAuthority(
  origin: string,
  generation: unknown,
  expectedSource: WindowProxy,
): { source: WindowProxy; binding: RegisteredBinding } | undefined {
  let authority:
    { source: WindowProxy; binding: RegisteredBinding } | undefined;
  for (const binding of origins.get(origin) ?? []) {
    const source = binding.source.deref();
    if (
      !source ||
      source !== expectedSource ||
      binding.privacyGeneration !== generation
    )
      continue;
    if (
      authority &&
      (authority.source !== source ||
        authority.binding.bookId !== binding.bookId)
    )
      return;
    // Repeated registrations of one source/authority are not ambiguous. Prefer
    // its live binding so drains follow the document currently being admitted.
    if (
      !authority ||
      active.get(source) === binding ||
      (active.get(source) !== authority.binding &&
        !authority.binding.reserve &&
        binding.reserve)
    )
      authority = { source, binding };
  }
  return authority;
}

function settleLoadedDocuments(
  source: WindowProxy,
  frame: HTMLIFrameElement,
): Promise<void> {
  const binding = active.get(source);
  const pending = [...(bindings.get(source) ?? [])].filter(
    (registered) => registered !== binding && registered.reserve,
  );
  return messageFence().then(() => {
    if (
      !frame.isConnected ||
      frame.contentWindow !== source ||
      active.get(source) !== binding
    )
      return;
    for (const registered of pending) {
      if (registered === active.get(source) || !registered.reserve) continue;
      reservedBytes -= registered.reserve;
      registered.reserve = 0;
      reservations.delete(registered);
    }
    const current = currentDocuments.get(source);
    // A same-origin successor's ready is emitted after outgoing pagehide and
    // its microtasks. Revisit on both ready and hiding: a load can arrive before
    // the old document's advisory packet is delivered.
    for (const stream of streams) {
      if (
        stream.source === source &&
        stream.hiding &&
        stream.binding.origin === binding?.origin &&
        stream.documentId !== current &&
        current !== undefined
      )
        closeStream(stream);
    }
  });
}

/** Explicit retirement is not a retryable IO failure and cannot block a new session. */
export function retireReaderStorageGeneration(newGeneration: number): void {
  if (
    !Number.isSafeInteger(newGeneration) ||
    newGeneration <= privacyGeneration
  )
    return;
  privacyGeneration = newGeneration;
  reportedFailure = false;
  deferredRestores.clear();
  suppressFreezeRestore = false;
  for (const [id, lease] of freezeLeases)
    if (lease.privacyGeneration < newGeneration) freezeLeases.delete(id);
  for (const [id, drain] of drains) {
    if (drain.binding.privacyGeneration < newGeneration) {
      drains.delete(id);
      drain.resolve();
    }
  }
  for (const stream of [...streams]) {
    if (stream.binding.privacyGeneration < newGeneration) {
      if (stream.queued)
        failStream(
          stream,
          new Error(
            t(
              "Archive storage edits could not be saved because the privacy session changed.",
            ),
          ),
          true,
        );
      else {
        stream.failed = true;
        closeStream(stream);
        send(stream, { type: "kiwix-storage-failed" });
      }
    }
  }
  for (const binding of reservations) {
    if (binding.privacyGeneration < newGeneration && binding.reserve) {
      reservedBytes -= binding.reserve;
      binding.reserve = 0;
      reservations.delete(binding);
    }
  }
  settleNavigationWaiters();
}

function pump(): Promise<void> {
  if (writing) return writing;
  clearTimeout(retry);
  retry = undefined;
  const task = (async () => {
    while (queued()) {
      const item = queue[queueHead];
      const binding = item.stream.binding;
      const result = await api<{
        status: "committed" | "retired" | "rejected";
        privacyGeneration?: number;
        message?: string;
      }>("reader_storage_write", {
        origin: binding.origin,
        bookId: binding.bookId,
        privacyGeneration: binding.privacyGeneration,
        batch: item.batch,
      });
      if (result.status === "retired") {
        if (typeof result.privacyGeneration !== "number")
          throw new Error(t("Kiwix could not finish closing the reader. Close and reopen this window to continue."));
        retireReaderStorageGeneration(result.privacyGeneration);
        continue;
      }
      // A native reply can arrive after explicit App retirement removed the item.
      if (queue[queueHead] !== item) continue;
      if (result.status === "rejected") {
        failStream(
          item.stream,
          new Error(
            result.message ||
              t("Archive storage rejected an invalid mutation."),
          ),
        );
        continue;
      }
      if (result.status !== "committed")
        throw new Error("Invalid native storage acknowledgement");
      dequeue();
      item.stream.queued -= 1;
      item.stream.queuedBytes -= item.bytes;
      if (item.stream.closed) reservedBytes -= item.bytes;
      else item.stream.available += item.bytes;
      send(item.stream, {
        type: "kiwix-storage-ack",
        id: item.batch.id,
        sequence: item.batch.sequence,
      });
      releaseClosed(item.stream);
      reportedFailure = false;
    }
  })();
  writing = task;
  void task.then(
    () => {
      if (writing === task) writing = undefined;
      if (queued()) void pump().catch(() => undefined);
    },
    (error: unknown) => {
      if (writing === task) writing = undefined;
      if (!reportedFailure) {
        reportedFailure = true;
        report(error);
      }
      // Only retry durable failures. An explicitly retired/invalid stream was
      // removed above without blocking unrelated documents or generations.
      if (queued())
        retry = setTimeout(() => {
          void pump().catch(() => undefined);
        }, 1000);
    },
  );
  return task;
}

window.addEventListener("message", (event: MessageEvent<unknown>) => {
  const message = event.data;
  if (!object(message)) return;
  if (
    event.source === window &&
    event.origin === location.origin &&
    message.type === "kiwix-storage-fence" &&
    typeof message.id === "string"
  ) {
    const resolve = fences.get(message.id);
    if (resolve) {
      fences.delete(message.id);
      resolve();
    }
    return;
  }
  if (event.source !== null && !("postMessage" in event.source)) return;
  const eventSource = event.source as WindowProxy | null;
  // Only the registered direct reader WindowProxy can introduce a producer.
  // An origin-only initial ready could instead come from a removed child frame.
  if (message.type === "kiwix-storage-ready" && !eventSource) return;
  if (
    message.type === "kiwix-storage-drained" &&
    fields(message, ["type", "id", "documentId"]) &&
    typeof message.id === "string"
  ) {
    const drain = drains.get(message.id);
    // A close may already have released the old document stream. Its drain
    // keeps its own authority instead of borrowing the WindowProxy's new one.
    if (
      drain &&
      (!eventSource || drain.source === eventSource) &&
      drain.binding.origin === event.origin &&
      typeof drain.message.documentId === "string" &&
      drain.message.documentId === message.documentId
    ) {
      drains.delete(message.id);
      drain.resolve();
    }
    return;
  }
  // WindowProxy survives navigation. Old documents retain their own authority
  // until their ordered final mutations and close have reached the shell.
  const documentId =
    message.type === "kiwix-storage-batch" && object(message.batch)
      ? message.batch.documentId
      : message.documentId;
  let documentStream: Stream | undefined;
  for (const stream of streams) {
    if (
      stream.documentId !== documentId ||
      (eventSource
        ? stream.source !== eventSource
        : stream.binding.origin !== event.origin)
    )
      continue;
    // A document nonce is not authority by itself. Ambiguous null-source
    // matches fail closed instead of selecting a source/registration by order.
    if (documentStream) return;
    documentStream = stream;
  }
  let source = documentStream?.source ?? eventSource;
  let binding = documentStream?.binding;
  if (!binding && message.type === "kiwix-storage-ready" && eventSource) {
    const authority = registeredAuthority(
      event.origin,
      message.privacyGeneration,
      eventSource,
    );
    source = authority?.source ?? null;
    binding = authority?.binding;
  }
  if (!source || !binding || event.origin !== binding.origin) return;
  if (
    message.type === "kiwix-storage-ready" &&
    fields(message, ["type", "documentId", "sequence", "privacyGeneration"]) &&
    message.privacyGeneration === binding.privacyGeneration &&
    typeof message.documentId === "string" &&
    uuid.test(message.documentId) &&
    typeof message.sequence === "number" &&
    Number.isSafeInteger(message.sequence) &&
    message.sequence >= 0
  ) {
    if (binding.retired || binding.quarantined) return;
    if (binding.privacyGeneration < privacyGeneration || binding.failed) {
      source.postMessage(
        { type: "kiwix-storage-failed", documentId: message.documentId },
        binding.origin,
      );
      return;
    }
    let stream = documentStream;
    if (
      stream &&
      (stream.binding !== binding || message.sequence !== stream.sequence)
    )
      return;
    // Successor identity is independent of admission capacity: even a new
    // document refused credit has completed the outgoing same-origin dispatch.
    if (active.get(source) === binding)
      currentDocuments.set(source, message.documentId);
    const frame = activeFrames.get(source);
    if (frame) void settleLoadedDocuments(source, frame);
    if (!stream) {
      if (!binding.reserve && reservedBytes + SOURCE_CREDIT > ROOT_CREDIT) {
        const error = new Error(
          t("Archive storage is waiting for persistence acknowledgement"),
        );
        binding.failed = error;
        report(error);
        source.postMessage(
          { type: "kiwix-storage-failed", documentId: message.documentId },
          binding.origin,
        );
        return;
      }
      if (!binding.reserve) reservedBytes += SOURCE_CREDIT;
      binding.reserve = 0;
      reservations.delete(binding);
      stream = {
        source,
        binding,
        documentId: message.documentId,
        sequence: message.sequence,
        available: SOURCE_CREDIT,
        closed: false,
        hiding: false,
        failed: false,
        queued: 0,
        queuedBytes: 0,
      };
      streams.add(stream);
    } else if (stream.closed && !stream.failed) {
      const available = SOURCE_CREDIT - stream.queuedBytes;
      if (reservedBytes + available > ROOT_CREDIT) {
        failStream(
          stream,
          new Error(
            t("Archive storage is waiting for persistence acknowledgement"),
          ),
        );
        return;
      }
      stream.closed = false;
      streams.add(stream);
      stream.available = available;
      reservedBytes += available;
    }
    stream.hiding = false;
    binding.documentId = stream.documentId;
    binding.ready = true;
    // This watermark prevents queued pre-BFCache acknowledgements from adding
    // credit a second time after the restored document receives its snapshot.
    let outstanding: QueuedBatch | undefined;
    for (let index = queueHead; index < queue.length; index++)
      if (queue[index].stream === stream) {
        outstanding = queue[index];
        break;
      }
    send(stream, {
      type: "kiwix-storage-credit",
      bytes: stream.available,
      sequence: outstanding ? outstanding.batch.sequence - 1 : stream.sequence,
    });
    for (const drain of drains.values()) {
      if (drain.source !== source || drain.binding !== binding) continue;
      drain.message.documentId = stream.documentId;
      source.postMessage(drain.message, binding.origin);
    }
    for (const lease of freezeLeases.values())
      adoptFreeze(lease, source, binding);
  } else if (
    message.type === "kiwix-storage-hiding" &&
    fields(message, ["type", "documentId", "sequence", "privacyGeneration"]) &&
    documentStream &&
    message.privacyGeneration === binding.privacyGeneration &&
    typeof message.sequence === "number" &&
    Number.isSafeInteger(message.sequence) &&
    message.sequence >= 0 &&
    message.sequence <= documentStream.sequence
  ) {
    // Advisory only: archive pagehide handlers and their microtasks may still
    // write. A same-origin successor ready and root fence establish completion.
    documentStream.hiding = true;
    const frame = activeFrames.get(source);
    if (frame) void settleLoadedDocuments(source, frame);
  } else if (
    message.type === "kiwix-storage-closed" &&
    fields(message, ["type", "documentId", "sequence"])
  ) {
    if (documentStream && message.sequence === documentStream.sequence)
      closeStream(documentStream);
  } else if (message.type === "kiwix-storage-batch") {
    if (binding.privacyGeneration < privacyGeneration) {
      if (!binding.retirementReported) {
        binding.retirementReported = true;
        report(
          new Error(
            t(
              "Archive storage edits could not be saved because the privacy session changed.",
            ),
          ),
        );
      }
      return;
    }
    const stream = documentStream;
    if (!stream) {
      report(new Error(t("Archive storage rejected an invalid mutation.")));
      return;
    }
    if (stream.failed || stream.binding.privacyGeneration < privacyGeneration)
      return;
    const admitted =
      fields(message, ["type", "privacyGeneration", "batch"]) &&
      message.privacyGeneration === binding.privacyGeneration
        ? validBatch(message.batch)
        : undefined;
    if (!admitted) {
      failStream(
        stream,
        new Error(t("Archive storage rejected an invalid mutation.")),
        false,
        false,
      );
      return;
    }
    const { batch, bytes } = admitted;
    if (
      stream.closed ||
      batch.sequence !== stream.sequence + 1 ||
      bytes > stream.available
    ) {
      failStream(
        stream,
        new Error(t("Archive storage rejected an invalid mutation.")),
        false,
        false,
      );
      return;
    }
    stream.available -= bytes;
    stream.sequence = batch.sequence;
    stream.queued += 1;
    stream.queuedBytes += bytes;
    queue.push({ stream, batch, bytes });
    void pump().catch(() => undefined);
  }
});

/**
 * Register trusted native origin/book/generation before starting navigation.
 * A null result forbids navigation. Dispose stops active ownership, not already-
 * posted document authority; pass the real frame to settle aborted reservations.
 */
export function registerReaderStorage(
  source: WindowProxy,
  binding: Binding,
  frame?: HTMLIFrameElement,
  onRetire?: () => void | Promise<void>,
  onRestore?: (url: string) => void,
): (() => void) | null {
  // Callers must not start navigation after a null admission.
  if (!canNavigateReaderStorage()) return null;
  try {
    const url = new URL(binding.retirementUrl);
    if (
      url.origin !== binding.origin ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      !/^\/\.kiwix\/[0-9a-f]{32}\/retire\.html$/.test(url.pathname) ||
      url.href !== binding.retirementUrl ||
      typeof binding.retirementProof !== "string" ||
      !/^[0-9a-f]{32}$/.test(binding.retirementProof)
    )
      return null;
  } catch {
    return null;
  }
  // A fresh native nonce origin has exactly one book/installation generation.
  // Repeated registrations/sources may share it, but may not redefine it.
  for (const existing of origins.get(binding.origin) ?? []) {
    if (
      existing.source.deref() &&
      (existing.bookId !== binding.bookId ||
        existing.privacyGeneration !== binding.privacyGeneration)
    ) {
      report(new Error(t("Archive storage rejected an invalid mutation.")));
      return null;
    }
  }
  if (
    binding.privacyGeneration < privacyGeneration ||
    reservedBytes + SOURCE_CREDIT > ROOT_CREDIT
  ) {
    report(
      new Error(
        t("Archive storage is waiting for persistence acknowledgement"),
      ),
    );
    return null;
  }
  const registered: RegisteredBinding = {
    ...binding,
    source: new WeakRef(source),
    onRetire,
    onRestore,
    ready: false,
    reserve: SOURCE_CREDIT,
  };
  reservedBytes += SOURCE_CREDIT;
  reservations.add(registered);
  abandonedReservations.register(source, registered, registered);
  let registeredBindings = bindings.get(source);
  if (!registeredBindings) {
    registeredBindings = new Set();
    bindings.set(source, registeredBindings);
  }
  registeredBindings.add(registered);
  let originBindings = origins.get(binding.origin);
  if (!originBindings) {
    originBindings = new Set();
    origins.set(binding.origin, originBindings);
  }
  originBindings.add(registered);
  active.set(source, registered);
  if (frame) activeFrames.set(source, frame);
  else activeFrames.delete(source);
  const loaded = (event: Event) => {
    if (!event.isTrusted || !frame || frame.contentWindow !== source) return;
    void settleLoadedDocuments(source, frame);
  };
  frame?.addEventListener("load", loaded);
  return () => {
    frame?.removeEventListener("load", loaded);
    if (active.get(source) === registered) {
      active.delete(source);
      activeFrames.delete(source);
    }
    releaseBinding(source, registered);
    // Genuine navigation, not iframe removal, runs archive pagehide handlers.
    // Retain weak authority until their final batches and close are received.
  };
}

/**
 * Record a URL only after Reader validated the current source/origin/session
 * and article path. The active source+origin+generation binding must still
 * match. Rollback uses the latest URL validated for this same binding.
 */
export function updateReaderStorageLocation(
  source: WindowProxy,
  location: { origin: string; privacyGeneration: number; url: string },
): boolean {
  const binding = active.get(source);
  if (
    !binding ||
    binding.origin !== location.origin ||
    binding.privacyGeneration !== location.privacyGeneration ||
    binding.privacyGeneration < privacyGeneration
  )
    return false;
  try {
    const url = new URL(location.url);
    if (url.origin !== binding.origin) return false;
    binding.currentUrl = url.href;
    return true;
  } catch {
    return false;
  }
}

/** Admission must be checked again after asynchronous open/activation replies. */
export function canNavigateReaderStorage(): boolean {
  return !retirementActive && !freezeLeases.size && !quarantinedBindings.size;
}

/** Await an owned checkpoint/retirement, never poll or discard its writes. */
export function waitForReaderStorageNavigation(signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(signal.reason);
  if (canNavigateReaderStorage()) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const finish = (error?: unknown) => {
      navigationWaiters.delete(waiter);
      signal.removeEventListener("abort", abort);
      if (error !== undefined) reject(error);
      else resolve();
    };
    const abort = () => finish(signal.reason);
    const waiter: NavigationWaiter = {
      resolve: () => finish(),
      reject: (error) => finish(error),
    };
    navigationWaiters.add(waiter);
    signal.addEventListener("abort", abort, { once: true });
    settleNavigationWaiters();
  });
}

function settleNavigationWaiters(failure?: unknown): void {
  if (quarantinedBindings.size) failure = new Error(t(
    "Kiwix could not confirm that the article finished saving. Reading is paused to protect it. Close and reopen this window to continue.",
  ));
  if (failure === undefined && !canNavigateReaderStorage()) return;
  for (const waiter of navigationWaiters) {
    if (failure !== undefined) waiter.reject(failure);
    else waiter.resolve();
  }
}

function restoreDeferredDocuments(): void {
  settleNavigationWaiters();
  if (retirementActive || freezeLeases.size || quarantinedBindings.size) return;
  const pending = [...deferredRestores];
  deferredRestores.clear();
  if (!suppressFreezeRestore) for (const restore of pending) restore();
}

/**
 * Call before force-removing a security-revoked frame whose handoff failed.
 * Known authenticated late packets keep their existing sequence/credit/native
 * listener; quarantine is not a close, a drain, or permission for fresh opens.
 */
export function quarantineReaderStorage(source: WindowProxy): void {
  for (const binding of bindings.get(source) ?? []) {
    binding.quarantined = true;
    binding.restoreBlocked = true;
    quarantinedBindings.add(binding);
  }
  if (quarantinedBindings.size) {
    retirementActive = true;
    suppressFreezeRestore = true;
    deferredRestores.clear();
  }
  settleNavigationWaiters();
}

/**
 * Discard quarantined authority after the shell has replaced the affected
 * frames. Late packets become unknown rather than reusing an unproved origin;
 * this recovers the window without claiming that the lost writes were saved.
 */
export function discardQuarantinedReaderStorage(): void {
  if (!quarantinedBindings.size) return;
  for (const binding of quarantinedBindings) {
    for (const stream of [...streams])
      if (stream.binding === binding) closeStream(stream);
    const source = binding.source.deref();
    if (source) {
      if (active.get(source) === binding) {
        active.delete(source);
        activeFrames.delete(source);
        currentDocuments.delete(source);
      }
      bindings.get(source)?.delete(binding);
    }
    binding.retired = true;
    binding.ready = false;
    binding.quarantined = false;
    if (binding.reserve) {
      reservedBytes -= binding.reserve;
      binding.reserve = 0;
    }
    reservations.delete(binding);
    removeOriginBinding(binding);
    abandonedReservations.unregister(binding);
  }
  quarantinedBindings.clear();
  retirementActive = false;
  suppressFreezeRestore = false;
  deferredRestores.clear();
  settleNavigationWaiters();
}

function retireDocuments(restoreOnFailure = true): Promise<() => void> {
  if (quarantinedBindings.size)
    return Promise.reject(
      new Error(
        t(
          "Kiwix could not confirm that the article finished saving. Reading is paused to protect it. Close and reopen this window to continue.",
        ),
      ),
    );
  if (!restoreOnFailure) {
    // Security revocation must not reactivate a cached-grant origin, including
    // rollback owned by an already-running shared retirement.
    suppressFreezeRestore = true;
    deferredRestores.clear();
    for (const binding of active.values()) binding.restoreBlocked = true;
  }
  if (retiringDocuments) return retiringDocuments;
  retirementActive = true;
  if (!freezeLeases.size && restoreOnFailure) suppressFreezeRestore = false;
  // A completed freeze already navigated these bindings to their trusted
  // successor and drained their writes. Never demand a second pagehide from
  // that successor when ReaderHost subsequently removes the retired node.
  const targets = [...active].filter(([, binding]) => !binding.retired).map(([source, binding]) => {
    let deactivation: Promise<void>;
    try {
      // Sensor admission must stop synchronously, before any frame navigates.
      deactivation = Promise.resolve(binding.onRetire?.());
    } catch (error) {
      deactivation = Promise.reject(error);
    }
    return {
      source,
      binding,
      frame: activeFrames.get(source),
      url: binding.currentUrl,
      deactivation,
    };
  });
  const restore = () => {
    if (retirementActive || freezeLeases.size) {
      deferredRestores.add(restore);
      return;
    }
    for (const { source, binding, frame, url } of targets) {
      const currentUrl = binding.currentUrl ?? url;
      if (
        binding.restoreBlocked ||
        !frame?.isConnected ||
        frame.contentWindow !== source ||
        !currentUrl ||
        active.get(source) !== binding ||
        binding.privacyGeneration < privacyGeneration
      )
        continue;
      binding.retired = false;
      if (binding.onRestore) binding.onRestore(currentUrl);
      else frame.src = currentUrl;
    }
  };
  const task = (async () => {
    try {
      const results = await Promise.allSettled(
        targets.flatMap(({ source, binding, frame, deactivation }) => [
          deactivation,
          new Promise<void>((resolve, reject) => {
            const error = new Error(
              t("The archive could not finish closing its document."),
            );
            if (
              !frame?.isConnected ||
              frame.contentWindow !== source ||
              active.get(source) !== binding
            ) {
              reject(error);
              return;
            }
            const id = crypto.randomUUID();
            const url = `${binding.retirementUrl}?retire=${id}`;
            const stillCurrent = () =>
              frame.isConnected &&
              frame.contentWindow === source &&
              active.get(source) === binding &&
              binding.privacyGeneration >= privacyGeneration &&
              frame.src === url;
            const cleanup = () => {
              clearTimeout(timer);
              window.removeEventListener("message", acknowledged);
            };
            const finish = async () => {
              // This same-origin successor runs after the entire outgoing
              // subtree's pagehide dispatch and microtasks. Child mutations
              // used the direct frame's synchronous producer/delivery channel.
              await messageFence();
              if (!stillCurrent()) throw error;
              for (const registered of bindings.get(source) ?? []) {
                registered.retired = true;
                registered.ready = false;
                reservedBytes -= registered.reserve;
                registered.reserve = 0;
                reservations.delete(registered);
              }
              // Hiding is advisory and can arrive late. The trusted successor
              // retires every outgoing stream, not only ones already hiding.
              for (const stream of [...streams])
                if (stream.source === source) closeStream(stream);
              for (const registered of bindings.get(source) ?? [])
                releaseBinding(source, registered);
              currentDocuments.delete(source);
            };
            const acknowledged = (event: MessageEvent<unknown>) => {
              if (
                event.source !== source ||
                event.origin !== binding.origin ||
                !object(event.data) ||
                !fields(event.data, ["type", "id", "proof"]) ||
                event.data.type !== "kiwix-storage-retired" ||
                event.data.id !== id ||
                event.data.proof !== binding.retirementProof ||
                !stillCurrent()
              )
                return;
              cleanup();
              void finish().then(resolve, reject);
            };
            const timer = setTimeout(() => {
              cleanup();
              reject(error);
            }, 5000);
            window.addEventListener("message", acknowledged);
            // Keep the actual frame alive through its real pagehide and
            // durable handoff. DOM removal cannot provide this boundary.
            try {
              frame.src = url;
            } catch (error) {
              cleanup();
              reject(error);
            }
          }),
        ]),
      );
      const failed = results.find((result) => result.status === "rejected");
      if (failed?.status === "rejected") throw failed.reason;
      await flushReaderStorage();
      for (const { source, binding, frame } of targets)
        if (
          !frame?.isConnected ||
          frame.contentWindow !== source ||
          active.get(source) !== binding
        )
          throw new Error(
            t("The archive could not finish closing its document."),
          );
      return restore;
    } catch (error) {
      if (restoreOnFailure) deferredRestores.add(restore);
      throw error;
    }
  })();
  retiringDocuments = task;
  void task.then(
    () => {
      if (retiringDocuments === task) retiringDocuments = undefined;
      retirementActive = false;
      restoreDeferredDocuments();
    },
    (error: unknown) => {
      if (retiringDocuments === task) retiringDocuments = undefined;
      // An unproved security handoff must not allow open_article to replace
      // the old native listener while accepted batches still belong to it.
      // A subsequent genuine retirement can settle it and release admission.
      retirementActive = [...active.values()].some(
        (binding) => binding.restoreBlocked,
      );
      settleNavigationWaiters(error);
      restoreDeferredDocuments();
    },
  );
  return task;
}

/**
 * Navigate immediately, then finish genuine subtree pagehide and durable
 * ownership before callers remove reader nodes or replace the native listener.
 * Security revocations must disable restoration of the outgoing nonce origin.
 */
export async function retireReaderDocuments({
  restoreOnFailure = true,
  discardQuarantine = false,
}: { restoreOnFailure?: boolean; discardQuarantine?: boolean } = {}): Promise<void> {
  if (discardQuarantine) discardQuarantinedReaderStorage();
  await retireDocuments(restoreOnFailure);
}

function messageFence(): Promise<void> {
  return new Promise((resolve) => {
    const id = crypto.randomUUID();
    fences.set(id, resolve);
    window.postMessage({ type: "kiwix-storage-fence", id }, location.origin);
  });
}
function drainFrame(
  source: WindowProxy,
  binding: RegisteredBinding,
  freeze?: string,
): Promise<void> {
  if (binding.retired) return Promise.resolve();
  if (binding.privacyGeneration < privacyGeneration) return Promise.resolve();
  if (binding.failed) return Promise.reject(binding.failed);
  if (!binding.ready && !freeze) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    const message: DrainMessage = {
      type: "kiwix-storage-drain",
      id,
      ...(freeze ? { freeze } : {}),
      ...(binding.ready ? { documentId: binding.documentId } : {}),
    };
    const timer = setTimeout(() => {
      drains.delete(id);
      reject(
        new Error(t("The archive did not acknowledge its storage drain.")),
      );
    }, 5000);
    drains.set(id, {
      source,
      binding,
      message,
      resolve: () => {
        clearTimeout(timer);
        resolve();
      },
    });
    if (binding.ready) source.postMessage(message, binding.origin);
  });
}

/** Await handoff from live frames before the trusted shell retires its session. */
export function flushReaderStorage(): Promise<void> {
  if (flushing) return flushing;
  const task = (async () => {
    await Promise.all(
      [...active]
        .filter(([, binding]) => !binding.quarantined)
        .map(([source, binding]) => drainFrame(source, binding)),
    );
    do {
      // Also receive already-posted pagehide mutations from detached frames.
      await messageFence();
      await pump();
      await messageFence();
      const failed = [...active.values()].find(
        (binding) =>
          binding.privacyGeneration >= privacyGeneration && binding.failed,
      );
      if (failed?.failed) throw failed.failed;
    } while (queued());
    // Committing the known queue is not proof that a forcibly removed producer
    // has no more in-flight packets. Never advertise a complete final drain.
    if (quarantinedBindings.size)
      throw new Error(
        t(
          "Kiwix could not confirm that the article finished saving. Reading is paused to protect it. Close and reopen this window to continue.",
        ),
      );
  })();
  flushing = task;
  void task.then(
    () => {
      if (flushing === task) flushing = undefined;
    },
    () => {
      if (flushing === task) flushing = undefined;
    },
  );
  return task;
}

function adoptFreeze(
  lease: FreezeLease,
  source: WindowProxy,
  binding: RegisteredBinding,
): void {
  if (lease.retiring || binding.retired) return;
  const previous = lease.targets.get(binding);
  if (previous && previous.documentId === binding.documentId) return;
  lease.targets.set(binding, { source, documentId: binding.documentId });
  // A ready handshake replays an outstanding drain to the current document.
  // Do not create a second request just because the WindowProxy navigated.
  for (const drain of drains.values())
    if (drain.binding === binding && drain.message.freeze === lease.id) return;
  let pending: Promise<void>;
  pending = drainFrame(source, binding, lease.id).then(
    () => {
      lease.pending.delete(pending);
    },
    (error: unknown) => {
      lease.pending.delete(pending);
      lease.failures.push(error);
    },
  );
  lease.pending.add(pending);
}

/** Quiesce setters and synchronously gate admission until retirement or resume. */
export async function freezeReaderStorage(): Promise<
  (restore?: boolean) => void
> {
  if (!freezeLeases.size) {
    deferredRestores.clear();
    suppressFreezeRestore = false;
  }
  const lease: FreezeLease = {
    id: crypto.randomUUID(),
    privacyGeneration,
    retiring: true,
    targets: new Map(),
    pending: new Set(),
    failures: [],
  };
  freezeLeases.set(lease.id, lease);
  let resumed = false;
  let restoreDocuments: (() => void) | undefined;
  const resume = (restore = true) => {
    if (resumed) return;
    resumed = true;
    freezeLeases.delete(lease.id);
    for (const [binding, { source }] of lease.targets)
      if (binding.privacyGeneration >= privacyGeneration)
        source.postMessage(
          { type: "kiwix-storage-resume", freeze: lease.id },
          binding.origin,
        );
    if (lease.privacyGeneration < privacyGeneration) {
      settleNavigationWaiters();
      return;
    }
    if (!restore) suppressFreezeRestore = true;
    else if (restoreDocuments) deferredRestores.add(restoreDocuments);
    restoreDeferredDocuments();
  };
  try {
    restoreDocuments = await retireDocuments();
    lease.retiring = false;
    for (const [source, binding] of active) adoptFreeze(lease, source, binding);
    do {
      // Newly-ready documents are adopted while this awaits. Settle all
      // requests before rollback, so no late drain can refreeze a resumed page.
      while (lease.pending.size) await Promise.all(lease.pending);
      if (lease.failures.length) throw lease.failures[0];
      await flushReaderStorage();
    } while (lease.pending.size);
    return resume;
  } catch (error) {
    resume();
    throw error;
  }
}

registerPendingPreferenceDrain(flushReaderStorage);
registerPendingPreferenceCutoverDrain(async () => {
  const resume = await freezeReaderStorage();
  return (committed: boolean) => {
    if (!committed) resume();
  };
});
