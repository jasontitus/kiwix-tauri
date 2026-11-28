/* SPDX-License-Identifier: GPL-3.0-or-later */
/* The server bootstraps the trusted shell origin; only storage mutations cross it. */
(() => {
  "use strict";
  const bootstrap = window.__kiwixStorageBootstrap;
  delete window.__kiwixStorageBootstrap;
  if (
    !bootstrap ||
    typeof bootstrap.parentOrigin !== "string" ||
    !Number.isSafeInteger(bootstrap.privacyGeneration)
  )
    return;
  const installed = Symbol.for("kiwix.reader.storage.bridge");
  if (Object.prototype.hasOwnProperty.call(window, installed)) return;
  Object.defineProperty(window, installed, { value: true });
  // This script's message listener is registered before the reader bridge's,
  // so the shell's reader messages (which carry the reader session and its
  // photo secret) can pass through it first. Archive scripts can replace any
  // prototype accessor later; the event is therefore read only through natives
  // captured now, and a message that is not a storage message leaves the
  // listener before anything replaceable runs.
  const capturedGetter = (key) => {
    try {
      const getter = Object.getOwnPropertyDescriptor(MessageEvent.prototype, key)?.get;
      return getter ? Function.prototype.call.bind(getter) : null;
    } catch (_) {
      return null;
    }
  };
  const nativeMessageData = capturedGetter("data");
  const nativeMessageSource = capturedGetter("source");
  const nativeMessageOrigin = capturedGetter("origin");
  const hasOwnField = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
  // `parent` is replaceable by page script; the embedding window is not.
  const embedder = parent;
  // Descendants share one synchronous producer owned by the shell's direct
  // reader frame. Only that registered WindowProxy ever posts durable batches;
  // a detached child cannot acquire its own credit or root-message authority.
  const producerKey = Symbol.for("kiwix.reader.storage.producer");
  let ancestor = window;
  try {
    while (
      ancestor.parent !== ancestor &&
      ancestor.parent.location.origin === location.origin
    )
      ancestor = ancestor.parent;
  } catch (_) {
    // The shell is deliberately a different origin.
  }
  const upstream = ancestor === window ? null : ancestor[producerKey];
  if (ancestor !== window && !upstream) return;
  // This webview is nonpersistent and each reader origin is fresh. Keep the
  // browser's synchronous, same-origin memory across document navigations; the
  // shell still owns every accepted durable mutation, never this mirror.
  const nativeStorage = window.localStorage;
  const nativeGet = Storage.prototype.getItem.bind(nativeStorage);
  const nativeSet = Storage.prototype.setItem.bind(nativeStorage);
  const nativeRemove = Storage.prototype.removeItem.bind(nativeStorage);
  const nativeKey = Storage.prototype.key.bind(nativeStorage);
  const namespace = "\u0000kiwix.reader.storage.v1:";
  const localPrefix = `${namespace}local:`;
  const cookieKey = `${namespace}cookies`;
  const markerKey = `${namespace}initialized`;
  const freezeKey = `${namespace}freezes`;
  const nativeTransaction = (changes) => {
    const previous = changes.map(([key]) => [key, nativeGet(key)]);
    const restore = () => {
      // Free the changed allocation first so restoring a quota-sized previous
      // value cannot itself run out of space.
      for (const [key] of previous) nativeRemove(key);
      for (const [key, value] of previous)
        if (value !== null) nativeSet(key, value);
    };
    try {
      for (const [key] of changes) nativeRemove(key);
      for (const [key, value] of changes)
        if (value !== null) nativeSet(key, value);
    } catch (error) {
      restore();
      throw error;
    }
    return restore;
  };
  const readLocal = () => {
    const values = new Map();
    for (let index = 0; index < nativeStorage.length; index++) {
      const key = nativeKey(index);
      if (key?.startsWith(localPrefix)) {
        const value = nativeGet(key);
        if (value !== null) values.set(key.slice(localPrefix.length), value);
      }
    }
    return values;
  };
  const readCookies = () =>
    new Map(
      JSON.parse(nativeGet(cookieKey) || "[]")
        .filter(
          (item) =>
            item &&
            typeof item.name === "string" &&
            typeof item.value === "string" &&
            typeof item.path === "string" &&
            (item.expires === null ||
              (Number.isFinite(item.expires) && item.expires > Date.now())),
        )
        .map((item) => [`${item.path}\n${item.name}`, item]),
    );
  const readFreezes = () => new Set(JSON.parse(nativeGet(freezeKey) || "[]"));
  const updateFreeze = (freeze, add) => {
    const leases = readFreezes();
    if (add) leases.add(freeze);
    else leases.delete(freeze);
    if (leases.size) nativeSet(freezeKey, JSON.stringify([...leases]));
    else nativeRemove(freezeKey);
  };
  const parentOrigin = bootstrap.parentOrigin;
  const readOnly = bootstrap.readOnly;
  const privacyGeneration = bootstrap.privacyGeneration;
  const documentId = crypto.randomUUID();
  let sequence = 0;
  const initial =
    bootstrap.values && typeof bootstrap.values === "object"
      ? bootstrap.values
      : {};
  if (nativeGet(markerKey) === null) {
    const local = Object.entries(initial.localStorage || {}).filter(
      ([key, value]) => typeof key === "string" && typeof value === "string",
    );
    const seededCookies = (
      Array.isArray(initial.cookies) ? initial.cookies : []
    ).filter(
      (item) =>
        item &&
        typeof item.name === "string" &&
        typeof item.value === "string" &&
        typeof item.path === "string" &&
        Number.isFinite(item.expires) &&
        item.expires > Date.now(),
    );
    // The marker is deliberately not sessionStorage: an archive may clear its
    // own storage during logout without resurrecting an unacknowledged Profile.
    nativeTransaction([
      ...local.map(([key, value]) => [`${localPrefix}${key}`, value]),
      [cookieKey, JSON.stringify(seededCookies)],
      [markerKey, "1"],
    ]);
  }
  const encode = new TextEncoder();
  const bytes = (value) => encode.encode(value).length;
  // The quota metric is the UTF-8 length of
  //   JSON.stringify({ localStorage: {...}, cookies: [...] })
  // Its framing is constant and an object or array body is two braces plus one
  // separator per gap, so both halves stay measurable without serializing.
  const framing = bytes('{"localStorage":,"cookies":}');
  const body = (count, sum) => (count ? sum + count + 1 : 2);
  // One `"key":value` pair of the serialized store, in UTF-8 bytes.
  const entry = (key, value) =>
    bytes(JSON.stringify(key)) + 1 + bytes(JSON.stringify(value));
  // Mirror of the namespaced native keys plus their encoded sizes: a single-key
  // write must never re-read or re-serialize the whole store.
  let persistent = new Map();
  const costs = new Map();
  let total = 0;
  let keyList = null;
  let cookiePayload = -1;
  const loadLocal = () => {
    persistent = readLocal();
    costs.clear();
    total = 0;
    for (const [key, value] of persistent) {
      const cost = entry(key, value);
      costs.set(key, cost);
      total += cost;
    }
    keyList = null;
  };
  let cookies = readCookies();
  const refreshCookies = () => {
    cookies = readCookies();
    cookiePayload = -1;
  };
  const cookieBytes = () => {
    if (cookiePayload < 0)
      cookiePayload = bytes(JSON.stringify([...cookies.values()]));
    return cookiePayload;
  };
  // Enumeration must not pay one native read per key per call.
  const localKeys = () => {
    if (keyList) return keyList;
    const keys = [];
    for (let index = 0; index < nativeStorage.length; index++) {
      const key = nativeKey(index);
      if (key?.startsWith(localPrefix))
        keys.push(key.slice(localPrefix.length));
    }
    keyList = keys;
    return keys;
  };
  // Only the producer root mirrors the store; descendants forward operations.
  if (!upstream) loadLocal();
  const pending = new Map();
  // Per-document allowance for unacknowledged mutations. It is local to this
  // producer; the shell charges only batches that are actually admitted.
  let credit = 2 * 1024 * 1024;
  let closed = false;
  let failed = false;
  let awaitingCredit = false;
  // Sequence at the ready that requested a credit snapshot. Batches posted
  // after it are not reflected in that snapshot and must stay deducted.
  let snapshotBoundary = 0;
  // Mutations admitted in one task share a single bounded IPC batch. The
  // reservation is computed synchronously (so setItem still throws
  // QuotaExceededError before touching the mirror); only delivery is deferred.
  let pendingOperations = [];
  let pendingBody = 2; // bytes(JSON.stringify([]))
  let pendingReserved = 0;
  let flushScheduled = false;
  // Native mirror restores for operations whose IPC batch has not been posted.
  // A post failure rolls these back instead of losing the mutation silently.
  let pendingRestores = [];
  const batchSize = (bodyBytes, seq) =>
    bytes(`{"id":"${"0".repeat(36)}","documentId":"${documentId}","sequence":${seq}`) +
    bytes(',"operations":') +
    1 +
    bodyBytes;
  const writable = () => {
    if (readFreezes().size || closed || failed)
      throw new DOMException(
        "Archive storage is closing its session",
        "InvalidStateError",
      );
  };
  const quotaError = (message) =>
    new DOMException(message, "QuotaExceededError");
  // Native JSON strings are Unicode scalar values, not lone UTF-16 surrogates.
  const bounded = (value, limit) =>
    !/[\uD800-\uDFFF]/u.test(value) && bytes(value) <= limit;
  // Predicts the payload growth of a mutation without touching the mirror.
  const planLocal = (operations) => {
    const sizes = [];
    let sum = total;
    let count = persistent.size;
    for (let index = 0; index < operations.length; index++) {
      const operation = operations[index];
      sizes.push(0);
      if (operation.kind === "local-clear") {
        sum = 0;
        count = 0;
      } else if (operation.kind === "local-remove") {
        const cost = costs.get(operation.key);
        if (cost !== undefined) {
          sum -= cost;
          count -= 1;
        }
      } else if (operation.kind === "local-set") {
        const cost = entry(operation.key, operation.value);
        const previous = costs.get(operation.key);
        if (previous === undefined) count += 1;
        else sum -= previous;
        sum += cost;
        sizes[index] = cost;
      }
    }
    return { size: body(count, sum), sizes };
  };
  // Native mirror writes implied by a mutation; unchanged keys are skipped.
  const nativeChanges = (operations) => {
    const changes = [];
    for (const operation of operations) {
      if (operation.kind === "local-clear") {
        // Scan native truth: clear() must also drop keys another same-origin
        // document wrote behind this mirror.
        for (let index = 0; index < nativeStorage.length; index++) {
          const key = nativeKey(index);
          if (key?.startsWith(localPrefix)) changes.push([key, null]);
        }
      } else if (operation.kind === "local-remove") {
        // Removing an absent key is a native no-op, and this also drops keys
        // the mirror has not observed yet.
        changes.push([`${localPrefix}${operation.key}`, null]);
      } else if (
        operation.kind === "local-set" &&
        persistent.get(operation.key) !== operation.value
      )
        changes.push([`${localPrefix}${operation.key}`, operation.value]);
    }
    return changes;
  };
  // Commits an accepted mutation to the mirror and its running payload size.
  const applyLocal = (operations, sizes) => {
    for (let index = 0; index < operations.length; index++) {
      const operation = operations[index];
      if (operation.kind === "local-clear") {
        persistent = new Map();
        costs.clear();
        total = 0;
        keyList = null;
      } else if (operation.kind === "local-remove") {
        const cost = costs.get(operation.key);
        if (cost === undefined) continue;
        persistent.delete(operation.key);
        costs.delete(operation.key);
        total -= cost;
        keyList = null;
      } else if (operation.kind === "local-set") {
        const previous = costs.get(operation.key);
        if (previous === undefined) keyList = null;
        else total -= previous;
        total += sizes[index];
        costs.set(operation.key, sizes[index]);
        persistent.set(operation.key, operation.value);
      }
    }
  };
  const flushPending = () => {
    flushScheduled = false;
    if (!pendingOperations.length) return;
    if (upstream || readOnly) {
      pendingOperations = [];
      pendingBody = 2;
      pendingReserved = 0;
      return;
    }
    const operations = pendingOperations;
    const reserved = pendingReserved;
    pendingOperations = [];
    pendingBody = 2;
    pendingReserved = 0;
    const batch = {
      id: crypto.randomUUID(),
      documentId,
      sequence: sequence + 1,
      operations,
    };
    const size = bytes(JSON.stringify(batch));
    try {
      parent.postMessage(
        {
          type: "kiwix-storage-batch",
          privacyGeneration,
          batch,
        },
        parentOrigin,
      );
    } catch {
      // The producer already returned success for these mutations. Restore the
      // exact native values, rebuild the mirror, and surface the failure
      // instead of silently losing the batch.
      for (let index = pendingRestores.length - 1; index >= 0; index--) {
        try {
          pendingRestores[index]();
        } catch {
          // A restore failure cannot improve the already-failed session.
        }
      }
      pendingRestores = [];
      credit += reserved;
      loadLocal();
      refreshCookies();
      failed = true;
      closed = true;
      window.dispatchEvent(
        new CustomEvent("kiwix-reader-storage-error", {
          detail: new Error("Archive storage could not reach the shell"),
        }),
      );
      return;
    }
    // Replace the upper-bound reservation with the exact serialized size.
    credit += reserved - size;
    pendingRestores = [];
    pending.set(batch.id, { sequence: batch.sequence, size });
    sequence = batch.sequence;
  };
  const scheduleFlush = () => {
    if (flushScheduled || upstream || readOnly) return;
    flushScheduled = true;
    queueMicrotask(flushPending);
  };
  const changed = (operations, nextCookies) => {
    if (upstream) {
      upstream.commit(operations, nextCookies);
      keyList = null;
      return;
    }
    const plan = planLocal(operations);
    const cookieChanged = nextCookies !== undefined && nextCookies !== cookies;
    const cookieText = cookieChanged
      ? JSON.stringify([...nextCookies.values()])
      : null;
    const cookieSize = cookieChanged ? bytes(cookieText) : cookieBytes();
    // Include session cookies conservatively: the native persistent subset is
    // never larger than the synchronous compatibility view.
    if (framing + plan.size + cookieSize > 1024 * 1024)
      throw quotaError("Archive storage quota exceeded");
    // The shell validates at most 10000 operations per batch.
    if (operations.length > 10000)
      throw quotaError("Archive storage rejected an invalid mutation");
    if (!readOnly && pendingOperations.length + operations.length > 10000) {
      // Deliver the full batch before starting another one.
      flushPending();
      if (failed)
        throw new DOMException(
          "Archive storage is closing its session",
          "InvalidStateError",
        );
    }
    // Measure the coalesced batch before mutating anything. A synchronous
    // credit failure must not leave the compatibility mirror changed.
    let nextBody = pendingBody;
    let queued = pendingOperations.length;
    for (const operation of operations) {
      nextBody += (queued ? 1 : 0) + bytes(JSON.stringify(operation));
      queued += 1;
    }
    const reserved = readOnly
      ? pendingReserved
      : batchSize(nextBody, sequence + 1);
    if (reserved - pendingReserved > credit)
      throw quotaError(
        "Archive storage is waiting for persistence acknowledgement",
      );
    const changes = nativeChanges(operations);
    if (cookieChanged) changes.push([cookieKey, cookieText]);
    // Quota and credit are already checked; a native write failure still
    // restores the exact previous values.
    const rollback = nativeTransaction(changes);
    if (!readOnly) {
      pendingOperations.push(...operations);
      pendingBody = nextBody;
      credit -= reserved - pendingReserved;
      pendingReserved = reserved;
      pendingRestores.push(rollback);
    }
    applyLocal(operations, plan.sizes);
    if (cookieChanged) {
      cookies = nextCookies;
      cookiePayload = cookieSize;
    }
    scheduleFlush();
  };
  if (!upstream)
    Object.defineProperty(window, producerKey, {
      value: Object.freeze({
        commit(operations, nextCookies) {
          writable();
          changed(operations, nextCookies);
        },
      }),
    });
  const ready = () => {
    flushPending();
    parent.postMessage(
      {
        type: "kiwix-storage-ready",
        documentId,
        sequence,
        privacyGeneration,
      },
      parentOrigin,
    );
  };
  addEventListener("message", (event) => {
    if (upstream) return;
    if (
      (nativeMessageSource ? nativeMessageSource(event) : event.source) !== embedder ||
      (nativeMessageOrigin ? nativeMessageOrigin(event) : event.origin) !== parentOrigin
    )
      return;
    const message = nativeMessageData ? nativeMessageData(event) : event.data;
    if (message === null || typeof message !== "object" || !hasOwnField(message, "type")) return;
    const type = message.type;
    // Plain comparisons only: nothing an archive script can replace may run
    // while a reader message is being dispatched.
    if (
      type !== "kiwix-storage-credit" &&
      type !== "kiwix-storage-ack" &&
      type !== "kiwix-storage-failed" &&
      type !== "kiwix-storage-drain" &&
      type !== "kiwix-storage-resume"
    )
      return;
    if (
      message?.documentId === documentId &&
      message.type === "kiwix-storage-credit"
    ) {
      if (
        awaitingCredit &&
        Number.isSafeInteger(message.bytes) &&
        message.bytes >= 0 &&
        Number.isSafeInteger(message.sequence) &&
        message.sequence >= 0 &&
        message.sequence <= sequence
      ) {
        // A frozen document may receive old acknowledgements after this reply.
        // The credit snapshot already includes these committed mutations.
        for (const [id, item] of pending)
          if (item.sequence <= message.sequence) pending.delete(id);
        // Preserve credit for batches posted after the ready that requested
        // this snapshot; the shell had not processed them when it replied.
        let afterSnapshot = 0;
        for (const item of pending.values())
          if (item.sequence > snapshotBoundary) afterSnapshot += item.size;
        credit = Math.max(0, message.bytes - afterSnapshot);
        awaitingCredit = false;
      }
    } else if (
      message?.documentId === documentId &&
      message.type === "kiwix-storage-ack"
    ) {
      const item = pending.get(message.id);
      if (item && item.sequence === message.sequence) {
        pending.delete(message.id);
        credit += item.size;
      }
    } else if (
      message?.documentId === documentId &&
      message.type === "kiwix-storage-failed"
    ) {
      failed = true;
      closed = true;
      pendingOperations = [];
      pendingBody = 2;
      pendingReserved = 0;
      flushScheduled = false;
      for (let index = pendingRestores.length - 1; index >= 0; index--) {
        try {
          pendingRestores[index]();
        } catch {
          // The shell already rejected this producer.
        }
      }
      pendingRestores = [];
      loadLocal();
      refreshCookies();
    } else if (
      message?.type === "kiwix-storage-drain" &&
      typeof message.id === "string" &&
      message.id.length <= 64
    ) {
      if (message.documentId !== documentId) return;
      if (typeof message.freeze === "string" && message.freeze.length <= 64)
        updateFreeze(message.freeze, true);
      // Every earlier mutation from this document precedes its own drain ack.
      flushPending();
      parent.postMessage(
        { type: "kiwix-storage-drained", id: message.id, documentId },
        parentOrigin,
      );
    } else if (
      message?.type === "kiwix-storage-resume" &&
      typeof message.freeze === "string"
    ) {
      updateFreeze(message.freeze, false);
    }
  });
  // On same-origin navigation this new document runs only after the outgoing
  // pagehide dispatch and its microtasks finish. Root uses ready as a successor
  // boundary, never the outgoing hiding packet's advisory sequence.
  if (!upstream) ready();
  // This is advisory, not the final sequence: archive pagehide handlers and
  // their microtasks must remain writable. Root settles old hiding streams on
  // same-origin successor ready plus a fence, revisiting late hiding packets.
  // Explicit retirement instead uses a server-owned same-origin challenge ack.
  addEventListener(
    "pagehide",
    () => {
      if (upstream) return;
      // The advisory packet must not overtake mutations admitted in this task.
      flushPending();
      parent.postMessage(
        {
          type: "kiwix-storage-hiding",
          documentId,
          sequence,
          privacyGeneration,
        },
        parentOrigin,
      );
    },
    { capture: true },
  );
  addEventListener("pageshow", (event) => {
    if (upstream) return;
    if (event.persisted && !failed) {
      // Settle any admitted-but-undelivered mutations before the snapshot.
      flushPending();
      closed = false;
      // Keep the credit this document already had. A synchronous pageshow
      // write must succeed; the asynchronous snapshot reconciles the exact
      // value and restores acknowledgements missed while frozen.
      snapshotBoundary = sequence;
      awaitingCredit = true;
      // The frozen document missed every storage event of its successor.
      loadLocal();
      refreshCookies();
      ready();
    }
  });
  // A same-origin document that never ran this bridge writes the native mirror
  // directly. Recompute once when that happens instead of on every mutation.
  addEventListener("storage", (event) => {
    if (event.key === null || event.key.startsWith(localPrefix)) {
      if (upstream) keyList = null;
      else loadLocal();
    }
    if (event.key === null || event.key === cookieKey) refreshCookies();
  });
  // Only user data is exposed. clear() leaves the private bootstrap, cookie,
  // and freeze metadata intact across same-origin document navigation.
  class ArchiveStorage {
    get length() {
      return localKeys().length;
    }
    key(index) {
      return localKeys()[Number(index)] ?? null;
    }
    getItem(key) {
      return nativeGet(`${localPrefix}${String(key)}`);
    }
    setItem(key, value) {
      writable();
      key = String(key);
      value = String(value);
      if (!bounded(key, 8192) || !bounded(value, 1024 * 1024))
        throw quotaError("Archive storage key or value too large");
      changed([{ kind: "local-set", key, value }]);
    }
    removeItem(key) {
      writable();
      key = String(key);
      if (!bounded(key, 8192))
        throw quotaError("Archive storage key too large");
      changed([{ kind: "local-remove", key }]);
    }
    clear() {
      writable();
      changed([{ kind: "local-clear" }]);
    }
  }
  const target = new ArchiveStorage();
  const storage = new Proxy(target, {
    get(object, key) {
      if (key in object || typeof key === "symbol") {
        const value = Reflect.get(object, key, object);
        return typeof value === "function" ? value.bind(object) : value;
      }
      return nativeGet(`${localPrefix}${key}`) ?? undefined;
    },
    set(object, key, value) {
      if (typeof key === "symbol" || key in object) return false;
      object.setItem(key, value);
      return true;
    },
    deleteProperty(object, key) {
      object.removeItem(key);
      return true;
    },
    ownKeys() {
      return [...localKeys()];
    },
    getOwnPropertyDescriptor(object, key) {
      const value =
        typeof key === "string" ? nativeGet(`${localPrefix}${key}`) : null;
      return value !== null
        ? {
            configurable: true,
            enumerable: true,
            writable: true,
            value,
          }
        : Reflect.getOwnPropertyDescriptor(object, key);
    },
    has(object, key) {
      return (
        key in object ||
        (typeof key === "string" && nativeGet(`${localPrefix}${key}`) !== null)
      );
    },
  });
  Object.defineProperty(window, "localStorage", {
    configurable: false,
    value: storage,
  });
  function pathMatches(path) {
    return (
      location.pathname === path ||
      location.pathname.startsWith(path.endsWith("/") ? path : `${path}/`)
    );
  }
  function visibleCookies() {
    cookies = readCookies();
    cookiePayload = -1;
    const result = [];
    for (const [key, cookie] of cookies) {
      if (cookie.expires !== null && cookie.expires <= Date.now()) {
        cookies.delete(key);
        continue;
      }
      if (pathMatches(cookie.path)) result.push(cookie);
    }
    return result.sort((a, b) => b.path.length - a.path.length);
  }
  function setCookie(input) {
    writable();
    const text = String(input);
    refreshCookies();
    const pieces = text.split(";");
    const first = pieces.shift() || "";
    const equal = first.indexOf("=");
    if (equal <= 0) return;
    const name = first.slice(0, equal).trim();
    const value = first.slice(equal + 1).trim();
    if (
      /[^\x21-\x7e]|[()<>@,;:\\"/\[\]?={}\s]/.test(name) ||
      /[\r\n\0]/.test(value)
    )
      return;
    if (
      !bounded(name, 4096) ||
      !bounded(value, 4096) ||
      bytes(name) + bytes(value) > 4096
    )
      throw quotaError("Archive cookie name or value too large");
    let path =
      location.pathname.slice(0, location.pathname.lastIndexOf("/") + 1) || "/";
    let expires = null;
    let maxAge;
    for (const raw of pieces) {
      const [key, ...rest] = raw.trim().split("=");
      const attribute = rest.join("=").trim();
      if (
        key.toLowerCase() === "domain" &&
        attribute.replace(/^\./, "").toLowerCase() !==
          location.hostname.toLowerCase()
      )
        return;
      if (key.toLowerCase() === "path" && attribute.startsWith("/"))
        path = attribute;
      if (key.toLowerCase() === "expires") {
        const date = Date.parse(attribute);
        if (Number.isFinite(date)) expires = date;
      }
      if (key.toLowerCase() === "max-age" && /^-?\d+$/.test(attribute))
        maxAge = Number(attribute);
    }
    if (maxAge !== undefined)
      expires = Date.now() + Math.min(maxAge, 34560000) * 1000;
    const key = `${path}\n${name}`;
    if (!bounded(path, 8192) || !bounded(key, 8192))
      throw quotaError("Archive cookie path too large");
    const nextCookies = new Map(cookies);
    const operations = [];
    const expired = expires !== null && expires <= Date.now();
    if (expired) nextCookies.delete(key);
    else nextCookies.set(key, { name, value, path, expires });
    if (nextCookies.size > 200) {
      const evicted = nextCookies.keys().next().value;
      nextCookies.delete(evicted);
      operations.push({ kind: "cookie-remove", key: evicted });
    }
    if (expires === null || expired)
      operations.push({ kind: "cookie-remove", key });
    else
      operations.push({
        kind: "cookie-set",
        key,
        cookie: { name, value, path, expires },
      });
    changed(operations, nextCookies);
    window.dispatchEvent(new Event("kiwix-cookie-change"));
  }
  Object.defineProperty(document, "cookie", {
    configurable: false,
    get: () =>
      visibleCookies()
        .map((c) => `${c.name}=${c.value}`)
        .join("; "),
    set: setCookie,
  });
  class ArchiveCookieStore extends EventTarget {
    async get(options) {
      const name = typeof options === "string" ? options : options?.name;
      return (await this.getAll(name ? { name } : {}))[0] ?? null;
    }
    async getAll(options = {}) {
      const name = typeof options === "string" ? options : options.name;
      return visibleCookies()
        .filter((c) => !name || c.name === name)
        .map((c) => ({
          ...c,
          domain: null,
          secure: true,
          sameSite: "strict",
          partitioned: false,
        }));
    }
    async set(name, value) {
      const c = typeof name === "object" ? name : { name, value };
      if (!c || typeof c.name !== "string")
        throw new TypeError("Cookie name is required");
      setCookie(
        `${c.name}=${c.value ?? ""}; Path=${c.path || "/"}${c.expires ? `; Expires=${new Date(c.expires).toUTCString()}` : ""}`,
      );
    }
    async delete(options) {
      const c = typeof options === "string" ? { name: options } : options;
      setCookie(`${c.name}=; Path=${c.path || "/"}; Max-Age=0`);
    }
  }
  const cookieStore = new ArchiveCookieStore();
  window.addEventListener("kiwix-cookie-change", () =>
    cookieStore.dispatchEvent(new Event("change")),
  );
  Object.defineProperty(window, "cookieStore", {
    configurable: false,
    value: cookieStore,
  });
})();
