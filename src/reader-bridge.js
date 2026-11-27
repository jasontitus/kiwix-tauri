/* Archive-side bridge. No Tauri IPC or arbitrary parent command forwarding. */
(() => {
  "use strict";
  if (window.parent === window || window.__kiwixReaderBridge) return;
  Object.defineProperty(window, "__kiwixReaderBridge", { value: true });
  // Replay engines replace reflected URL getters with original web URLs.
  // Shell actions need the browser's actual offline target, including <base>.
  const nativeAnchorHref = Object.getOwnPropertyDescriptor(HTMLAnchorElement.prototype, "href").get;
  const nativeAreaHref = Object.getOwnPropertyDescriptor(HTMLAreaElement.prototype, "href").get;
  function linkHref(link) {
    if (link.namespaceURI !== "http://www.w3.org/1999/xhtml") return link.href;
    return (link.localName === "area" ? nativeAreaHref : nativeAnchorHref).call(link);
  }
  let archiveChild = false;
  try { archiveChild = window.parent.location.origin === location.origin; } catch (_) { /* Shell is cross-origin. */ }
  const descendantChannel = "kiwix-reader-media";
  const mediaChildren = new Map();
  let mediaParentSession = "";
  let mediaReady = false;
  let mediaRetired = false;
  const checkpointTracked = new WeakSet();
  let reportedCheckpoint = "";
  let mediaRestore = null;
  let completedMediaRestore = null;
  function positionKey(position) {
    return JSON.stringify([position.frames, position.document, position.source, position.index]);
  }
  function validMediaPositions(positions) {
    return Array.isArray(positions) && positions.length <= 16 && positions.every((position) =>
      position && typeof position.document === "string" && position.document.startsWith("/") && position.document.length <= 8192 &&
      typeof position.source === "string" && position.source.startsWith("/") && position.source.length <= 8192 &&
      Array.isArray(position.frames) && position.frames.length <= 32 &&
      position.frames.every((index) => Number.isSafeInteger(index) && index >= 0 && index < 64) &&
      !position.document.startsWith("/.kiwix/") && !position.source.startsWith("/.kiwix/") &&
      Number.isSafeInteger(position.index) && position.index >= 0 && position.index < 16 &&
      Number.isFinite(position.time) && position.time >= 0 && Number.isFinite(position.duration) &&
      position.duration > 0 && position.duration <= 100000000 && position.time <= position.duration);
  }
  function localMediaPath(value) {
    if (typeof value !== "string" || !value) return null;
    try {
      const url = new URL(value, location.href);
      return url.origin === location.origin && !url.username && !url.password &&
        !url.pathname.startsWith("/.kiwix/") && ["http:", "https:"].includes(url.protocol) ? url.pathname + url.search : null;
    } catch (_) { return null; }
  }
  function checkpointVideos() {
    let index = 0;
    const videos = [];
    const path = localMediaPath(location.href);
    if (!path) return videos;
    for (const video of document.querySelectorAll("video")) {
      const slot = index++;
      const source = localMediaPath(video.currentSrc || video.src);
      if (source && slot < 16) videos.push({ video, frames: [], document: path, source, index: slot });
    }
    return videos;
  }
  function reportMediaCheckpoint() {
    if (!mediaReady || mediaRetired) return;
    const positions = [];
    for (const entry of checkpointVideos()) {
      const { video, ...identity } = entry;
      if (video.currentTime > 0) checkpointTracked.add(video);
      const position = { ...identity, time: video.currentTime, duration: video.duration };
      // Initial zero in a replacement document cannot overwrite the data that
      // is about to restore it; a subsequent explicit seek to zero still can.
      if (checkpointTracked.has(video) && validMediaPositions([position])) positions.push(position);
      if (positions.length === 16) break;
    }
    for (const [frame, entry] of mediaChildren) {
      if (!frame.isConnected || frame.contentDocument !== entry.document) continue;
      const occurrence = mediaFrameOccurrence(frame);
      if (occurrence < 0 || occurrence >= 64) continue;
      for (const position of entry.positions || []) {
        if (positions.length === 16) break;
        if (position.frames.length < 32)
          positions.push({ ...position, frames: [occurrence, ...position.frames] });
      }
    }
    if (!positions.length) return;
    const signature = JSON.stringify(positions);
    if (signature === reportedCheckpoint) return;
    reportedCheckpoint = signature;
    if (archiveChild) mediaPost(window.parent, "checkpoint", mediaParentSession, { url: location.href, positions });
    else post("media-checkpoint", { url: location.href, positions });
  }
  function cancelMediaRestore() {
    const operation = mediaRestore;
    if (!operation) return;
    mediaRestore = null;
    operation.observer.disconnect();
    for (const type of ["loadedmetadata", "durationchange", "canplay"])
      document.removeEventListener(type, operation.apply, true);
    for (const [frame, entry] of mediaChildren)
      mediaPost(frame.contentWindow, "restore-cancel", entry.session, { requestId: operation.id });
  }
  function mediaFrameOccurrence(frame) {
    return Array.prototype.indexOf.call(document.querySelectorAll("iframe,frame"), frame);
  }
  function restoreFramesCurrent(operation) {
    const frames = document.querySelectorAll("iframe,frame");
    for (const [frame, bound] of operation.frames)
      if (!frame.isConnected || frames[bound.index] !== frame || frame.contentDocument !== bound.document) return false;
    return true;
  }
  function forwardMediaRestore(frame, entry) {
    const operation = mediaRestore;
    if (!operation || !frame.isConnected || frame.contentDocument !== entry.document) return;
    if (!restoreFramesCurrent(operation)) { cancelMediaRestore(); return; }
    const index = mediaFrameOccurrence(frame);
    if (index < 0 || index >= 64) return;
    const positions = [...operation.remaining.values()].filter((position) => position.frames[0] === index)
      .map((position) => ({ ...position, frames: position.frames.slice(1) }));
    if (!positions.length) return;
    operation.frames.set(frame, { index, document: entry.document });
    mediaPost(frame.contentWindow, "restore", entry.session, { requestId: operation.id, positions });
  }
  function restoreMediaPositions(requestId, positions) {
    if (typeof requestId !== "string" || !/^[0-9a-f-]{36}$/.test(requestId) ||
        !validMediaPositions(positions) || mediaRetired) return;
    const authority = archiveChild ? mediaParentSession : sessionId;
    if (!authority) return;
    if (completedMediaRestore?.id === requestId && completedMediaRestore.authority === authority) {
      if (archiveChild) mediaPost(window.parent, "restore-consumed", mediaParentSession,
        { requestId, keys: completedMediaRestore.keys });
      return;
    }
    if (mediaRestore?.id === requestId && mediaRestore.authority === authority) return;
    cancelMediaRestore();
    const operation = { id: requestId, authority, url: location.href,
      remaining: new Map(positions.map((position) => [positionKey(position), position])),
      consumed: [], targets: new Map(), frames: new Map(), observer: null, apply: null };
    mediaRestore = operation;
    operation.apply = (mutations) => {
      if (mediaRestore !== operation || mediaRetired || location.href !== operation.url ||
          (archiveChild ? mediaParentSession : sessionId) !== authority) return;
      if (!restoreFramesCurrent(operation)) { cancelMediaRestore(); return; }
      const consumed = [];
      for (const [key, bound] of operation.targets) {
        const sourceChanged = Array.isArray(mutations) && mutations.some((mutation) =>
          mutation.type === "attributes" && mutation.attributeName === "src" &&
          (mutation.target === bound.video || mutation.target.parentElement === bound.video));
        if ((!bound.video.isConnected || sourceChanged ||
            localMediaPath(bound.video.currentSrc || bound.video.src) !== bound.source ||
            (bound.video.src && localMediaPath(bound.video.src) !== bound.source)) &&
            operation.remaining.delete(key)) {
          operation.consumed.push(key);
          consumed.push(key);
        }
      }
      for (const entry of checkpointVideos()) {
        const { video, ...identity } = entry;
        const key = positionKey(identity);
        const position = operation.remaining.get(key);
        if (!position) continue;
        const bound = operation.targets.get(key);
        let discard = bound && bound.video !== video;
        if (!bound) operation.targets.set(key, { video, source: entry.source });
        if (!discard && (!Number.isFinite(video.duration) || video.duration <= 0)) continue;
        discard ||= Math.abs(video.duration - position.duration) > 0.25;
        if (!discard) {
          try {
            video.pause();
            if (video.paused) {
              video.currentTime = Math.min(position.time, video.duration);
              checkpointTracked.add(video);
            }
          } catch (_) { continue; /* Wait for real metadata/readiness, never a timer. */ }
        }
        operation.remaining.delete(key);
        operation.consumed.push(key);
        consumed.push(key);
      }
      if (archiveChild && consumed.length)
        mediaPost(window.parent, "restore-consumed", mediaParentSession, { requestId, keys: consumed });
      if (!operation.remaining.size) {
        completedMediaRestore = { id: requestId, authority, keys: operation.consumed };
        cancelMediaRestore();
      }
      reportMediaCheckpoint();
    };
    operation.observer = new MutationObserver(operation.apply);
    operation.observer.observe(document.documentElement,
      { childList: true, subtree: true, attributes: true, attributeFilter: ["src"] });
    for (const type of ["loadedmetadata", "durationchange", "canplay"])
      document.addEventListener(type, operation.apply, true);
    operation.apply();
    if (mediaRestore === operation) for (const [frame, entry] of mediaChildren) forwardMediaRestore(frame, entry);
  }
  function mediaToken() { return crypto.randomUUID(); }
  function mediaPost(target, type, session, extra = {}) {
    target.postMessage({ channel: descendantChannel, version: 1, type, session, ...extra }, location.origin);
  }
  function childFrame(source) {
    for (const frame of document.querySelectorAll("iframe,frame")) {
      try {
        if (frame.contentWindow === source && frame.contentDocument &&
            frame.contentWindow.location.origin === location.origin) return frame;
      } catch (_) { /* Cross-origin players have no archive media authority. */ }
    }
    return null;
  }
  function retireMediaChildren() {
    for (const [frame, entry] of mediaChildren)
      mediaPost(frame.contentWindow, "retire", entry.session);
    mediaChildren.clear();
  }
  function pruneMediaChildren() {
    let changed = false;
    for (const [frame, entry] of mediaChildren) {
      if (!frame.isConnected || frame.contentDocument !== entry.document) {
        cancelMediaRestore();
        mediaChildren.delete(frame);
        changed = true;
      }
    }
    if (changed) reportMediaState();
  }
  function initializeMediaChild(frame) {
    const entry = { document: frame.contentDocument, session: mediaToken(),
      audible: false, pictureInPicture: false };
    entry.positions = [];
    mediaChildren.set(frame, entry);
    mediaPost(frame.contentWindow, "init", entry.session);
    forwardMediaRestore(frame, entry);
    reportMediaState();
  }
  window.addEventListener("message", (event) => {
    const message = event.data;
    if (mediaRetired || event.origin !== location.origin ||
        message?.channel !== descendantChannel || message.version !== 1) return;
    if (archiveChild && event.source === window.parent) {
      if (message.type === "probe") {
        mediaPost(window.parent, "hello", "");
      } else if (message.type === "init" && typeof message.session === "string" &&
          message.session.length > 0 && message.session.length <= 128) {
        if (mediaParentSession && mediaParentSession !== message.session) {
          pauseAllMedia();
          retireMediaChildren();
          for (const frame of document.querySelectorAll("iframe,frame"))
            if (childFrame(frame.contentWindow)) mediaPost(frame.contentWindow, "probe", "");
        }
        mediaParentSession = message.session;
        mediaReady = true;
        reportMediaState(true);
        reportMediaCheckpoint();
      } else if (mediaParentSession && message.session === mediaParentSession) {
        if (message.type === "pause") pauseAllMedia();
        else if (message.type === "pause-video") pauseVideosUnlessPictureInPicture();
        else if (message.type === "restore") restoreMediaPositions(message.requestId, message.positions);
        else if (message.type === "restore-cancel" && mediaRestore?.id === message.requestId) cancelMediaRestore();
        else if (message.type === "retire") {
          pauseAllMedia();
          retireMediaChildren();
          mediaParentSession = "";
          mediaReady = false;
        }
      }
      return;
    }
    const frame = childFrame(event.source);
    if (!frame) return;
    if (message.type === "hello") {
      const entry = mediaChildren.get(frame);
      if (entry && entry.document !== frame.contentDocument) cancelMediaRestore();
      if (!entry || entry.document !== frame.contentDocument) initializeMediaChild(frame);
      else mediaPost(event.source, "init", entry.session);
      return;
    }
    const entry = mediaChildren.get(frame);
    if (!entry || entry.document !== frame.contentDocument || message.session !== entry.session) return;
    if (message.type === "checkpoint") {
      if (message.url !== frame.contentWindow.location.href || !validMediaPositions(message.positions)) return;
      entry.positions = message.positions;
      reportMediaCheckpoint();
      return;
    }
    if (message.type === "restore-consumed" && mediaRestore?.id === message.requestId &&
        Array.isArray(message.keys) && message.keys.length <= 16) {
      const operation = mediaRestore;
      const bound = operation.frames.get(frame);
      if (!bound || !restoreFramesCurrent(operation)) { cancelMediaRestore(); return; }
      const keys = [];
      for (const key of message.keys) {
        if (typeof key !== "string" || key.length > 20000) continue;
        try {
          const value = JSON.parse(key);
          if (!Array.isArray(value) || value.length !== 4 || !Array.isArray(value[0])) continue;
          const position = { frames: [bound.index, ...value[0]], document: value[1],
            source: value[2], index: value[3], time: 0, duration: 1 };
          const routed = positionKey(position);
          if (validMediaPositions([position]) && operation.remaining.has(routed)) keys.push(routed);
        } catch (_) { /* Malformed acknowledgments have no restore ownership. */ }
      }
      for (const key of keys) { operation.remaining.delete(key); operation.consumed.push(key); }
      if (archiveChild && keys.length)
        mediaPost(window.parent, "restore-consumed", mediaParentSession, { requestId: operation.id, keys });
      if (!operation.remaining.size) {
        completedMediaRestore = { id: operation.id, authority: operation.authority, keys: operation.consumed };
        cancelMediaRestore();
      }
      return;
    }
    if (message.type === "retired") { cancelMediaRestore(); mediaChildren.delete(frame); }
    else if (message.type === "state" && typeof message.audible === "boolean" &&
        typeof message.pictureInPicture === "boolean") {
      entry.audible = message.audible;
      entry.pictureInPicture = message.pictureInPicture;
    } else return;
    reportMediaState();
  });
  document.addEventListener("load", (event) => {
    const frame = event.target;
    if (!["IFRAME", "FRAME"].includes(frame?.tagName)) return;
    const entry = mediaChildren.get(frame);
    if (entry && entry.document !== frame.contentDocument) {
      cancelMediaRestore();
      mediaChildren.delete(frame);
      reportMediaState();
    }
    if (childFrame(frame.contentWindow)) mediaPost(frame.contentWindow, "probe", "");
  }, true);
  // Archive media policy. iOS needs `playsinline` on every video in addition to
  // the webview's inline-playback capability; without it the native player
  // leaves the article. The observer covers videos the archive inserts later.
  // Audio is never paused here: background audio belongs to the platform, while
  // a video that is not the active PiP element must stop when the page hides.
  function markVideoInline(video) {
    if (!video.hasAttribute("playsinline")) video.setAttribute("playsinline", "");
    if (!video.hasAttribute("webkit-playsinline"))
      video.setAttribute("webkit-playsinline", "");
  }
  // Video format capability. Phone WebKit ships no VP8/VP9 decoder, so archives
  // that publish WebM fall back to a JavaScript decoder that stutters on a
  // phone. The same source plays natively on an iPad or the desktop app, so the
  // verdict is per element and never per archive. The notice copy arrives from
  // the shell (localized through the English source) in `reader-init`.
  const videoSourceTypes = {
    mp4: "video/mp4",
    m4v: "video/mp4",
    mov: "video/quicktime",
    webm: "video/webm",
    ogv: "video/ogg",
    ogg: "video/ogg",
  };
  const mediaNotice = { text: "", dismiss: "" };
  const noticedVideos = new WeakSet();
  const dismissedVideos = new WeakSet();
  let readerInteracted = false;
  for (const type of ["pointerdown", "keydown", "touchstart"])
    document.addEventListener(
      type,
      () => {
        readerInteracted = true;
      },
      { capture: true, passive: true },
    );
  function videoSourceType(source) {
    const declared = source.getAttribute?.("type");
    if (declared) return declared;
    const raw =
      source.getAttribute?.("src") || source.currentSrc || source.src || "";
    const match = /\.([a-z0-9]+)(?:[?#]|$)/i.exec(raw);
    return match ? videoSourceTypes[match[1].toLowerCase()] || "" : "";
  }
  function videoPlaysSource(video, source) {
    const type = videoSourceType(source);
    // A blob: or extensionless source carries no declared type. Stay neutral and
    // let the element's own `error` report an unsupported decoder.
    if (!type) return null;
    try {
      return video.canPlayType(type) !== "";
    } catch (_) {
      return null;
    }
  }
  function videoIsUnsupported(video) {
    if (!video.isConnected) return false;
    const error = video.error;
    if (error && error.code === 4) return true;
    const sources = video.querySelectorAll
      ? Array.from(video.querySelectorAll("source"))
      : [];
    if (sources.length) {
      const verdicts = sources.map((source) =>
        videoPlaysSource(video, source),
      );
      return verdicts.every((playable) => playable === false);
    }
    return videoPlaysSource(video, video) === false;
  }
  // The nearest player chrome around a video. video.js and ogv.js wrap the
  // element in a positioned div that mutates class and style on every progress
  // tick; the theme pass must never walk that subtree.
  function markVideoSurface(video) {
    let anchor = video;
    for (let depth = 0; depth < 3; depth++) {
      const parent = anchor.parentElement;
      if (!parent || parent === document.body) break;
      const className =
        typeof parent.className === "string" ? parent.className : "";
      const player =
        /(?:^|\s)(?:video-js|ogv-player|ogvjs-player|video-player|player)(?:\s|$)/i.test(
          className,
        ) ||
        Array.prototype.some.call(
          parent.children || [],
          (child) => child.tagName === "CANVAS",
        );
      if (!player) break;
      anchor = parent;
    }
    if (anchor !== video) anchor.setAttribute("data-kiwix-media-surface", "");
    return anchor;
  }
  // A software decoder competes with the reader for the same phone, so a video
  // whose format this WebKit cannot decode is reported, not rewritten: the
  // archive's own player owns `preload`, `autoplay` and play/pause. Mutating
  // those attributes here made a player's own `play()` stop again a fraction of
  // a second in, which reads as "it plays a few frames and stops".
  function applySoftwareVideoSafeguards(video) {
    if (!readerInteracted && !video.paused && videoIsUnsupported(video)) {
      try {
        video.pause();
      } catch (_) {
        /* A detached or unplayable element is already stopped. */
      }
    }
  }
  function showVideoNotice(video) {
    if (
      !mediaNotice.text ||
      !mediaNotice.dismiss ||
      noticedVideos.has(video) ||
      dismissedVideos.has(video) ||
      !video.isConnected
    )
      return;
    const surface = markVideoSurface(video);
    // A `<p>` cannot hold a block notice; put it before the paragraph instead.
    const anchor = surface.parentElement?.tagName === "P" ? surface.parentElement : surface;
    const host = anchor.parentElement;
    if (!host) return;
    noticedVideos.add(video);
    const notice = document.createElement("div");
    notice.setAttribute("data-kiwix-notice", "");
    notice.setAttribute("role", "status");
    // Quiet, in flow, above the player: never an overlay and never a red band.
    notice.style.cssText =
      "display:flex;gap:8px;align-items:flex-start;margin:8px 0;padding:8px 10px;" +
      "border:1px solid rgba(128,128,128,.45);border-radius:8px;" +
      "background:rgba(128,128,128,.14);color:inherit;font-size:13px;line-height:1.4;" +
      "max-width:100%;box-sizing:border-box;";
    const text = document.createElement("span");
    text.style.cssText = "flex:1 1 auto;min-width:0;";
    text.textContent = mediaNotice.text;
    const dismiss = document.createElement("button");
    dismiss.setAttribute("type", "button");
    dismiss.setAttribute("aria-label", mediaNotice.dismiss);
    dismiss.style.cssText =
      "flex:none;border:0;background:transparent;color:inherit;font:inherit;" +
      "padding:2px 4px;text-decoration:underline;cursor:pointer;";
    dismiss.textContent = mediaNotice.dismiss;
    dismiss.addEventListener("click", () => {
      dismissedVideos.add(video);
      notice.remove();
    });
    notice.append(text, dismiss);
    host.insertBefore(notice, anchor);
  }
  function evaluateVideo(video) {
    if (!video || video.tagName !== "VIDEO" || !video.isConnected) return;
    if (!videoIsUnsupported(video)) return;
    applySoftwareVideoSafeguards(video);
    showVideoNotice(video);
  }
  function scheduleVideoEvaluation(video) {
    if (!video || video.tagName !== "VIDEO") return;
    setTimeout(() => evaluateVideo(video), 0);
  }
  function prepareVideo(video) {
    markVideoInline(video);
    markVideoSurface(video);
    scheduleVideoEvaluation(video);
  }
  function prepareArchiveMedia(root) {
    if (!root) return;
    if (root.nodeType === Node.ELEMENT_NODE && root.tagName === "VIDEO")
      prepareVideo(root);
    if (root.querySelectorAll)
      for (const video of root.querySelectorAll("video")) prepareVideo(video);
  }
  function acceptMediaNotice(value) {
    if (!value || typeof value !== "object") return;
    if (typeof value.text !== "string" || !value.text || value.text.length > 400)
      return;
    if (
      typeof value.dismiss !== "string" ||
      !value.dismiss ||
      value.dismiss.length > 40
    )
      return;
    mediaNotice.text = value.text;
    mediaNotice.dismiss = value.dismiss;
    for (const video of document.querySelectorAll("video")) evaluateVideo(video);
  }
  prepareArchiveMedia(document.documentElement);
  const archiveMediaObserver = new MutationObserver((mutations) => {
    for (const mutation of mutations)
      for (const node of mutation.addedNodes) {
        if (node.nodeType !== Node.ELEMENT_NODE) continue;
        prepareArchiveMedia(node);
        // Sources are appended after their video element, so the video is
        // evaluated again once the browser can see what it actually plays.
        if (node.tagName === "SOURCE" && node.parentElement?.tagName === "VIDEO")
          scheduleVideoEvaluation(node.parentElement);
      }
    pruneMediaChildren();
  });
  document.addEventListener(
    "error",
    (event) => {
      const target = event.target;
      if (target?.tagName === "VIDEO") scheduleVideoEvaluation(target);
      else if (target?.tagName === "SOURCE" && target.parentElement?.tagName === "VIDEO")
        scheduleVideoEvaluation(target.parentElement);
    },
    true,
  );
  document.addEventListener(
    "play",
    (event) => {
      const video = event.target;
      if (video?.tagName !== "VIDEO" || readerInteracted) return;
      // A software-decoded video must not start before the person asks for it.
      if (videoIsUnsupported(video)) applySoftwareVideoSafeguards(video);
    },
    true,
  );
  if (document.documentElement)
    archiveMediaObserver.observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
  function videoIsPictureInPicture(video) {
    return video === document.pictureInPictureElement ||
      video.webkitPresentationMode === "picture-in-picture";
  }
  function pauseVideosUnlessPictureInPicture() {
    reportMediaCheckpoint();
    for (const video of document.querySelectorAll("video")) {
      if (videoIsPictureInPicture(video)) continue;
      try {
        video.pause();
      } catch (_) {
        /* A detached or unplayable element is already stopped. */
      }
    }
    for (const [frame, entry] of mediaChildren)
      mediaPost(frame.contentWindow, "pause-video", entry.session);
  }
  function pauseAllMedia() {
    reportMediaCheckpoint();
    cancelMediaRestore();
    if (document.pictureInPictureElement && document.exitPictureInPicture) {
      try {
        void document.exitPictureInPicture().catch(() => {});
      } catch (_) {
        /* PiP is unavailable or already leaving. */
      }
    }
    for (const media of document.querySelectorAll("video,audio")) {
      if (media.tagName === "VIDEO" && media.webkitPresentationMode === "picture-in-picture" &&
          typeof media.webkitSetPresentationMode === "function") {
        try { media.webkitSetPresentationMode("inline"); } catch (_) { /* Still pause below. */ }
      }
      try {
        media.pause();
      } catch (_) {
        /* A detached or unplayable element is already stopped. */
      }
    }
    for (const [frame, entry] of mediaChildren) {
      entry.audible = false;
      entry.pictureInPicture = false;
      mediaPost(frame.contentWindow, "pause", entry.session);
    }
    reportMediaState(true);
  }
  // Report the two user-started surfaces the shell may keep alive across a
  // mobile suspension. Audible media and an active PiP presentation are the
  // only exemptions; a paused or muted element is ordinary reading.
  let reportedMedia = { audible: false, pictureInPicture: false };
  function currentMediaState() {
    let audible = false;
    let pictureInPicture = document.pictureInPictureElement != null;
    for (const media of document.querySelectorAll("video,audio")) {
      if (media.tagName === "VIDEO") pictureInPicture ||= videoIsPictureInPicture(media);
      if (
        !media.paused &&
        !media.muted &&
        media.volume > 0 &&
        (media.currentTime > 0 || media.readyState >= 2)
      ) {
        audible = true;
      }
    }
    for (const [frame, entry] of mediaChildren) {
      if (!frame.isConnected || frame.contentDocument !== entry.document) continue;
      audible ||= entry.audible;
      pictureInPicture ||= entry.pictureInPicture;
    }
    return { audible, pictureInPicture };
  }
  function reportMediaState(force = false) {
    if (!mediaReady) return;
    const state = currentMediaState();
    if (
      !force &&
      state.audible === reportedMedia.audible &&
      state.pictureInPicture === reportedMedia.pictureInPicture
    )
      return;
    reportedMedia = state;
    if (archiveChild) mediaPost(window.parent, "state", mediaParentSession, state);
    else post("media-state", state);
  }
  for (const type of [
    "play",
    "playing",
    "pause",
    "ended",
    "volumechange",
    "emptied",
    "enterpictureinpicture",
    "leavepictureinpicture",
    "webkitpresentationmodechanged",
  ])
    document.addEventListener(
      type,
      () => reportMediaState(),
      true,
    );
  for (const type of ["timeupdate", "seeked", "loadedmetadata", "durationchange", "pause"])
    document.addEventListener(type, reportMediaCheckpoint, true);
  document.addEventListener("visibilitychange", () => {
    reportMediaCheckpoint();
    if (document.hidden) {
      pauseVideosUnlessPictureInPicture();
    }
    reportMediaState();
  });
  window.addEventListener("pagehide", () => {
    // Real navigation/teardown, never an app-background signal: backgrounding
    // fires visibilitychange, where audio intentionally continues. Teardown
    // must stop every surface, including PiP.
    archiveMediaObserver.disconnect();
    pauseAllMedia();
    retireMediaChildren();
    if (archiveChild && mediaParentSession)
      mediaPost(window.parent, "retired", mediaParentSession);
    mediaRetired = true;
  });
  if (archiveChild) {
    // Descendants share archive storage, not shell navigation or sensor authority.
    // Native permission policy remains the enforcement boundary for uninjected documents.
    if (navigator.mediaDevices) Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: false, writable: false,
      value: () => Promise.reject(new DOMException("Sensors are unavailable in embedded archive frames", "NotAllowedError")),
    });
    const deniedPosition = (_success, error) => {
      if (typeof error === "function") queueMicrotask(() => error({
        code: 1, message: "Location is unavailable in embedded archive frames",
        PERMISSION_DENIED: 1, POSITION_UNAVAILABLE: 2, TIMEOUT: 3,
      }));
      return 0;
    };
    const childGeolocation = Object.freeze({
      getCurrentPosition: deniedPosition, watchPosition: deniedPosition, clearWatch() {},
    });
    try {
      Object.defineProperty(navigator, "geolocation", {
        configurable: false, writable: false, value: childGeolocation,
      });
    } catch (_) {
      if (navigator.geolocation) Object.defineProperties(navigator.geolocation, {
        getCurrentPosition: { configurable: false, writable: false, value: deniedPosition },
        watchPosition: { configurable: false, writable: false, value: deniedPosition },
        clearWatch: { configurable: false, writable: false, value: childGeolocation.clearWatch },
      });
    }
    window.open = () => null;
    const containNavigation = (event) => {
      const element = event.target instanceof Element ? event.target : null;
      const link = element?.closest("a[href],area[href]");
      const form = event.type === "submit" && event.target instanceof HTMLFormElement ? event.target : null;
      if (!link && !form) return;
      const target = link?.getAttribute("target") ?? form?.getAttribute("target") ?? document.querySelector("base[target]")?.getAttribute("target") ?? "_self";
      let local = false;
      try { local = new URL(link ? linkHref(link) : form.action, location.href).origin === location.origin; } catch (_) { /* Deny malformed targets. */ }
      if (!local || !["", "_self"].includes(target)) {
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    };
    document.addEventListener("click", containNavigation, true);
    document.addEventListener("auxclick", containNavigation, true);
    document.addEventListener("submit", containNavigation, true);
    mediaPost(window.parent, "hello", "");
    return;
  }
  const script = document.currentScript;
  let parentOrigin = script?.dataset.parentOrigin || "";
  if (!parentOrigin && document.referrer) {
    try {
      parentOrigin = new URL(document.referrer).origin;
    } catch (_) {
      /* closed below */
    }
  }
  // An opaque/unknown embedding origin cannot be authenticated safely.
  if (!parentOrigin || parentOrigin === "null" || parentOrigin === "*") return;
  const channel = "kiwix-reader";
  // Bootstrap must precede archive scripts. Keep the gesture proof private:
  // only the cross-origin shell receives it, never this window.
  const readingPanToken = crypto.randomUUID();
  const postToParent = parent.postMessage.bind(parent);
  // Announce the private gesture token before any archive script can run. The
  // shell pins this first announcement per document, so a document replaced by
  // archive code later cannot bind or forge gestures with a token of its own.
  postToParent(
    { channel, version: 1, type: "reading-gesture-hello", token: readingPanToken },
    parentOrigin,
  );
  const nativeGetter = (prototype, key) => {
    const getter = Object.getOwnPropertyDescriptor(prototype, key)?.get;
    return getter ? Function.prototype.call.bind(getter) : null;
  };
  const touchEventTouches = typeof TouchEvent === "function" ? nativeGetter(TouchEvent.prototype, "touches") : null;
  const touchListLength = typeof TouchList === "function" ? nativeGetter(TouchList.prototype, "length") : null;
  const touchListItem = typeof TouchList === "function" ? Function.prototype.call.bind(TouchList.prototype.item) : null;
  const touchX = typeof Touch === "function" ? nativeGetter(Touch.prototype, "clientX") : null;
  const touchY = typeof Touch === "function" ? nativeGetter(Touch.prototype, "clientY") : null;
  const touchIdentifier = typeof Touch === "function" ? nativeGetter(Touch.prototype, "identifier") : null;
  const wheelControlKey = nativeGetter(MouseEvent.prototype, "ctrlKey");
  const wheelDeltaY = nativeGetter(WheelEvent.prototype, "deltaY");
  const preventWheelDefault = Function.prototype.call.bind(Event.prototype.preventDefault);
  const nativeScrollY = nativeGetter(window, "scrollY");
  const readingScrollY = () => nativeScrollY ? nativeScrollY(window) : window.scrollY;
  const readingTouches = (event) => touchEventTouches ? touchEventTouches(event) : event.touches;
  const readingTouchCount = (touches) => touchListLength ? touchListLength(touches) : touches.length;
  const readingTouchPoint = (touches) => touchListItem ? touchListItem(touches, 0) : touches[0];
  const readingTouchX = (point) => touchX ? touchX(point) : point.clientX;
  const readingTouchY = (point) => touchY ? touchY(point) : point.clientY;
  const readingTouchIdentifier = (point) => touchIdentifier ? touchIdentifier(point) : point.identifier;
  const scrollReadingViewport = window.scrollBy?.bind(window);
  const scheduleReadingFrame = requestAnimationFrame.bind(window);
  let readingPanSequence = 0;
  let readingControlsEnabled = false;
  let readingControlsMinimumScroll = 54;
  let readingPanHidden = false;
  let readingTouch = null;
  const pending = new Map();
  const streams = new Set();
  const watches = new Map();
  const browserRequests = new Set();
  const nativeGeolocation = navigator.geolocation;
  const nativeWatchPosition =
    nativeGeolocation?.watchPosition.bind(nativeGeolocation);
  const nativeClearWatch =
    nativeGeolocation?.clearWatch.bind(nativeGeolocation);
  const nativeGetUserMedia = navigator.mediaDevices?.getUserMedia.bind(
    navigator.mediaDevices,
  );
  let sessionId = "";
  let nextId = 0;
  let nextWatch = 0;
  let cameraEpoch = 0;
  let locationEpoch = 0;
  let scrollScheduled = false;

  // Browser Permissions-Policy deliberately denies the browser's provider on
  // CoreLocation platforms. Queries must describe the same archive-scoped
  // provider as our geolocation API, not that disabled browser fallback.
  const nativePermissionQuery = navigator.permissions?.query.bind(navigator.permissions);
  let nativeLocation = false;
  let archiveLocationDecision = "notAsked";
  let observedLocationPermission = "prompt";
  let locationPermissionState = "prompt";
  let initializePermissions;
  const permissionsInitialized = new Promise((resolve) => { initializePermissions = resolve; });
  const locationPermissionStatuses = new Set();
  class LocationPermissionStatus extends EventTarget {
    onchange = null;
    get name() { return "geolocation"; }
    get state() { return locationPermissionState; }
    constructor() {
      super();
      this.addEventListener("change", (event) => {
        if (typeof this.onchange === "function") this.onchange.call(this, event);
      });
      locationPermissionStatuses.add(new WeakRef(this));
    }
  }
  function updateLocationPermission() {
    const state = archiveLocationDecision === "denied"
      ? "denied"
      : archiveLocationDecision === "granted" ? observedLocationPermission : "prompt";
    if (state === locationPermissionState) return;
    locationPermissionState = state;
    for (const reference of locationPermissionStatuses) {
      const status = reference.deref();
      if (status) setTimeout(() => status.dispatchEvent(new Event("change")), 0);
      else locationPermissionStatuses.delete(reference);
    }
  }
  function acceptSensorStatus(status) {
    if (!status || typeof status.nativeLocation !== "boolean" ||
        !["notAsked", "granted", "denied"].includes(status.location)) return;
    nativeLocation = status.nativeLocation;
    archiveLocationDecision = status.location;
    updateLocationPermission();
  }
  const queryPermission = async (descriptor) => {
    if (descriptor?.name !== "geolocation") {
      if (nativePermissionQuery) return nativePermissionQuery(descriptor);
      throw new TypeError("This permission is not supported");
    }
    // Archive scripts run before the authenticated shell handshake. Do not let
    // an early query permanently cache the intentionally denied browser policy.
    await permissionsInitialized;
    if (!nativeLocation) {
      if (nativePermissionQuery) return nativePermissionQuery(descriptor);
      throw new TypeError("Geolocation permission queries are not supported");
    }
    return new LocationPermissionStatus();
  };
  if (navigator.permissions) {
    Object.defineProperty(navigator.permissions, "query", {
      configurable: false, writable: false, value: queryPermission,
    });
  } else {
    Object.defineProperty(navigator, "permissions", {
      configurable: false, writable: false, value: Object.freeze({ query: queryPermission }),
    });
  }

  function post(type, data = {}) {
    if (!sessionId) return;
    postToParent(
      { channel, version: 1, type, sessionId, ...data },
      parentOrigin,
    );
  }

  // Desktop photo chooser. A desktop webview only ever offers a file picker for
  // an image input (it ignores `capture`), so an archive's photo search cannot
  // use the computer's camera. When the shell enables it (a desktop with the
  // app's camera window, not restricted mode), an image file input asks the app
  // instead: the app shows "Use camera / Choose a file" outside this page and
  // returns one photo, handed to the input as if the user had picked it. The
  // bridge only detects the request and delivers the result; it never touches
  // the camera. Natives are captured here, before any archive script runs.
  const photoNatives = (() => { try {
    const getter = (prototype, key) => {
      const get = Object.getOwnPropertyDescriptor(prototype, key)?.get;
      return get ? Function.prototype.call.bind(get) : null;
    };
    const setter = (prototype, key) => {
      const set = Object.getOwnPropertyDescriptor(prototype, key)?.set;
      return set ? Function.prototype.call.bind(set) : null;
    };
    const call = (fn) => (typeof fn === "function" ? Function.prototype.call.bind(fn) : null);
    return {
      click: call(HTMLInputElement.prototype.click),
      showPicker: call(HTMLInputElement.prototype.showPicker),
      type: getter(HTMLInputElement.prototype, "type"),
      disabled: getter(HTMLInputElement.prototype, "disabled"),
      accept: getter(HTMLInputElement.prototype, "accept"),
      multiple: getter(HTMLInputElement.prototype, "multiple"),
      setFiles: setter(HTMLInputElement.prototype, "files"),
      connected: getter(Node.prototype, "isConnected"),
      hasAttribute: call(Element.prototype.hasAttribute),
      getAttribute: call(Element.prototype.getAttribute),
      composedPath: call(Event.prototype.composedPath),
      preventDefault: call(Event.prototype.preventDefault),
      stopImmediatePropagation: call(Event.prototype.stopImmediatePropagation),
      dispatch: call(EventTarget.prototype.dispatchEvent),
      activation: typeof UserActivation === "function" ? getter(UserActivation.prototype, "isActive") : null,
      userActivation: navigator.userActivation || null,
      itemsAdd: typeof DataTransferItemList === "function" ? call(DataTransferItemList.prototype.add) : null,
      transferItems: typeof DataTransfer === "function" ? getter(DataTransfer.prototype, "items") : null,
      transferFiles: typeof DataTransfer === "function" ? getter(DataTransfer.prototype, "files") : null,
      File,
      DataTransfer: typeof DataTransfer === "function" ? DataTransfer : null,
      Event,
      WeakRef: typeof WeakRef === "function" ? WeakRef : null,
      randomUUID: crypto.randomUUID.bind(crypto),
    };
  } catch (_) { return null; } })();
  let photoChooser = false;
  // Per-document secret from reader-init (hidden from archive scripts): the
  // shell accepts a photo request only with it.
  let photoSecret = "";
  let photoPending = null;
  function clearPhotoPending() {
    if (photoPending?.timer) clearTimeout(photoPending.timer);
    photoPending = null;
  }
  /** An enabled single-file input whose whole accept list is supported photos. */
  function photoInput(element) {
    const n = photoNatives;
    if (!n || !(element instanceof HTMLInputElement) || !n.type || !n.accept || !n.multiple) return false;
    if (n.type(element) !== "file" || n.disabled(element) || n.multiple(element)) return false;
    const accept = String(n.accept(element) || "").toLowerCase();
    const parts = accept.split(",").map((part) => part.trim()).filter(Boolean);
    if (!parts.length) return false;
    // A mixed list (for example `image/*,application/pdf`) or an image type the
    // shell cannot return as a photo must keep the browser's own picker.
    return parts.every((type) =>
      type === "image/*" ||
      ["image/jpeg", "image/jpg", "image/png", "image/webp", "image/gif", "image/bmp",
        "image/heic", "image/heif", "image/avif"].includes(type) ||
      [".jpg", ".jpeg", ".jfif", ".png", ".webp", ".gif", ".bmp", ".heic", ".heif", ".avif"].includes(type));
  }
  function userActive() {
    const n = photoNatives;
    return !!(n && n.activation && n.userActivation && n.activation(n.userActivation));
  }
  /** Ask the app for a photo. Returns true when the request replaces the picker. */
  function requestPhoto(input) {
    const n = photoNatives;
    if (!n || !photoChooser || !photoSecret || !sessionId || !n.WeakRef || !n.DataTransfer || !n.setFiles || !n.itemsAdd) return false;
    // One request at a time; a second activation while the chooser is open is
    // absorbed rather than opening a native picker behind it.
    if (photoPending) return true;
    const requestId = n.randomUUID();
    // The shell acknowledges at once; without that, release the input so it
    // is never left dead.
    const timer = setTimeout(() => {
      if (photoPending?.requestId === requestId && !photoPending.accepted) photoPending = null;
    }, 3000);
    photoPending = { requestId, input: new n.WeakRef(input), timer, accepted: false };
    post("photo-request", {
      secret: photoSecret,
      requestId,
      accept: String(n.accept(input) || "").slice(0, 1024),
      capture: String(n.getAttribute(input, "capture") || "").slice(0, 32),
    });
    return true;
  }
  // A real click on the input itself (or its label's activation behaviour).
  window.addEventListener("click", (event) => {
    if (!event.isTrusted || !photoChooser || !photoNatives) return;
    const path = photoNatives.composedPath ? photoNatives.composedPath(event) : [];
    const input = path[0];
    if (!photoInput(input)) return;
    if (requestPhoto(input)) {
      photoNatives.preventDefault(event);
      photoNatives.stopImmediatePropagation(event);
    }
  }, true);
  // Pages usually open the hidden input from their own button handler with
  // input.click() or input.showPicker(). Inside a genuine user activation that
  // is the same intent, so it gets the chooser too; without one, the browser's
  // own rule applies (no picker), unchanged.
  for (const name of photoNatives ? ["click", "showPicker"] : []) {
    const original = photoNatives[name];
    if (!original) continue;
    Object.defineProperty(HTMLInputElement.prototype, name, {
      configurable: true,
      writable: true,
      value: function () {
        if (photoChooser && photoInput(this) && userActive() && requestPhoto(this)) return undefined;
        return original(this);
      },
    });
  }
  function photoResult(message) {
    const n = photoNatives;
    const pending = photoPending;
    if (!n) return;
    if (!pending || message.requestId !== pending.requestId) return;
    if (message.type === "photo-accepted") {
      pending.accepted = true;
      clearTimeout(pending.timer);
      return;
    }
    clearPhotoPending();
    const input = pending.input.deref();
    // The page may have removed or disabled the input meanwhile. (It may also
    // have removed `capture` after opening it; the photo still belongs to it.)
    if (!input || !n.connected(input) || !(input instanceof HTMLInputElement) ||
        n.type(input) !== "file" || n.disabled(input)) return;
    if (message.type === "photo-cancel") {
      n.dispatch(input, new n.Event("cancel", { bubbles: true }));
      return;
    }
    if (!(message.data instanceof ArrayBuffer) || typeof message.name !== "string" || typeof message.mime !== "string") return;
    const file = new n.File([message.data], message.name.slice(0, 255), {
      type: message.mime.slice(0, 64),
      lastModified: Number.isFinite(message.lastModified) ? message.lastModified : Date.now(),
    });
    const transfer = new n.DataTransfer();
    n.itemsAdd(n.transferItems(transfer), file);
    n.setFiles(input, n.transferFiles(transfer));
    n.dispatch(input, new n.Event("input", { bubbles: true, composed: true }));
    n.dispatch(input, new n.Event("change", { bubbles: true }));
  }
  // This capture listener is installed before archive scripts. Only a genuine
  // Control+wheel gets article zoom; ordinary scrolling and map wheels stay local.
  window.addEventListener("wheel", (event) => {
    if (!event.isTrusted || !sessionId || document.hidden || !wheelControlKey(event)) return;
    const delta = wheelDeltaY(event);
    if (!Number.isFinite(delta) || delta === 0) return;
    preventWheelDefault(event);
    post("reading-zoom", { token: readingPanToken, sequence: ++readingPanSequence,
      url: location.href, direction: delta < 0 ? 1 : -1 });
  }, { capture: true, passive: false });
  function readingControls(hidden) {
    if (!readingControlsEnabled || hidden === readingPanHidden) return;
    readingPanHidden = hidden;
    post("reading-pan", { token: readingPanToken, sequence: ++readingPanSequence,
      url: location.href, hidden });
  }
  function cancelReadingPan() {
    readingTouch = null;
  }
  function resetReadingPan() {
    cancelReadingPan();
    readingControls(false);
  }
  function readingInteractionBlocked(target) {
    const editing = (node) => node instanceof Element &&
      (node.isContentEditable || !!node.closest(
        "input,textarea,select,button,[contenteditable]:not([contenteditable=false]),[role=slider]"));
    return document.hidden || (window.visualViewport?.scale ?? 1) !== 1 ||
      !window.getSelection()?.isCollapsed || editing(target) || editing(document.activeElement);
  }
  function reportReadingPan() {
    if (!readingControlsEnabled) return;
    if (readingInteractionBlocked()) {
      cancelReadingPan();
      return;
    }
    // Native scrolling can lag the first touchmove at the top. Reveal there,
    // but retain the live gesture so its subsequent scroll can hide controls.
    if (readingScrollY() <= 2) { readingControls(false); return; }
    const touch = readingTouch;
    if (!touch) return;
    const x = Math.abs(touch.x - touch.anchorX);
    const y = touch.anchorY - touch.y;
    const scrolled = readingScrollY() - touch.scrollY;
    // A map drag, overscroll, selection drag or scripted jump alone is not a
    // reading pan: both genuine vertical finger movement and root scroll agree.
    if (Math.abs(y) < x * 1.5) return;
    if (y >= 28 && scrolled >= 28 && readingScrollY() > readingControlsMinimumScroll) readingControls(true);
    else if (y <= -12 && scrolled <= -12) readingControls(false);
  }
  // Bootstrap's window capture handlers run before any archive capture handler.
  // Captured native getters also bypass later shadowing of genuine event data.
  window.addEventListener("touchstart", (event) => {
    if (!readingControlsEnabled || !event.isTrusted) return;
    const touches = readingTouches(event);
    if (readingTouchCount(touches) !== 1 || readingInteractionBlocked(event.target)) {
      resetReadingPan();
      return;
    }
    const point = readingTouchPoint(touches);
    readingTouch = { id: readingTouchIdentifier(point),
      anchorX: readingTouchX(point), anchorY: readingTouchY(point),
      x: readingTouchX(point), y: readingTouchY(point), scrollY: readingScrollY(), direction: 0,
      startedAt: performance.now(), moved: false };
  }, { passive: true, capture: true });
  window.addEventListener("touchmove", (event) => {
    if (!readingControlsEnabled || !event.isTrusted || !readingTouch) return;
    const touches = readingTouches(event);
    if (readingTouchCount(touches) !== 1 || readingInteractionBlocked(event.target)) {
      resetReadingPan();
      return;
    }
    const point = readingTouchPoint(touches), touch = readingTouch;
    if (readingTouchIdentifier(point) !== touch.id) { resetReadingPan(); return; }
    const step = touch.y - readingTouchY(point);
    const direction = Math.abs(step) >= 3 ? Math.sign(step) : touch.direction;
    if (touch.direction && direction !== touch.direction) {
      touch.anchorX = touch.x;
      touch.anchorY = touch.y;
      touch.scrollY = readingScrollY();
    }
    touch.direction = direction;
    touch.x = readingTouchX(point);
    touch.y = readingTouchY(point);
    if (Math.abs(touch.x - touch.anchorX) > 8 || Math.abs(touch.y - touch.anchorY) > 8)
      touch.moved = true;
    if (Math.abs(touch.x - touch.anchorX) > 16 &&
        Math.abs(touch.x - touch.anchorX) > Math.abs(touch.y - touch.anchorY) * 1.5)
      resetReadingPan();
    else reportReadingPan();
  }, { passive: true, capture: true });
  window.addEventListener("touchend", (event) => {
    if (!readingControlsEnabled || !event.isTrusted || !readingTouch) return;
    const ended = readingTouch;
    const tapped = !ended.moved && performance.now() - ended.startedAt < 500;
    reportReadingPan();
    if (tapped) resetReadingPan();
    // Native scrolling can settle just after touchend. Consume that one frame,
    // never unrelated later momentum or a subsequent programmatic restore.
    scheduleReadingFrame(() => {
      if (readingTouch !== ended) return;
      reportReadingPan();
      readingTouch = null;
    });
  }, { passive: true, capture: true });
  window.addEventListener("touchcancel", (event) => {
    if (event.isTrusted) cancelReadingPan();
  }, { passive: true, capture: true });
  function geoError(code, message, cancelled = false) {
    return Object.freeze({
      code,
      message,
      ...(cancelled ? { cancelled: true } : {}),
      PERMISSION_DENIED: 1,
      POSITION_UNAVAILABLE: 2,
      TIMEOUT: 3,
    });
  }
  function permissionError(message) {
    return new DOMException(message, "NotAllowedError");
  }
  function timeoutOf(options) {
    const value = Number(options?.timeout ?? 30000);
    return Number.isNaN(value) ? 30000 : Math.max(0, Math.min(30000, value));
  }
  function requestSensor(sensor, options) {
    if (pending.size >= 32)
      return {
        id: "",
        promise: Promise.reject(
          geoError(2, "Too many pending sensor requests"),
        ),
      };
    const id = String(++nextId);
    // Bound the permission/provider handoff independently of position acquisition.
    // Waiting for archive or OS consent must not spend PositionOptions.timeout.
    const requestLifetime = 120000;
    const message = { requestId: id, sensor, options };
    let rejectRequest;
    const promise = new Promise((resolve, reject) => {
      rejectRequest = reject;
      const timer = setTimeout(() => {
        pending.delete(id);
        post("sensor-cancel", { requestId: id });
        reject(geoError(2, "Sensor permission or provider request expired"));
      }, requestLifetime);
      pending.set(id, { resolve, reject, timer, sensor, message });
      post("sensor-request", message);
    });
    return {
      id,
      promise,
      cancel() {
        const entry = pending.get(id);
        if (!entry) return;
        clearTimeout(entry.timer);
        pending.delete(id);
        post("sensor-cancel", { requestId: id });
        rejectRequest(geoError(1, "Sensor request cancelled"));
      },
    };
  }

  function stopCamera() {
    for (const stream of streams)
      for (const track of stream.getTracks()) track.stop();
    streams.clear();
  }
  function revoke(sensor) {
    if (sensor === "camera") {
      cameraEpoch++;
      stopCamera();
    }
    if (sensor === "location") {
      locationEpoch++;
      for (const [id, watch] of watches) {
        clearTimeout(watch.timer);
        watch.cancel?.();
        watches.delete(id);
        if (watch.error)
          queueMicrotask(() =>
            watch.error(geoError(1, "Location permission revoked")),
          );
      }
      for (const cancel of browserRequests) cancel();
    }
    for (const [id, entry] of pending) {
      if (entry.sensor !== sensor) continue;
      clearTimeout(entry.timer);
      pending.delete(id);
      post("sensor-cancel", { requestId: id });
      entry.reject(geoError(1, "Sensor permission revoked"));
    }
  }

  if (navigator.mediaDevices && nativeGetUserMedia) {
    Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
      configurable: false,
      writable: false,
      value: async (constraints) => {
        if (
          !constraints ||
          (constraints.audio !== undefined && constraints.audio !== false) ||
          !constraints.video
        ) {
          throw permissionError(
            "Only camera-only requests are supported; microphone access is disabled",
          );
        }
        const epoch = cameraEpoch;
        const request = requestSensor("camera", {});
        let reply;
        try {
          reply = await request.promise;
        } catch (error) {
          throw permissionError(error.message || "Camera permission denied");
        }
        if (!reply.allowed)
          throw permissionError(
            reply.error?.message || "Camera permission denied for this archive",
          );
        if (epoch !== cameraEpoch)
          throw permissionError("Camera permission was revoked");
        // This is the actual browser media API: WebKit/WebView2/WebKitGTK and
        // the OS still own their permission prompts and the physical device.
        const stream = await nativeGetUserMedia({
          ...constraints,
          audio: false,
        });
        if (epoch !== cameraEpoch) {
          for (const track of stream.getTracks()) track.stop();
          throw permissionError("Camera permission was revoked");
        }
        streams.add(stream);
        for (const track of stream.getTracks())
          track.addEventListener("ended", () => {
            if (stream.getTracks().every((t) => t.readyState === "ended"))
              streams.delete(stream);
          });
        return stream;
      },
    });
  }

  function positionRequest(options) {
    const epoch = locationEpoch;
    const normalized = {
      enableHighAccuracy: Boolean(options?.enableHighAccuracy),
      timeout: timeoutOf(options),
      maximumAge: Math.min(
        4294967295,
        Math.max(0, Math.floor(Number(options?.maximumAge) || 0)),
      ),
    };
    const request = requestSensor("location", normalized);
    let browserCancelled = false;
    let cancelBrowser = null;
    const promise = request.promise.then((reply) => {
      if (browserCancelled || epoch !== locationEpoch)
        throw geoError(1, "Location permission revoked");
      if (!reply.allowed && reply.cancelled === true && reply.error?.code === 2)
        throw geoError(2, reply.error.message || "Location request was cancelled", true);
      if (!reply.allowed)
        throw geoError(
          reply.error?.code || 1,
          reply.error?.message || "Location permission denied for this archive",
        );
      if (reply.position) {
        return reply.position;
      }
      if (!reply.browser || !nativeWatchPosition || !nativeClearWatch)
        throw geoError(2, "No location provider is available on this platform");
      // Use the genuine browser provider after archive consent, including headless.
      // Its acquisition timeout excludes OS permission; do not race it with a timer.
      // A one-fix browser watch is cancellable (unlike getCurrentPosition).
      return new Promise((resolve, reject) => {
        let browserWatch;
        let settled = false;
        const finish = (failure, position) => {
          if (settled) return;
          settled = true;
          if (browserWatch !== undefined) nativeClearWatch(browserWatch);
          browserRequests.delete(cancelBrowser);
          if (browserCancelled || epoch !== locationEpoch)
            reject(geoError(1, "Location permission revoked"));
          else if (failure) reject(failure);
          else resolve(position);
        };
        cancelBrowser = () =>
          finish(geoError(1, "Location request cancelled or revoked"));
        browserRequests.add(cancelBrowser);
        try {
          browserWatch = nativeWatchPosition(
            (position) => finish(null, position),
            (error) => finish(geoError(error.code, error.message)),
            normalized,
          );
          if (settled) nativeClearWatch(browserWatch);
        } catch (error) {
          finish(
            geoError(2, error.message || "Browser location provider failed"),
          );
        }
      });
    });
    return {
      promise,
      cancel() {
        browserCancelled = true;
        request.cancel?.();
        cancelBrowser?.();
      },
    };
  }

  function getCurrentPosition(success, error, options) {
    if (typeof success !== "function")
      throw new TypeError("A geolocation success callback is required");
    if (error != null && typeof error !== "function")
      throw new TypeError("Geolocation error callback must be a function");
    const request = positionRequest(options);
    request.promise.then(
      (position) => success(position),
      (failure) => {
        if (error) error(failure);
      },
    );
  }
  function watchPosition(success, error, options) {
    if (typeof success !== "function")
      throw new TypeError("A geolocation success callback is required");
    if (error != null && typeof error !== "function")
      throw new TypeError("Geolocation error callback must be a function");
    const id = ++nextWatch;
    const watch = { error, timer: 0, cancel: null };
    watches.set(id, watch);
    const poll = () => {
      if (!watches.has(id)) return;
      const request = positionRequest(options);
      watch.cancel = request.cancel;
      request.promise.then(
        (position) => {
          if (!watches.has(id)) return;
          try {
            success(position);
          } finally {
            if (watches.has(id)) watch.timer = setTimeout(poll, 1000);
          }
        },
        (failure) => {
          if (!watches.has(id)) return;
          if (failure.code === 1 || failure.cancelled === true) watches.delete(id);
          else watch.timer = setTimeout(poll, 1000);
          if (error) error(failure);
        },
      );
    };
    poll();
    return id;
  }
  function clearWatch(id) {
    const watch = watches.get(Number(id));
    if (!watch) return;
    watches.delete(Number(id));
    clearTimeout(watch.timer);
    watch.cancel?.();
  }
  const geolocation = Object.freeze({
    getCurrentPosition,
    watchPosition,
    clearWatch,
  });
  // WKWebView may expose no native geolocation object at all. Define the same
  // public browser API while obtaining actual coordinates from CoreLocation.
  try {
    Object.defineProperty(navigator, "geolocation", {
      value: geolocation,
      configurable: false,
      writable: false,
    });
  } catch (_) {
    if (nativeGeolocation) {
      Object.defineProperties(nativeGeolocation, {
        getCurrentPosition: {
          value: getCurrentPosition,
          configurable: false,
          writable: false,
        },
        watchPosition: {
          value: watchPosition,
          configurable: false,
          writable: false,
        },
        clearWatch: { value: clearWatch, configurable: false, writable: false },
      });
    }
  }

  function toc() {
    const entries = [];
    Array.from(document.querySelectorAll("h1,h2,h3,h4,h5,h6"))
      .slice(0, 2000)
      .forEach((heading, index) => {
        // A heading with no text, or one the page hides, would be a blank,
        // unnamed entry in the contents panel. Generated ids keep the index of
        // the full heading list, so skipping never renumbers the others.
        const title = heading.textContent.trim();
        if (!title || heading.hidden || heading.getAttribute("aria-hidden") === "true") return;
        if (!heading.id) heading.id = `kiwix-heading-${index}`;
        entries.push({
          id: heading.id,
          title,
          level: Number(heading.tagName.slice(1)),
        });
      });
    return entries;
  }
  function voices() {
    return (
      window.speechSynthesis?.getVoices().map((voice) => ({
        name: voice.name,
        lang: voice.lang,
        voiceURI: voice.voiceURI,
        default: voice.default,
      })) || []
    );
  }
  let activeSpeech = null;
  function stopSpeech() {
    activeSpeech = null;
    // Keep the extracted document: native speech can survive backgrounding and
    // resume its offsets after this visibility stop. Navigation/session
    // replacement explicitly resets it below.
    try { clearReadAloudPaint(); } catch (_) { /* paint only */ }
    window.speechSynthesis?.cancel();
    post("speech-state", { speaking: false });
  }
  function speak(text, options, mapping) {
    stopSpeech();
    if (!window.speechSynthesis || !window.SpeechSynthesisUtterance) {
      post("speech-state", { speaking: false, error: "Speech synthesis is unavailable in this webview" });
      return;
    }
    const spoken = String(text);
    const source = mapping && mapping.text && mapping.runs?.length ? mapping : null;
    const base = source && Number.isSafeInteger(mapping.base) ? Math.max(0, mapping.base) : 0;
    if (source) {
      try { setReadAloudDocument(source); } catch (_) { /* paint only */ }
    } else resetReadAloud();
    const utterance = new SpeechSynthesisUtterance(spoken);
    utterance.lang = typeof options.lang === "string" ? options.lang : "";
    if (Number.isFinite(options.rate)) utterance.rate = Math.max(0.25, Math.min(2, options.rate));
    utterance.voice = speechSynthesis.getVoices().find((voice) => voice.voiceURI === options.voice) || null;
    const current = { spoken, base, offset: 0, utterance, state: source ? readAloudDocument : null };
    activeSpeech = current;
    utterance.onboundary = (event) => {
      if (activeSpeech !== current || !Number.isInteger(event.charIndex)) return;
      current.offset = Math.max(0, Math.min(spoken.length, event.charIndex));
      if (current.state) highlightReadAloud(current.base + current.offset);
    };
    utterance.onstart = () => {
      if (activeSpeech !== current) return;
      if (current.state) highlightReadAloud(current.base);
      post("speech-state", { speaking: true });
    };
    utterance.onend = () => {
      if (activeSpeech !== current) return;
      activeSpeech = null;
      resetReadAloud();
      post("speech-state", { speaking: false });
    };
    utterance.onerror = (event) => {
      if (activeSpeech !== current) return;
      activeSpeech = null;
      resetReadAloud();
      post("speech-state", { speaking: false, error: event.error });
    };
    speechSynthesis.speak(utterance);
    // Paint the first/resumed sentence while the engine is queued; onstart will
    // repaint the same range once playback is confirmed.
    if (current.state) highlightReadAloud(current.base);
    // Include queued speech so a fast Stop never accidentally starts a second utterance.
    post("speech-state", { speaking: true });
  }
  // Read-along highlighting is a paint-only layer. Speech engines report UTF-16
  // character offsets into the exact text this bridge extracted; those offsets
  // are mapped back to live text nodes without wrapping or rewriting archive
  // content. CSS Custom Highlight is preferred; older WebViews get an
  // aria-hidden, pointer-events-none overlay so selection and keyboard focus
  // remain untouched.
  const readAloudHighlightName = "kiwix-read-aloud";
  const readAloudWordHighlightName = "kiwix-read-aloud-word";
  const sentenceTerminators = ".!?…";
  const cjkSentenceTerminators = "。！？";
  const sentenceClosers = "\"'”’)]}»」』】）";
  let readAloudDocument = null;
  let readAloudStyle = null;
  let readAloudLayer = null;
  let readAloudMarks = null;
  let readAloudPoints = [];
  let readAloudFrame = 0;
  let readAloudOverlayRanges = null;
  let readAloudLastSentence = -1;

  function sentenceRanges(text) {
    const ranges = [];
    let start = -1;
    for (let index = 0; index < text.length; index++) {
      const character = text[index];
      if (start < 0) {
        if (/\s/.test(character)) continue;
        start = index;
      }
      if (character === "\n") {
        ranges.push({ start, end: index });
        start = -1;
        continue;
      }
      const cjk = cjkSentenceTerminators.includes(character);
      if (!cjk && !sentenceTerminators.includes(character)) continue;
      let end = index + 1;
      while (end < text.length &&
        (sentenceTerminators.includes(text[end]) || cjkSentenceTerminators.includes(text[end]) ||
         sentenceClosers.includes(text[end]))) end++;
      // CJK sentences normally have no space after the terminator; Latin ones
      // need whitespace/end so decimals and abbreviations are not split.
      if (!cjk && end < text.length && !/\s/.test(text[end])) continue;
      ranges.push({ start, end });
      start = -1;
      index = end - 1;
    }
    if (start >= 0) ranges.push({ start, end: text.length });
    return ranges;
  }
  function sentenceAt(ranges, offset) {
    let low = 0;
    let high = ranges.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (ranges[middle].end <= offset) low = middle + 1;
      else high = middle;
    }
    const next = ranges[low];
    const previous = ranges[low - 1];
    // A pause between sentences keeps the sentence just read highlighted.
    return previous && next && offset < next.start ? previous : next || previous || null;
  }
  function wordBounds(text, offset) {
    let index = Math.max(0, Math.min(text.length, offset));
    // Whitespace and CJK sentence terminators bound a word. CJK prose has no
    // spaces, so without the terminator bound the highlight would cover the
    // rest of the paragraph. Latin punctuation behavior is unchanged.
    while (index < text.length &&
      (/\s/.test(text[index]) || cjkSentenceTerminators.includes(text[index]))) index++;
    if (index >= text.length) return null;
    let end = index + 1;
    while (end < text.length && !/\s/.test(text[end]) &&
      !cjkSentenceTerminators.includes(text[end])) end++;
    return { start: index, end };
  }
  function speechPoint(runs, offset, end) {
    if (!runs.length) return null;
    let low = 0;
    let high = runs.length;
    if (end) {
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (runs[middle].start < offset) low = middle + 1;
        else high = middle;
      }
      const run = runs[low - 1];
      if (!run) return null;
      return { node: run.node, offset: run.offset + Math.min(run.end, offset) - run.start };
    }
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (runs[middle].end <= offset) low = middle + 1;
      else high = middle;
    }
    const run = runs[low];
    if (!run) return null;
    return { node: run.node, offset: run.offset + Math.max(0, offset - run.start) };
  }
  function speechRange(runs, start, end) {
    const first = speechPoint(runs, start, false);
    const last = speechPoint(runs, end, true);
    if (!first || !last || !first.node || !last.node) return null;
    if (first.node.isConnected === false || last.node.isConnected === false) return null;
    if (first.node === last.node && first.offset >= last.offset) return null;
    const range = document.createRange();
    try {
      range.setStart(first.node, first.offset);
      range.setEnd(last.node, last.offset);
    } catch (_) { return null; }
    return range.collapsed ? null : range;
  }
  function ensureReadAloudStyle() {
    if (readAloudStyle?.isConnected) return;
    readAloudStyle = document.createElement("style");
    readAloudStyle.dataset.kiwixReadAloudUi = "";
    readAloudStyle.textContent =
      "::highlight(kiwix-read-aloud){background-color:#f2ce687d;color:inherit;}" +
      "::highlight(kiwix-read-aloud-word){background-color:#e0a92ecc;color:inherit;}";
    (document.head || document.documentElement).append(readAloudStyle);
  }
  function clearReadAloudOverlay() {
    cancelAnimationFrame(readAloudFrame);
    readAloudFrame = 0;
    readAloudOverlayRanges = null;
    readAloudMarks?.replaceChildren();
    readAloudLayer?.remove();
    readAloudLayer = null;
    readAloudMarks = null;
    readAloudPoints = [];
  }
  function clearReadAloudPaint() {
    window.CSS?.highlights?.delete(readAloudHighlightName);
    window.CSS?.highlights?.delete(readAloudWordHighlightName);
    readAloudStyle?.remove();
    readAloudStyle = null;
    readAloudLastSentence = -1;
    clearReadAloudOverlay();
  }
  function resetReadAloud() {
    readAloudDocument = null;
    readAloudLastSentence = -1;
    try { clearReadAloudPaint(); } catch (_) { /* paint only */ }
  }
  function setReadAloudDocument(mapping) {
    readAloudDocument = mapping && mapping.text && mapping.runs?.length
      ? {
          text: mapping.text,
          runs: mapping.runs,
          sentences: mapping.sentences || sentenceRanges(mapping.text),
        }
      : null;
    readAloudLastSentence = -1;
    clearReadAloudPaint();
  }
  function ensureReadAloudOverlay() {
    if (readAloudLayer?.isConnected) return true;
    if (typeof document.createElement !== "function" || !document.documentElement) return false;
    try {
      readAloudLayer = document.createElement("div");
      readAloudLayer.dataset.kiwixReadAloudUi = "";
      readAloudLayer.setAttribute("aria-hidden", "true");
      readAloudLayer.style.cssText =
        "all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483645;overflow:hidden;contain:strict;";
      if (typeof readAloudLayer.attachShadow !== "function") { readAloudLayer = null; return false; }
      const root = readAloudLayer.attachShadow({ mode: "closed" });
      readAloudMarks = document.createElement("div");
      readAloudMarks.style.cssText = "all:initial;position:absolute;inset:0;pointer-events:none;";
      readAloudPoints = [[0, 0], [128, 0], [0, 128]].map(([left, top]) => {
        const point = document.createElement("span");
        point.style.cssText =
          `all:initial;position:absolute;left:${left}px;top:${top}px;width:0;height:0;pointer-events:none;`;
        root.append(point);
        return point;
      });
      root.append(readAloudMarks);
      document.documentElement.append(readAloudLayer);
      return true;
    } catch (_) {
      readAloudLayer = null;
      readAloudMarks = null;
      readAloudPoints = [];
      return false;
    }
  }
  function paintReadAloudOverlay() {
    readAloudFrame = 0;
    const entries = readAloudOverlayRanges;
    if (!entries?.length) {
      readAloudMarks?.replaceChildren();
      return;
    }
    if (!ensureReadAloudOverlay()) return;
    const [origin, horizontal, vertical] = readAloudPoints.map((point) => point.getBoundingClientRect());
    const a = (horizontal.left - origin.left) / 128;
    const b = (horizontal.top - origin.top) / 128;
    const c = (vertical.left - origin.left) / 128;
    const d = (vertical.top - origin.top) / 128;
    const determinant = a * d - b * c;
    if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-8) {
      readAloudMarks.replaceChildren();
      return;
    }
    const fragment = document.createDocumentFragment();
    let count = 0;
    let inspected = 0;
    for (const entry of entries) {
      const bounds = entry.range.getBoundingClientRect();
      if (bounds.bottom < 0 || bounds.top > innerHeight || bounds.right < 0 || bounds.left > innerWidth) continue;
      for (const rect of entry.range.getClientRects()) {
        if (++inspected > 50000 || ++count > 10000) break;
        if (rect.bottom < 0 || rect.top > innerHeight || rect.right < 0 || rect.left > innerWidth) continue;
        const mark = document.createElement("span");
        const x = rect.left - origin.left;
        const y = rect.top - origin.top;
        const localX = (d * x - c * y) / determinant;
        const localY = (a * y - b * x) / determinant;
        mark.style.cssText =
          `position:absolute;left:0;top:0;width:${rect.width}px;height:${rect.height}px;transform-origin:0 0;` +
          `transform:matrix(${d / determinant},${-b / determinant},${-c / determinant},${a / determinant},${localX},${localY});` +
          entry.style;
        fragment.append(mark);
      }
      if (count > 10000 || inspected > 50000) break;
    }
    readAloudMarks.replaceChildren(fragment);
  }
  function scheduleReadAloudOverlay() {
    if (!readAloudOverlayRanges?.length || readAloudFrame) return;
    readAloudFrame = requestAnimationFrame(paintReadAloudOverlay);
  }
  function revealReadAloud(range, sentenceStart) {
    if (!range || sentenceStart === readAloudLastSentence) return;
    readAloudLastSentence = sentenceStart;
    let rect;
    try { rect = range.getBoundingClientRect(); } catch (_) { return; }
    if (!rect || !Number.isFinite(rect.top) || !Number.isFinite(rect.bottom)) return;
    const viewport = Number.isFinite(innerHeight) ? innerHeight : 0;
    // Do not scroll when any part of the active sentence is already visible.
    if (!viewport || (rect.bottom > 0 && rect.top < viewport)) return;
    if (typeof window.scrollTo !== "function") return;
    const current = Number.isFinite(window.scrollY) ? window.scrollY : 0;
    const top = Math.max(0, current + rect.top - viewport / 3);
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches === true;
    try { window.scrollTo({ top, behavior: reduce ? "auto" : "smooth" }); }
    catch (_) { window.scrollTo(0, top); }
  }
  function paintReadAloud(sentenceRange, wordRange, sentenceStart) {
    if (!sentenceRange && !wordRange) { clearReadAloudPaint(); return; }
    if (window.CSS?.highlights && typeof window.Highlight === "function") {
      ensureReadAloudStyle();
      window.CSS.highlights.set(readAloudHighlightName, new Highlight(sentenceRange || wordRange));
      if (wordRange) window.CSS.highlights.set(readAloudWordHighlightName, new Highlight(wordRange));
      else window.CSS.highlights.delete(readAloudWordHighlightName);
      if (readAloudLayer) clearReadAloudOverlay();
    } else {
      window.CSS?.highlights?.delete(readAloudHighlightName);
      window.CSS?.highlights?.delete(readAloudWordHighlightName);
      readAloudStyle?.remove();
      readAloudStyle = null;
      readAloudOverlayRanges = [];
      if (sentenceRange)
        readAloudOverlayRanges.push({
          range: sentenceRange,
          style: "background:#f2ce687d;border-radius:2px;",
        });
      if (wordRange)
        readAloudOverlayRanges.push({
          range: wordRange,
          style: "background:#e0a92ecc;border-radius:2px;box-shadow:inset 0 -2px #b07a16;",
        });
      scheduleReadAloudOverlay();
    }
    try { revealReadAloud(sentenceRange, sentenceStart); } catch (_) { /* paint only */ }
  }
  function highlightReadAloud(offset) {
    try {
      const documentState = readAloudDocument;
      if (!documentState || !Number.isSafeInteger(offset)) return;
      const position = Math.max(0, Math.min(documentState.text.length, offset));
      const sentence = sentenceAt(documentState.sentences, position);
      if (!sentence) { clearReadAloudPaint(); return; }
      const sentenceRange = speechRange(documentState.runs, sentence.start, sentence.end);
      const word = wordBounds(documentState.text, position);
      const wordRange = word ? speechRange(documentState.runs, word.start, word.end) : null;
      paintReadAloud(sentenceRange, wordRange, sentence.start);
    } catch (_) {
      // Highlighting is paint-only; a failed range must never interrupt speech.
      try { clearReadAloudPaint(); } catch (_) { /* paint only */ }
    }
  }
  window.addEventListener("scroll", scheduleReadAloudOverlay, { passive: true, capture: true });
  window.addEventListener("resize", scheduleReadAloudOverlay, { passive: true });
  // Only a contiguous, positively observed live-document segment may use go().
  // Do not wrap history.state: archives own its type, contents and identity.
  function createDocumentHistory(url) {
    function entry(value) {
      let key = value;
      try {
        const target = new URL(value);
        // Keep path/query/fragment separate: an encoded ? or # belongs to
        // the archive entry, not to the URL's search or fragment.
        key = JSON.stringify([target.origin, decodeURIComponent(target.pathname), target.search, target.hash]);
      } catch (_) { /* Relative test values and malformed locations stay exact. */ }
      return { url: value, key };
    }
    let entries = [entry(url)];
    let cursor = 0;
    return {
      get url() { return entries[cursor].url; },
      reset(value) { entries = [entry(value)]; cursor = 0; },
      push(value) {
        entries.splice(cursor + 1);
        entries.push(entry(value));
        cursor++;
        if (entries.length > 512) { entries.shift(); cursor--; }
      },
      replace(value) { entries[cursor] = entry(value); },
      target(delta) { return entries[cursor + delta]?.url; },
      delta(direction, destinations) {
        const step = direction === "back" ? -1 : 1;
        let index = cursor;
        let previous = entries[cursor].key;
        for (const destination of destinations) {
          do { index += step; } while (index >= 0 && index < entries.length && entries[index].key === previous);
          if (entries[index]?.key !== entry(destination).key) return null;
          previous = entries[index].key;
        }
        return index - cursor;
      },
      commit(delta, value) {
        const index = cursor + delta;
        if (!Number.isSafeInteger(delta) || !delta || index < 0 || index >= entries.length || entries[index].url !== value) return false;
        cursor = index;
        return true;
      },
    };
  }
  const documentHistory = createDocumentHistory(location.href);
  let expectedHashNavigation = null;
  let documentTraversal = null;
  function resetDocumentHistory() {
    expectedHashNavigation = null;
    documentTraversal = null;
    documentHistory.reset(location.href);
  }
  function expectHashNavigation(url) {
    const expected = { from: location.href, url };
    expectedHashNavigation = expected;
    // A later archive listener may cancel an anchor's default action.
    queueMicrotask(() => {
      if (expectedHashNavigation === expected && location.href !== url)
        expectedHashNavigation = null;
    });
  }
  function traverseDocumentHistory(message) {
    const refuse = () => post("history-result", { requestId: message.requestId, accepted: false, url: location.href });
    if (typeof message.requestId !== "string" || !uuid.test(message.requestId)) return;
    if (documentTraversal || message.url !== location.href || documentHistory.url !== location.href ||
        !["back", "forward"].includes(message.direction) ||
        !Array.isArray(message.destinations) || !message.destinations.length || message.destinations.length > 200 ||
        !Number.isFinite(message.y) || message.y < 0 || message.y > 100000000) return refuse();
    for (const value of message.destinations) {
      if (typeof value !== "string" || value.length > 16384) return refuse();
      try {
        const target = new URL(value);
        if (target.origin !== location.origin || target.username || target.password ||
            target.pathname.startsWith("/.kiwix/")) return refuse();
      } catch (_) { return refuse(); }
    }
    const delta = documentHistory.delta(message.direction, message.destinations);
    if (!Number.isSafeInteger(delta) || !delta) return refuse();
    documentTraversal = {
      requestId: message.requestId, sessionId, delta, y: message.y,
      url: documentHistory.target(delta), arrived: false,
    };
    try { history.go(delta); }
    catch (_) { resetDocumentHistory(); refuse(); }
  }
  function finishDocumentTraversal(traversal, url = location.href, restoreScroll = true) {
    if (documentTraversal !== traversal || sessionId !== traversal.sessionId) return;
    if (restoreScroll && location.href !== traversal.url) return;
    documentTraversal = null;
    if (restoreScroll) window.scrollTo(0, traversal.y);
    navigationChanged("pop", traversal.requestId, url);
  }
  function documentHistoryChanged(kind) {
    const traversal = documentTraversal;
    if (traversal?.arrived && kind === "hash" && documentHistory.url === location.href) return;
    if (traversal && !traversal.arrived && traversal.url === location.href &&
        documentHistory.commit(traversal.delta, location.href)) {
      traversal.arrived = true;
      expectedHashNavigation = null;
      requestAnimationFrame(() => finishDocumentTraversal(traversal));
      return;
    }
    const expected = expectedHashNavigation;
    if (expected && expected.from === documentHistory.url && expected.url === location.href) {
      expectedHashNavigation = null;
      documentTraversal = null;
      documentHistory.push(location.href);
      navigationChanged("push");
      return;
    }
    if (kind === "hash" && documentHistory.url === location.href) return;
    resetDocumentHistory();
    navigationChanged("pop");
  }
  function headingFragment() {
    if (!location.hash || /^#[/!]/.test(location.hash)) return false;
    try {
      const id = decodeURIComponent(location.hash.slice(1));
      return Boolean(
        document.getElementById(id) || document.getElementsByName(id)[0],
      );
    } catch (_) {
      return false;
    }
  }
  function ready() {
    updateAnnotationObservation();
    post("ready", {
      title: document.title,
      url: location.href,
      headingFragment: headingFragment(),
      mime: script?.dataset.entryMime || document.contentType,
      navigationKind:
        performance.getEntriesByType("navigation")[0]?.type === "back_forward"
          ? "pop"
          : "push",
      toc: toc(),
      voices: voices(),
      capabilities: {
        camera: Boolean(nativeGetUserMedia),
        geolocation: true,
        speech: Boolean(
          window.speechSynthesis && window.SpeechSynthesisUtterance,
        ),
      },
    });
  }
  function reveal(element) {
    for (
      let current = element;
      current && current !== document.documentElement;
      current = current.parentElement
    ) {
      if (current.tagName === "DETAILS") current.open = true;
      current.hidden = false;
      current.classList.remove(
        "mw-collapsed",
        "collapsed",
        "pcs-collapse-section-hidden",
      );
      if (current.getAttribute("aria-expanded") === "false")
        current.setAttribute("aria-expanded", "true");
      if (current.getAttribute("aria-hidden") === "true")
        current.setAttribute("aria-hidden", "false");
      if (current.style.display === "none")
        current.style.removeProperty("display");
      if (
        current.matches(
          ".mw-collapsible-content,.collapsible-block,.pcs-collapse-section",
        ) &&
        getComputedStyle(current).display === "none"
      )
        current.style.display = "block";
    }
  }
  function revealSearchableContent() {
    for (const element of document.querySelectorAll(
      "details:not([open]),.mw-collapsed,.collapsible.collapsed,.pcs-collapse-section-hidden,.mw-collapsible-content,.collapsible-block",
    ))
      reveal(element);
  }
  function navigateToHeading(id) {
    const element = document.getElementById(id);
    if (!element) return;
    reveal(element);
    const hash = `#${encodeURIComponent(id)}`;
    if (location.hash !== hash) {
      expectHashNavigation(new URL(hash, location.href).href);
      location.hash = hash;
    }
    element.scrollIntoView();
  }
  // Text offsets are UTF-16, like DOM Range offsets and JavaScript strings.
  // Reject entire excluded subtrees; never rewrite or split archive text nodes.
  const annotationExcluded =
    "script,style,noscript,template,input,textarea,select,option,button,meter,progress,output," +
    "iframe,object,embed,svg,canvas,[contenteditable]:not([contenteditable='false'])," +
    "[role='button'],[role='textbox'],[role='combobox'],[role='listbox']," +
    "[role='slider'],[role='spinbutton'],[role='switch'],[role='checkbox'],[role='radio']," +
    "[data-kiwix-annotation-ui],[data-kiwix-read-aloud-ui]";
  // Both bridge paint layers are outside archive content. Their insertion and
  // removal must not invalidate saved passage ranges.
  const annotationUiSelector = "[data-kiwix-annotation-ui],[data-kiwix-read-aloud-ui]";
  const maxIndexText = 5000000;
  const maxIndexNodes = 100000;
  const maxDocumentHighlights = 1000;
  let annotationItems = [];
  let annotationDocument = "";
  let annotationUrl = "";
  let annotationReceiving = false;
  let annotationRanges = [];
  let annotationLayer;
  let annotationStyle;
  let annotationTimer;
  let annotationFrame = 0;
  const highlightName = "kiwix-passages";
  const uuid =
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

  function documentKey(path) {
    const split = path.search(/[?#]/);
    const pathname = split < 0 ? path : path.slice(0, split);
    const suffix = split < 0 ? "" : path.slice(split);
    try {
      return JSON.stringify([decodeURIComponent(pathname), suffix]);
    } catch (_) {
      return JSON.stringify([pathname, suffix]);
    }
  }
  function textIndex() {
    if (!document.body || document.body.matches(annotationExcluded))
      return null;
    let visited = 0;
    let section = 0;
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
      {
        acceptNode(node) {
          // Count rejected/empty nodes too, not just accepted text leaves.
          if (++visited > maxIndexNodes) return NodeFilter.FILTER_ACCEPT;
          if (node.nodeType === Node.ELEMENT_NODE) {
            if (node.matches(annotationExcluded)) {
              section++;
              return NodeFilter.FILTER_REJECT;
            }
            return NodeFilter.FILTER_ACCEPT;
          }
          return node.nodeValue
            ? NodeFilter.FILTER_ACCEPT
            : NodeFilter.FILTER_SKIP;
        },
      },
    );
    const nodes = [];
    const parts = [];
    let length = 0;
    let node;
    while ((node = walker.nextNode())) {
      if (visited > maxIndexNodes) return null;
      if (node.nodeType !== Node.TEXT_NODE) continue;
      if (length + node.nodeValue.length > maxIndexText) return null;
      nodes.push({
        node,
        section,
        start: length,
        end: length + node.nodeValue.length,
      });
      parts.push(node.nodeValue);
      length += node.nodeValue.length;
    }
    return { nodes, text: parts.join("") };
  }
  function indexRange(index, start, end) {
    const endpoint = (offset, inclusive) => {
      let low = 0;
      let high = index.nodes.length;
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (
          inclusive
            ? index.nodes[middle].end < offset
            : index.nodes[middle].end <= offset
        )
          low = middle + 1;
        else high = middle;
      }
      return index.nodes[low];
    };
    const first = endpoint(start, false);
    const last = endpoint(end, true);
    if (!first || !last || first.section !== last.section || end <= start)
      return null;
    const range = document.createRange();
    range.setStart(first.node, start - first.start);
    range.setEnd(last.node, end - last.start);
    if (range.toString() !== index.text.slice(start, end)) return null;
    return range;
  }
  function boundaryOffset(index, container, offset) {
    const entry = index.nodes.find((item) => item.node === container);
    if (entry) return entry.start + offset;
    if (!document.body.contains(container)) return null;
    const boundary = document.createRange();
    boundary.setStart(container, offset);
    boundary.collapse(true);
    for (const item of index.nodes)
      if (boundary.comparePoint(item.node, 0) >= 0) return item.start;
    return index.text.length;
  }
  // A selection report carries the anchor offsets, so mapping stays exact even
  // after the shell takes focus and the live selection collapses.
  function textIndexMapping(text, start, end) {
    if (!text || !Number.isSafeInteger(start) || !Number.isSafeInteger(end) ||
        start < 0 || end <= start || end - start !== text.length) return null;
    const index = textIndex();
    if (!index || end > index.text.length || index.text.slice(start, end) !== text) return null;
    const runs = [];
    for (const entry of index.nodes) {
      const from = Math.max(start, entry.start);
      const to = Math.min(end, entry.end);
      if (to <= from) continue;
      runs.push({ start: from - start, end: to - start, node: entry.node, offset: from - entry.start });
    }
    return runs.length ? { text, runs } : null;
  }
  function selectionSpeechMapping(text) {
    const selected = window.getSelection();
    if (!selected || selected.rangeCount !== 1 || selected.isCollapsed ||
        selected.toString() !== text) return null;
    const range = selected.getRangeAt(0);
    const root = document.body || document.documentElement;
    if (!root) return null;
    const runs = [];
    const parts = [];
    let length = 0;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (!range.intersectsNode(node)) continue;
      const start = node === range.startContainer ? range.startOffset : 0;
      const end = node === range.endContainer ? range.endOffset : node.nodeValue.length;
      if (end <= start) continue;
      const value = node.nodeValue.slice(start, end);
      runs.push({ start: length, end: length + value.length, node, offset: start });
      parts.push(value);
      length += value.length;
    }
    return runs.length && parts.join("") === text ? { text, runs } : null;
  }
  function textSearchMapping(text) {
    if (!text) return null;
    const index = textIndex();
    if (!index) return null;
    const at = index.text.indexOf(text);
    // A duplicated passage is ambiguous; never guess which copy was spoken.
    if (at < 0 || index.text.indexOf(text, at + 1) >= 0) return null;
    const runs = [];
    for (const entry of index.nodes) {
      const from = Math.max(at, entry.start);
      const to = Math.min(at + text.length, entry.end);
      if (to > from)
        runs.push({ start: from - at, end: to - at, node: entry.node, offset: from - entry.start });
    }
    return runs.length ? { text, runs } : null;
  }
  function selectionReport() {
    const selected = window.getSelection();
    const text = (selected?.toString() || "").slice(0, 120000);
    const report = {
      text,
      url: location.href.slice(0, 16384),
      headingFragment: headingFragment(),
      anchor: null,
    };
    if (!selected || selected.rangeCount !== 1 || selected.isCollapsed) {
      post("selection", report);
      return;
    }
    const range = selected.getRangeAt(0);
    if (
      text.length > 16384 ||
      range.startContainer.parentElement?.closest(annotationExcluded) ||
      range.endContainer.parentElement?.closest(annotationExcluded)
    ) {
      post("selection", report);
      return;
    }
    const index = textIndex();
    if (index) {
      const start = boundaryOffset(
        index,
        range.startContainer,
        range.startOffset,
      );
      const end = boundaryOffset(index, range.endContainer, range.endOffset);
      if (start === null || end === null) {
        post("selection", report);
        return;
      }
      const exact = index.text.slice(start, end);
      // A selection crossing excluded content is not a valid passage.
      if (
        exact &&
        exact.length <= 16384 &&
        exact === range.toString() &&
        indexRange(index, start, end)
      )
        report.anchor = {
          exact,
          start,
          end,
          prefix: index.text
            .slice(Math.max(0, start - 128), start)
            .replace(/^[\uDC00-\uDFFF]/, ""),
          suffix: index.text
            .slice(end, end + 128)
            .replace(/[\uD800-\uDBFF]$/, ""),
        };
    }
    post("selection", report);
  }
  function validAnnotation(item) {
    return (
      item &&
      typeof item === "object" &&
      typeof item.id === "string" &&
      uuid.test(item.id) &&
      typeof item.bookmarkId === "string" &&
      uuid.test(item.bookmarkId) &&
      typeof item.documentPath === "string" &&
      item.documentPath.length <= 16384 &&
      typeof item.exact === "string" &&
      item.exact.length > 0 &&
      item.exact.length <= 16384 &&
      typeof item.prefix === "string" &&
      item.prefix.length <= 128 &&
      typeof item.suffix === "string" &&
      item.suffix.length <= 128 &&
      Number.isInteger(item.start) &&
      item.start >= 0 &&
      Number.isInteger(item.end) &&
      item.end <= 100000000 &&
      item.end > item.start &&
      item.end - item.start === item.exact.length
    );
  }
  function resolveAnnotation(index, item, budget) {
    const contextMatches = (at) =>
      (!item.prefix ||
        index.text.slice(Math.max(0, at - item.prefix.length), at) ===
          item.prefix) &&
      (!item.suffix ||
        index.text.slice(
          at + item.exact.length,
          at + item.exact.length + item.suffix.length,
        ) === item.suffix);
    if (
      index.text.slice(item.start, item.end) === item.exact &&
      contextMatches(item.start)
    ) {
      const range = indexRange(index, item.start, item.end);
      if (range) return { range, status: "found" };
    }
    if (budget.remaining < index.text.length) return { status: "unavailable" };
    budget.remaining -= index.text.length;
    let found = null;
    let at = index.text.indexOf(item.exact);
    let matches = 0;
    while (at >= 0) {
      if (--budget.candidates < 0) return { status: "unavailable" };
      if (contextMatches(at)) {
        const range = indexRange(index, at, at + item.exact.length);
        if (range) {
          found = range;
          if (++matches > 1) return { status: "ambiguous" };
        }
      }
      at = index.text.indexOf(item.exact, at + 1);
    }
    return found
      ? { range: found, status: "found" }
      : { status: "unavailable" };
  }
  // Overlay rectangles are a non-destructive fallback for older WebViews. They
  // never wrap text or interfere with links, listeners, selection, or layout.
  let overlayRoot;
  let overlayMarks;
  let overlayPoints = [];
  function paintAnnotations() {
    annotationFrame = 0;
    if (!annotationRanges.length) {
      window.CSS?.highlights?.delete(highlightName);
      annotationStyle?.remove();
      annotationStyle = null;
      annotationLayer?.remove();
      annotationLayer = null;
      overlayRoot = null;
      overlayMarks = null;
      overlayPoints = [];
      return;
    }
    if (window.CSS?.highlights && typeof window.Highlight === "function") {
      CSS.highlights.set(
        highlightName,
        new Highlight(...annotationRanges.map((item) => item.range)),
      );
      overlayMarks?.replaceChildren();
      return;
    }
    if (!annotationLayer?.isConnected) {
      annotationLayer = document.createElement("div");
      annotationLayer.dataset.kiwixAnnotationUi = "";
      annotationLayer.setAttribute("aria-hidden", "true");
      annotationLayer.style.cssText =
        "all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483646;overflow:hidden;contain:strict;";
      overlayRoot = annotationLayer.attachShadow({ mode: "closed" });
      overlayMarks = document.createElement("div");
      overlayMarks.style.cssText =
        "all:initial;position:absolute;inset:0;pointer-events:none;";
      // Three zero-size probes measure the actual local-to-viewport affine map.
      // This includes root zoom, translated/scaled/rotated ancestors and scroll;
      // getClientRects coordinates must not be zoomed a second time.
      overlayPoints = [
        [0, 0],
        [128, 0],
        [0, 128],
      ].map(([left, top]) => {
        const point = document.createElement("span");
        point.style.cssText = `all:initial;position:absolute;left:${left}px;top:${top}px;width:0;height:0;pointer-events:none;`;
        overlayRoot.append(point);
        return point;
      });
      overlayRoot.append(overlayMarks);
      document.documentElement.append(annotationLayer);
    }
    if (!overlayMarks || overlayPoints.length !== 3) return;
    const [origin, horizontal, vertical] = overlayPoints.map((point) =>
      point.getBoundingClientRect(),
    );
    const a = (horizontal.left - origin.left) / 128;
    const b = (horizontal.top - origin.top) / 128;
    const c = (vertical.left - origin.left) / 128;
    const d = (vertical.top - origin.top) / 128;
    const determinant = a * d - b * c;
    if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-8) {
      overlayMarks.replaceChildren();
      return;
    }
    const fragment = document.createDocumentFragment();
    let count = 0;
    let inspected = 0;
    for (const { range } of annotationRanges) {
      const bounds = range.getBoundingClientRect();
      if (
        bounds.bottom < 0 ||
        bounds.top > innerHeight ||
        bounds.right < 0 ||
        bounds.left > innerWidth
      )
        continue;
      for (const rect of range.getClientRects()) {
        if (++inspected > 50000) break;
        if (
          rect.bottom < 0 ||
          rect.top > innerHeight ||
          rect.right < 0 ||
          rect.left > innerWidth
        )
          continue;
        if (++count > 10000) break;
        const mark = document.createElement("span");
        const x = rect.left - origin.left;
        const y = rect.top - origin.top;
        const localX = (d * x - c * y) / determinant;
        const localY = (a * y - b * x) / determinant;
        mark.style.cssText =
          `position:absolute;left:0;top:0;width:${rect.width}px;height:${rect.height}px;transform-origin:0 0;` +
          `transform:matrix(${d / determinant},${-b / determinant},${-c / determinant},${a / determinant},${localX},${localY});` +
          "background:#e9b94355;box-shadow:inset 0 -1px #ad7c35;border-radius:2px;pointer-events:none;";
        fragment.append(mark);
      }
      if (count > 10000 || inspected > 50000) break;
    }
    overlayMarks.replaceChildren(fragment);
  }
  function scheduleAnnotationPaint() {
    if (!annotationFrame && annotationRanges.length)
      annotationFrame = requestAnimationFrame(paintAnnotations);
  }
  function renderAnnotations() {
    clearTimeout(annotationTimer);
    if (
      annotationReceiving ||
      (annotationUrl && annotationUrl !== location.href)
    )
      return;
    annotationRanges = [];
    const index = annotationItems.length ? textIndex() : null;
    const budget = { remaining: 25000000, candidates: 50000 };
    const statuses = [];
    for (const item of annotationItems) {
      const result = index
        ? resolveAnnotation(index, item, budget)
        : { status: "unavailable" };
      if (result.range)
        annotationRanges.push({
          id: item.id,
          bookmarkId: item.bookmarkId,
          range: result.range,
        });
      statuses.push({
        id: item.id,
        bookmarkId: item.bookmarkId,
        status: result.range
          ? "found"
          : result.status === "ambiguous"
            ? "ambiguous"
            : "unavailable",
      });
    }
    if (
      annotationRanges.length &&
      window.CSS?.highlights &&
      typeof window.Highlight === "function" &&
      !annotationStyle?.isConnected
    ) {
      annotationStyle = document.createElement("style");
      annotationStyle.dataset.kiwixAnnotationUi = "";
      annotationStyle.textContent =
        "::highlight(kiwix-passages){background-color:#f2ce687d;color:inherit;text-decoration:underline #aa751d;}";
      (document.head || document.documentElement).append(annotationStyle);
    }
    paintAnnotations();
    for (let offset = 0; offset < statuses.length; offset += 200)
      post("highlight-status", {
        url: location.href,
        statuses: statuses.slice(offset, offset + 200),
      });
  }
  function annotationMutated(records) {
    if (
      !annotationItems.length ||
      records.every(
        (record) =>
          (record.target.nodeType === Node.ELEMENT_NODE &&
            record.target.closest(annotationUiSelector)) ||
          (record.type === "childList" &&
            [...record.addedNodes, ...record.removedNodes].every(
              (node) =>
                node.nodeType === Node.ELEMENT_NODE &&
                node.matches(annotationUiSelector),
            )),
      )
    )
      return;
    clearTimeout(annotationTimer);
    // Live ranges track text/child mutations before we can validate their
    // quotes, so clear paint immediately for structural/text changes. Pure
    // attribute toggles (e.g. expanding a collapsible section) do not mutate
    // text content; let the debounced re-anchor repaint without flicker.
    if (records.some((record) => record.type !== "attributes")) {
      annotationRanges = [];
      window.CSS?.highlights?.delete(highlightName);
      overlayMarks?.replaceChildren();
    }
    annotationTimer = setTimeout(renderAnnotations, 180);
  }
  const annotationObserverOptions = {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
    attributeFilter: ["hidden", "open", "class", "style", "contenteditable"],
  };
  let annotationObserver;
  let annotationResize;
  function stopAnnotationObservation() {
    annotationObserver?.disconnect();
    annotationResize?.disconnect();
  }
  function updateAnnotationObservation() {
    if (
      annotationReceiving ||
      !annotationItems.length ||
      annotationUrl !== location.href
    ) {
      stopAnnotationObservation();
      return;
    }
    // Do not allocate parser MutationRecords for documents with no highlights.
    annotationObserver ??= new MutationObserver(annotationMutated);
    annotationObserver.observe(document.documentElement, annotationObserverOptions);
    if (typeof ResizeObserver === "function") {
      annotationResize ??= new ResizeObserver(scheduleAnnotationPaint);
      if (document.body) annotationResize.observe(document.body);
    }
  }
  window.addEventListener("scroll", scheduleAnnotationPaint, {
    passive: true,
    capture: true,
  });
  window.addEventListener("resize", scheduleAnnotationPaint, { passive: true });
  let originalAppearance;
  const originalFontFamily = document.documentElement.style.fontFamily;
  // Reader appearance. A scoped stylesheet recolors the document and only the
  // neutral surfaces the archive itself paints; images, media, coloured callouts
  // and archive-authored CSS files are never rewritten. "original" removes every
  // trace and restores the document's own inline values.
  const themeSurfaces = {
    light: { background: "#ffffff", text: "#292920", page: "#f7f7f8", raised: "#ececef", line: "#c7cacf" },
    sepia: { background: "#f4ecd8", text: "#292920", page: "#efe5cf", raised: "#e6d9bd", line: "#c4b394" },
    dark: { background: "#171a1e", text: "#e8ebef", page: "#1f242a", raised: "#272d34", line: "#3c444e" },
  };
  const themeNames = Object.keys(themeSurfaces);
  const themeExcluded = new Set([
    "IMG", "SVG", "VIDEO", "AUDIO", "IFRAME", "OBJECT", "EMBED", "CANVAS",
    "MAP", "SCRIPT", "STYLE", "LINK", "META", "INPUT", "SELECT", "TEXTAREA",
    "BUTTON", "PROGRESS", "METER", "TEMPLATE", "NOSCRIPT", "HEAD",
  ]);
  // The theme pass must never visit a decoder. A video player's chrome (the
  // video.js/ogv.js wrapper, its canvas and its controls) mutates class and
  // style on every progress tick, so walking it re-scans the player several
  // times a second while it is already the most expensive thing on the page.
  // `data-kiwix-media-surface` is set on the wrapper when the video is
  // prepared; the bridge's own notice is opaque for the same reason.
  function themeOpaque(element) {
    return (
      element.tagName === "VIDEO" ||
      element.tagName === "CANVAS" ||
      element.hasAttribute?.("data-kiwix-media-surface") === true ||
      element.hasAttribute?.("data-kiwix-notice") === true
    );
  }
  function insideThemeOpaque(element) {
    for (
      let node = element;
      node && node.nodeType === Node.ELEMENT_NODE;
      node = node.parentElement
    )
      if (themeOpaque(node)) return true;
    return false;
  }
  // Style recalculations are batched: a limit keeps a long article's scan
  // responsive without paying a recalculation per element.
  const themeBatchLimit = 256;
  const themeState = {
    theme: "original",
    style: null,
    observer: null,
    timer: 0,
    queue: [],
    index: 0,
    queued: new WeakSet(),
    backdrop: new WeakMap(),
    batch: [],
    readings: [],
  };
  function themeStyleText() {
    const blocks = themeNames
      .map((name) => {
        const surface = themeSurfaces[name];
        return `:root[data-kiwix-theme="${name}"]{--kiwix-theme-bg:${surface.background};--kiwix-theme-text:${surface.text};--kiwix-theme-page:${surface.page};--kiwix-theme-raised:${surface.raised};--kiwix-theme-line:${surface.line};}`;
      })
      .join("");
    return (
      blocks +
      ":root[data-kiwix-theme]{background-color:var(--kiwix-theme-bg)!important;}" +
      ":root[data-kiwix-theme] body{background-color:var(--kiwix-theme-bg)!important;color:var(--kiwix-theme-text)!important;}" +
      ":root[data-kiwix-theme] [data-kiwix-notice]{color:var(--kiwix-theme-text)!important;}" +
      ':root[data-kiwix-theme] [data-kiwix-surface="page"]{background-color:var(--kiwix-theme-page)!important;}' +
      ':root[data-kiwix-theme] [data-kiwix-surface="raised"]{background-color:var(--kiwix-theme-raised)!important;}' +
      ":root[data-kiwix-theme] [data-kiwix-ink]{color:var(--kiwix-theme-text)!important;}" +
      ":root[data-kiwix-theme] [data-kiwix-edge]{border-color:var(--kiwix-theme-line)!important;}"
    );
  }
  // Reading width is a measure, not a recolour: one scoped stylesheet widens or
  // narrows the article column and never touches the archive's own type. Media
  // documents have no reading column, so they are left exactly as authored.
  const widthState = { width: "original", style: null };
  const widthRules = {
    comfortable:
      ':root[data-kiwix-width="comfortable"] body{max-width:52rem!important;margin-left:auto!important;margin-right:auto!important;}' +
      ':root[data-kiwix-width="comfortable"] img,:root[data-kiwix-width="comfortable"] video,:root[data-kiwix-width="comfortable"] table,:root[data-kiwix-width="comfortable"] pre{max-width:100%;}',
    // Full width releases the body measure plus the common wrappers that
    // archives use to cap the column. It expands these well-known containers,
    // not every bespoke layout an archive may ship.
    full:
      ':root[data-kiwix-width="full"] body{max-width:none!important;}' +
      ':root[data-kiwix-width="full"] .mw-page-container,:root[data-kiwix-width="full"] .mw-page-container-inner,:root[data-kiwix-width="full"] .mw-content-container,:root[data-kiwix-width="full"] #content,:root[data-kiwix-width="full"] #mw-content-text,:root[data-kiwix-width="full"] .vector-body,:root[data-kiwix-width="full"] #page,:root[data-kiwix-width="full"] .container,:root[data-kiwix-width="full"] article,:root[data-kiwix-width="full"] main{max-width:none!important;}',
  };
  // Textual resources (HTML, XHTML, plain text, JSON, XML) have a reading
  // column; media and PDF documents are laid out by their own renderer.
  function widthCapableMime(mime) {
    if (!mime) return true;
    if (/^(?:image|video|audio)\//i.test(mime)) return false;
    if (/^application\/pdf\b/i.test(mime)) return false;
    return (
      /^text\/html\b/i.test(mime) ||
      /^application\/xhtml\+xml\b/i.test(mime) ||
      /^text\//i.test(mime) ||
      /^application\/(?:json|xml)\b/i.test(mime)
    );
  }
  function applyReaderWidth(width) {
    // Nested same-origin frames are layout widgets, not the reading column.
    if (archiveChild) return;
    const root = document.documentElement;
    if (width === "original") {
      widthState.style?.remove();
      widthState.style = null;
      widthState.width = "original";
      delete root.dataset.kiwixWidth;
      return;
    }
    if (!widthRules[width]) return;
    const mime = script?.dataset.entryMime || document.contentType || "";
    if (!widthCapableMime(mime)) return;
    if (!widthState.style?.isConnected) {
      widthState.style = document.createElement("style");
      widthState.style.dataset.kiwixWidthUi = "";
      (document.head || document.documentElement).append(widthState.style);
    }
    widthState.style.textContent = widthRules[width];
    widthState.width = width;
    root.dataset.kiwixWidth = width;
  }
  function themeRgb(value) {
    const match =
      /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+%?))?\s*\)$/i.exec(
        value || "",
      );
    if (!match) return null;
    const alpha =
      match[4] === undefined
        ? 1
        : match[4].endsWith("%")
          ? parseFloat(match[4]) / 100
          : parseFloat(match[4]);
    return [Number(match[1]), Number(match[2]), Number(match[3]), alpha];
  }
  function themeTone([red, green, blue]) {
    return {
      luminance: (0.2126 * red + 0.7152 * green + 0.0722 * blue) / 255,
      chroma: Math.max(red, green, blue) - Math.min(red, green, blue),
    };
  }
  function inheritedThemeBackdrop(element) {
    for (let node = element.parentElement; node; node = node.parentElement) {
      const known = themeState.backdrop.get(node);
      if (known) return known;
      if (node === document.body) return "themed";
    }
    return "themed";
  }
  function themeReadCandidate(element) {
    if (themeState.theme === "original" || !element.isConnected) return false;
    if (insideThemeOpaque(element)) return false;
    if (element === document.body) return true;
    const tag = element.tagName;
    return tag !== "HTML" && !themeExcluded.has(tag);
  }
  function themeDepth(element) {
    let depth = 0;
    for (let node = element; node; node = node.parentElement) depth++;
    return depth;
  }
  function clearThemeElementMarks(element) {
    delete element.dataset.kiwixSurface;
    delete element.dataset.kiwixInk;
    delete element.dataset.kiwixEdge;
  }
  // `getComputedStyle` forces style resolution, so reading and then writing one
  // element at a time invalidates style for every element in a long article.
  // The scanner resolves a whole batch first, applies its paint/ink marks, then
  // samples the batch's borders and applies edge marks. That costs a couple of
  // recalculations per batch instead of one per node.
  function themeReading(index) {
    let reading = themeState.readings[index];
    if (!reading) {
      reading = themeState.readings[index] = {
        element: null,
        backgroundColor: "",
        backgroundImage: "",
        color: "",
        surface: "",
        ink: false,
        borderTopColor: "",
        borderRightColor: "",
        borderBottomColor: "",
        borderLeftColor: "",
      };
    }
    return reading;
  }
  function readThemeElement(element, index) {
    const style = getComputedStyle(element);
    const reading = themeReading(index);
    reading.element = element;
    reading.backgroundColor = style.backgroundColor;
    reading.backgroundImage = style.backgroundImage;
    reading.color = style.color;
  }
  // Classification only reads the snapshot above; it writes nothing to the DOM,
  // so the whole batch can be decided before the first mark is applied.
  function classifyThemeReading(reading) {
    const element = reading.element;
    const dark = themeState.theme === "dark";
    let backdrop = inheritedThemeBackdrop(element);
    let surface = "";
    const background = themeRgb(reading.backgroundColor);
    if (background && background[3] >= 0.99) {
      if (reading.backgroundImage === "none") {
        const tone = themeTone(background);
        if (
          tone.chroma <= 16 &&
          (dark ? tone.luminance >= 0.8 : tone.luminance <= 0.2)
        ) {
          // Only opaque neutral surfaces are recoloured; a card's coloured
          // neighbours and artwork backgrounds keep their authored colours.
          surface = dark
            ? tone.luminance >= 0.96
              ? "page"
              : "raised"
            : tone.luminance <= 0.06
              ? "page"
              : "raised";
          backdrop = "themed";
        } else {
          backdrop = "preserved";
        }
      } else {
        backdrop = "preserved";
      }
    } else if (reading.backgroundImage !== "none") {
      backdrop = "preserved";
    }
    const ink = themeRgb(reading.color);
    let inkMark = false;
    if (ink && backdrop === "themed") {
      const tone = themeTone(ink);
      // Text over a preserved (coloured or image) backdrop keeps its colour.
      if (
        tone.chroma <= 24 &&
        (dark ? tone.luminance <= 0.5 : tone.luminance >= 0.75)
      )
        inkMark = true;
    }
    reading.surface = surface;
    reading.ink = inkMark;
    themeState.backdrop.set(element, backdrop);
  }
  function writeThemeMarks(reading) {
    if (reading.surface) reading.element.dataset.kiwixSurface = reading.surface;
    if (reading.ink) reading.element.dataset.kiwixInk = "";
  }
  // A default border colour is `currentColor`, so the archive-authored border
  // must be sampled after the ink mark has been applied, exactly as the
  // per-element scanner did; only surface elements were ever checked.
  function readThemeBorders(reading) {
    if (!reading.surface) return;
    const style = getComputedStyle(reading.element);
    reading.borderTopColor = style.borderTopColor;
    reading.borderRightColor = style.borderRightColor;
    reading.borderBottomColor = style.borderBottomColor;
    reading.borderLeftColor = style.borderLeftColor;
  }
  function writeThemeEdge(reading) {
    const element = reading.element;
    if (reading.surface) {
      const sides = [
        reading.borderTopColor,
        reading.borderRightColor,
        reading.borderBottomColor,
        reading.borderLeftColor,
      ].map(themeRgb);
      if (sides.every((side) => side && side[3] > 0.5)) {
        const tones = sides.map(themeTone);
        const dark = themeState.theme === "dark";
        if (
          tones.every((tone) => tone.chroma <= 16) &&
          tones.every((tone) => (dark ? tone.luminance >= 0.55 : tone.luminance <= 0.45))
        )
          element.dataset.kiwixEdge = "";
      }
    }
    reading.element = null;
  }
  function scheduleThemeScan() {
    if (themeState.timer || themeState.theme === "original") return;
    themeState.timer = setTimeout(scanThemeChunk, 16);
  }
  function scanThemeChunk() {
    themeState.timer = 0;
    const batch = themeState.batch;
    batch.length = 0;
    // Order the entire pending queue, not only this chunk. A descendant can be
    // enqueued by an earlier mutation callback while its ancestor arrives later;
    // chunk-local sorting would still classify the descendant against a stale
    // backdrop. Depth is strictly smaller for every proper ancestor.
    if (themeState.index < themeState.queue.length) {
      const pending = themeState.queue.slice(themeState.index);
      const depths = new Map();
      for (const element of pending) depths.set(element, themeDepth(element));
      pending.sort((left, right) => depths.get(left) - depths.get(right));
      themeState.queue.length = themeState.index;
      themeState.queue.push(...pending);
    }
    // The batch is element-count bound rather than time bound: each batch pays
    // a small, fixed number of style recalculations, and 256 elements keeps a
    // single chunk's read/write work close to one frame on the slowest WebView.
    while (
      themeState.index < themeState.queue.length &&
      batch.length < themeBatchLimit
    ) {
      const element = themeState.queue[themeState.index++];
      themeState.queued.delete(element);
      if (themeReadCandidate(element)) batch.push(element);
    }
    if (batch.length > 1) {
      // Mutation records can enqueue a descendant before its ancestor. Sort the
      // batch ancestor-first so a child never classifies against a stale
      // ancestor backdrop left by an earlier pass.
      const depths = new Map();
      for (const element of batch) depths.set(element, themeDepth(element));
      batch.sort((left, right) => depths.get(left) - depths.get(right));
    }
    if (batch.length) {
      // Write phase 1: drop the previous pass's marks before any style read, so
      // every read below observes the archive's authored colours.
      for (const element of batch)
        if (element !== document.body) clearThemeElementMarks(element);
      // Read phase 1: background, image and text colour, then classify in
      // memory (no DOM writes) so a parent's backdrop is ready for its children.
      for (let index = 0; index < batch.length; index++) {
        const element = batch[index];
        if (element === document.body) {
          themeState.backdrop.set(element, "themed");
          continue;
        }
        readThemeElement(element, index);
        classifyThemeReading(themeState.readings[index]);
      }
      // Write phase 2: surface and ink marks.
      for (let index = 0; index < batch.length; index++)
        if (batch[index] !== document.body)
          writeThemeMarks(themeState.readings[index]);
      // Read phase 2: border colours, now that `currentColor` follows the ink.
      for (let index = 0; index < batch.length; index++)
        if (batch[index] !== document.body)
          readThemeBorders(themeState.readings[index]);
      // Write phase 3: edge marks.
      for (let index = 0; index < batch.length; index++)
        if (batch[index] !== document.body)
          writeThemeEdge(themeState.readings[index]);
      batch.length = 0;
    }
    if (themeState.index < themeState.queue.length) {
      scheduleThemeScan();
    } else {
      themeState.queue.length = 0;
      themeState.index = 0;
    }
  }
  function enqueueThemeSubtree(root) {
    if (!root || root.nodeType !== Node.ELEMENT_NODE) return;
    const stack = [root];
    while (stack.length) {
      const element = stack.pop();
      if (
        !element ||
        element.nodeType !== Node.ELEMENT_NODE ||
        themeState.queued.has(element) ||
        insideThemeOpaque(element)
      )
        continue;
      themeState.queued.add(element);
      themeState.queue.push(element);
      const children = element.children;
      for (let index = children.length - 1; index >= 0; index--)
        stack.push(children[index]);
    }
    scheduleThemeScan();
  }
  function clearThemeMarks() {
    clearTimeout(themeState.timer);
    themeState.timer = 0;
    themeState.queue.length = 0;
    themeState.index = 0;
    themeState.batch.length = 0;
    themeState.queued = new WeakSet();
    themeState.backdrop = new WeakMap();
    for (const element of document.querySelectorAll(
      "[data-kiwix-surface],[data-kiwix-ink],[data-kiwix-edge]",
    ))
      clearThemeElementMarks(element);
  }
  function enqueueThemeTargets(targets) {
    if (!targets.length) return;
    if (targets.length > 1) {
      // A class/style mutation can be observed on a descendant before its
      // ancestor. Enqueue ancestors first so their fresh backdrop is already
      // available when the descendant is classified.
      const depths = new Map();
      for (const target of targets) depths.set(target, themeDepth(target));
      targets.sort((left, right) => depths.get(left) - depths.get(right));
    }
    for (const target of targets) enqueueThemeSubtree(target);
  }
  function observeTheme() {
    if (themeState.observer || typeof MutationObserver !== "function") return;
    themeState.observer = new MutationObserver((records) => {
      if (themeState.theme === "original") return;
      const targets = [];
      for (const record of records) {
        if (insideThemeOpaque(record.target)) continue;
        if (record.type === "childList") {
          for (const node of record.addedNodes)
            if (
              node.nodeType === Node.ELEMENT_NODE &&
              !insideThemeOpaque(node)
            )
              targets.push(node);
        } else if (record.target.nodeType === Node.ELEMENT_NODE) {
          targets.push(record.target);
        }
      }
      enqueueThemeTargets(targets);
    });
    themeState.observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ["class", "style"],
    });
  }
  function applyReaderTheme(theme) {
    const root = document.documentElement;
    const body = document.body;
    if (theme === "original") {
      if (themeState.observer) {
        themeState.observer.disconnect();
        themeState.observer = null;
      }
      clearThemeMarks();
      themeState.style?.remove();
      themeState.style = null;
      themeState.theme = "original";
      root.style.colorScheme = originalAppearance.colorScheme;
      root.style.backgroundColor = originalAppearance.rootBackground;
      delete root.dataset.kiwixTheme;
      if (body) {
        body.style.backgroundColor = originalAppearance.background;
        body.style.color = originalAppearance.color;
      }
      return;
    }
    const surfaces = themeSurfaces[theme];
    const changed = themeState.theme !== theme;
    if (changed) clearThemeMarks();
    themeState.theme = theme;
    if (!themeState.style?.isConnected) {
      themeState.style = document.createElement("style");
      themeState.style.dataset.kiwixThemeUi = "";
      themeState.style.textContent = themeStyleText();
      (document.head || document.documentElement).append(themeState.style);
    }
    root.dataset.kiwixTheme = theme;
    root.style.colorScheme = theme === "dark" ? "dark" : "light";
    root.style.backgroundColor = surfaces.background;
    if (body) {
      body.style.backgroundColor = surfaces.background;
      body.style.color = surfaces.text;
    }
    observeTheme();
    if (changed) {
      if (body) enqueueThemeSubtree(body);
      else
        document.addEventListener(
          "DOMContentLoaded",
          () => {
            if (themeState.theme !== "original" && document.body)
              enqueueThemeSubtree(document.body);
          },
          { once: true },
        );
    }
  }
  function applyReaderAppearance(message) {
    const root = document.documentElement;
    const body = document.body;
    originalAppearance ??= {
      colorScheme: root.style.colorScheme,
      fontFamily: originalFontFamily,
      background: body?.style.backgroundColor || "",
      color: body?.style.color || "",
      rootBackground: root.style.backgroundColor || "",
    };
    if (["original", ...themeNames].includes(message.theme))
      applyReaderTheme(message.theme);
    if (["original", "serif", "sans"].includes(message.fontFamily))
      root.style.fontFamily =
        message.fontFamily === "original"
          ? originalAppearance.fontFamily
          : message.fontFamily === "sans"
            ? "sans-serif"
            : "serif";
    if (Number.isFinite(message.fontSize))
      root.style.fontSize = `${Math.max(10, Math.min(48, message.fontSize))}px`;
    if (["original", "comfortable", "full"].includes(message.width))
      applyReaderWidth(message.width);
  }
  // Pre-paint appearance. The shell serves this bridge with the reader's current
  // theme and type, so the archive's first frame is already themed instead of
  // painting its own light background and default type until the post-`ready`
  // appearance message arrives. `original` keeps the archive's own colours.
  {
    const served = script?.dataset ?? {};
    const bootTheme = served.readerTheme;
    const bootFamily = served.readerFontFamily;
    if (["light", "sepia", "dark"].includes(bootTheme))
      originalAppearance ??= {
        colorScheme: document.documentElement.style.colorScheme,
        fontFamily: originalFontFamily,
        background: document.body?.style.backgroundColor || "",
        color: document.body?.style.color || "",
        rootBackground: document.documentElement.style.backgroundColor || "",
      };
    if (["light", "sepia", "dark"].includes(bootTheme))
      applyReaderTheme(bootTheme);
    if (["serif", "sans"].includes(bootFamily))
      document.documentElement.style.fontFamily =
        bootFamily === "sans" ? "sans-serif" : "serif";
    const bootSize = Number(served.readerFontSize);
    if (Number.isFinite(bootSize) && bootSize >= 10 && bootSize <= 48)
      document.documentElement.style.fontSize = `${bootSize}px`;
    const bootWidth = served.readerWidth;
    if (["comfortable", "full"].includes(bootWidth)) applyReaderWidth(bootWidth);
  }
  function normalizeSpeechSegments(segments) {
    const runs = [];
    const parts = [];
    let chunk = "";
    let length = 0;
    let lastCharacter = "";
    let pending = null;
    function flush() {
      if (chunk) { parts.push(chunk); chunk = ""; }
    }
    function emit(value, node, offset) {
      if (node) {
        const last = runs[runs.length - 1];
        if (last && last.node === node && last.end === length &&
            last.offset + (last.end - last.start) === offset)
          last.end += value.length;
        else
          runs.push({ start: length, end: length + value.length, node, offset });
      }
      chunk += value;
      length += value.length;
      lastCharacter = value[value.length - 1] || lastCharacter;
      if (chunk.length > 8192) flush();
    }
    function emitPending() {
      if (!pending) return;
      if (length > 0 && lastCharacter !== "\n") emit(" ", pending.node, pending.offset);
      pending = null;
    }
    for (const segment of segments) {
      if (!segment.node) {
        pending = null;
        if (length > 0 && lastCharacter !== "\n") emit("\n");
        continue;
      }
      const value = segment.text;
      for (let index = 0; index < value.length; index++) {
        const character = value[index];
        if (character === "\n") {
          pending = null;
          if (length > 0 && lastCharacter !== "\n") emit("\n");
        } else if (/\s/.test(character)) {
          if (!pending) pending = { node: segment.node, offset: segment.offset + index };
        } else {
          emitPending();
          emit(character, segment.node, segment.offset + index);
        }
      }
    }
    flush();
    let text = parts.join("");
    // `.trim()` equivalent: pending trailing spaces are already dropped, and a
    // trailing synthetic newline has no mapping run.
    if (text.endsWith("\n")) text = text.slice(0, -1);
    while (runs.length && runs[runs.length - 1].start >= text.length) runs.pop();
    return { text, runs };
  }
  function speechExtraction() {
    const root = document.body || document.documentElement;
    const segments = [];
    const blocks = new WeakMap();
    const hiddenText = new WeakSet();
    blocks.set(root, root);
    const rootStyle = getComputedStyle(root);
    if (rootStyle.display === "none" || rootStyle.contentVisibility === "hidden")
      return { text: "", runs: [] };
    if (rootStyle.visibility === "hidden" || rootStyle.visibility === "collapse") hiddenText.add(root);
    let previousBlock = null;
    let length = 0;
    let nodes = 0;
    let skippedTitle = false;
    let stopped = false;
    let emittedContent = false;
    let pageTitle = (document.title || "").split(/\s[-–—]\s/)[0].trim();
    function append(value, node, offset) {
      if (length + value.length > 1000000) throw new RangeError("SPEECH_TEXT_LIMIT");
      segments.push(node ? { text: value, node, offset } : { text: value });
      length += value.length;
    }
    const skipTags = new Set([
      "SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "CANVAS", "SVG", "HEAD", "HEADER",
      "NAV", "ASIDE", "FOOTER", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "LABEL",
      "IFRAME", "OBJECT", "EMBED",
    ]);
    const skipIds = new Set([
      "mw-navigation", "mw-head", "mw-panel", "toc", "siteSub", "jump-to-nav",
      "mw-indicators", "catlinks", "printfooter", "mw-data-after-content", "footer",
      "mw-page-base", "mw-head-base", "contentSub", "contentSub2", "mw-history-subheader",
      "mw-clearyourcache", "mw-message-box", "coordinates",
    ]);
    const skipClasses = new Set([
      "mw-editsection", "mw-jump-link", "mw-indicators", "mw-empty-elt", "mw-authority-control",
      "navbox", "navbox-styles", "vertical-navbox", "infobox", "sidebar", "toc", "reflist",
      "references", "mw-references-wrap", "reference", "hatnote", "rellink", "dablink",
      "metadata", "ambox", "mbox", "shortdescription", "portal", "sistersitebox", "noprint",
      "catlinks", "printfooter", "mw-hidden-catlinks", "mw-normal-catlinks", "mw-collapsible",
      "navigation-not-searchable", "mw-cite-backlink", "mw-reflink-text", "mw-ext-cite-error",
      "gallery", "thumb", "thumbinner", "thumbcaption", "magnify", "image",
    ]);
    const skipRoles = new Set([
      "navigation", "banner", "contentinfo", "complementary", "search", "menu", "menubar", "toolbar",
    ]);
    const stopHeadings = new Set([
      "contents", "see also", "references", "external links", "further reading", "notes",
      "bibliography", "citations", "sources", "footnotes", "gallery", "navigation",
      "works cited", "literature",
    ]);
    function classTokens(node) {
      return (node.getAttribute?.("class") || "").split(/\s+/).filter(Boolean);
    }
    function nodeText(node) {
      if (node.nodeType === 3) return node.nodeValue || "";
      let value = "";
      for (const child of node.childNodes || node.children || []) value += nodeText(child);
      return value;
    }
    function headingText(node) {
      return nodeText(node).replace(/\[\s*edit\s*\]/gi, "").replace(/\s+/g, " ").trim().toLowerCase();
    }
    function skipElement(node) {
      if (skipTags.has(node.tagName)) return true;
      const id = node.getAttribute?.("id") || "";
      if (skipIds.has(id)) return true;
      const role = node.getAttribute?.("role") || "";
      if (skipRoles.has(role)) return true;
      const classes = classTokens(node);
      if (classes.some((name) => skipClasses.has(name))) return true;
      if (node.tagName === "H1" && !skippedTitle) {
        skippedTitle = true;
        if (!pageTitle) pageTitle = nodeText(node).replace(/\s+/g, " ").trim();
        return true;
      }
      if (/^H[2-6]$/.test(node.tagName)) {
        const heading = headingText(node);
        if (stopHeadings.has(heading)) { stopped = true; return true; }
      }
      return false;
    }
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        if (++nodes > 100000) throw new RangeError("SPEECH_TEXT_LIMIT");
        if (stopped) return NodeFilter.FILTER_REJECT;
        if (node.nodeType === Node.ELEMENT_NODE) {
          if (node.hidden || node.getAttribute("aria-hidden") === "true" || skipElement(node))
            return NodeFilter.FILTER_REJECT;
          const style = getComputedStyle(node);
          if (style.display === "none" || style.contentVisibility === "hidden") return NodeFilter.FILTER_REJECT;
          const block = style.display.startsWith("inline") || style.display === "contents"
            ? blocks.get(node.parentElement) || root : node;
          blocks.set(node, block);
          const invisible = style.visibility === "hidden" || style.visibility === "collapse";
          if (invisible) hiddenText.add(node);
          else if (node.tagName === "BR") append("\n");
          return NodeFilter.FILTER_SKIP;
        }
        if (hiddenText.has(node.parentElement)) return NodeFilter.FILTER_REJECT;
        if (!emittedContent && pageTitle &&
            (node.parentElement?.tagName === "B" || node.parentElement?.tagName === "STRONG") &&
            nodeText(node.parentElement).replace(/\s+/g, " ").trim() === pageTitle)
          return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const block = blocks.get(node.parentElement) || root;
      if (previousBlock && block !== previousBlock) append("\n");
      append(node.nodeValue || "", node, 0);
      if ((node.nodeValue || "").trim()) emittedContent = true;
      previousBlock = block;
    }
    return normalizeSpeechSegments(segments);
  }
  function nativeSpeechText(requestId) {
    if (typeof requestId !== "string" || !/^[0-9a-f-]{36}$/.test(requestId)) return;
    try {
      const extraction = speechExtraction();
      try { setReadAloudDocument(extraction); } catch (_) { /* paint only */ }
      post("speech-text", { requestId, text: extraction.text });
    } catch (error) {
      try { setReadAloudDocument(null); } catch (_) { /* paint only */ }
      post("speech-text", { requestId,
        error: error instanceof RangeError && error.message === "SPEECH_TEXT_LIMIT"
          ? "SPEECH_TEXT_LIMIT" : "SPEECH_TEXT_UNAVAILABLE" });
    }
  }
  async function exportSnapshot(message) {
    if (typeof message.requestId !== "string" || !/^[0-9a-f-]{36}$/.test(message.requestId) ||
        message.url !== location.href) return;
    const documentUrl = location.href;
    const requestSession = sessionId;
    const limit = 16 * 1024 * 1024;
    let resourceBytes = 0;
    let nodes = 0;
    const resources = new Map();
    const capturedDocuments = new Map();
    const capturedFrames = [];
    const capturedShadows = [];
    const slotOwners = new Map();
    const fonts = [];
    const styleSurfaces = new Map([
      ["", { prefix: "kb", chunks: [] }],
      ["::before", { prefix: "kp", chunks: [] }],
      ["::after", { prefix: "ka", chunks: [] }],
    ]);
    let styleChunkCount = 0;
    async function collectDocument(owner) {
      if (capturedDocuments.has(owner)) return;
      const entry = { url: owner.location?.href || owner.baseURI,
        prefix: `kiwix-document-${capturedDocuments.size}`, families: new Map() };
      capturedDocuments.set(owner, entry);
      const sheets = new Set();
      async function sheetRules(sheet) {
        if (sheets.has(sheet)) return;
        sheets.add(sheet);
        let items;
        try {
          items = sheet.cssRules;
        } catch (error) {
          // WebView2 refuses a same-origin archive stylesheet whose entry
          // declares a non-CSS type (seen: application/octet-stream), so the
          // live page is not styled by it either and it has no readable
          // rules to capture. The archive server only redirects within the
          // archive origin, so a same-origin href cannot hide a foreign
          // sheet. Any other unreadable sheet still fails the export.
          const href = sheet.href ? new URL(sheet.href, owner.baseURI) : null;
          if (error?.name === "SecurityError" && href?.origin === location.origin) return;
          throw error;
        }
        await rules(items, sheet.href || owner.baseURI);
      }
      async function rules(items, base) {
        for (const rule of items) {
          if (rule.type === 5) {
            const family = rule.style.getPropertyValue("font-family").trim().replace(/^(['"])(.*)\1$/, "$2");
            if (!entry.families.has(family))
              entry.families.set(family, `kiwix-export-${capturedDocuments.size}-${entry.families.size}`);
            let css = "";
            for (let index = 0; index < rule.style.length; index++) {
              const name = rule.style[index];
              css += `${name}:${name === "font-family"
                ? `"${entry.families.get(family)}"`
                : await cssResources(rule.style.getPropertyValue(name), owner, base)};`;
            }
            fonts.push(`@font-face{${css}}`);
          } else if (rule.styleSheet) await sheetRules(rule.styleSheet);
          else if (rule.cssRules) await rules(rule.cssRules, base);
        }
      }
      for (const sheet of [...owner.styleSheets, ...(owner.adoptedStyleSheets || [])])
        await sheetRules(sheet);
    }
    async function resource(value, owner = document, base = owner.baseURI) {
      const url = new URL(value, base);
      if (url.protocol === "data:") return url.href;
      if (url.origin !== location.origin || !["http:", "https:", "blob:"].includes(url.protocol))
        throw new Error("The live page contains a resource outside this archive.");
      if (!resources.has(url.href)) resources.set(url.href, (async () => {
        const response = await fetch(url.href, { credentials: "omit", redirect: "error" });
        if (!response.ok) throw new Error("A live-page resource could not be read.");
        const reader = response.body?.getReader();
        if (!reader) throw new Error("A live-page resource could not be read.");
        const parts = [];
        try {
          while (true) {
            const { done, value: bytes } = await reader.read();
            if (done) break;
            resourceBytes += bytes.length;
            if (resourceBytes > limit) {
              await reader.cancel();
              throw new Error("The live page is too large to export.");
            }
            for (let index = 0; index < bytes.length; index += 8192)
              parts.push(String.fromCharCode(...bytes.subarray(index, index + 8192)));
          }
        } finally { reader.releaseLock(); }
        const binary = parts.join("");
        return `data:${(response.headers.get("content-type") || "application/octet-stream").split(";")[0]};base64,${btoa(binary)}`;
      })());
      return resources.get(url.href);
    }
    function localFragment(value, owner, base = owner.baseURI) {
      const target = new URL(value, base);
      const identity = new URL(capturedDocuments.get(owner).url);
      const fragment = target.hash;
      target.hash = "";
      identity.hash = "";
      return fragment && (value.startsWith("#") || target.href === identity.href)
        ? `#${capturedDocuments.get(owner).prefix}-${fragment.slice(1)}` : null;
    }
    async function cssResources(value, owner = document, base = owner.baseURI) {
      const matches = Array.from(value.matchAll(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/gi));
      for (const match of matches) {
        const path = (match[1] ?? match[2] ?? match[3]).trim();
        const fragment = localFragment(path, owner, base);
        if (fragment) {
          value = value.replace(match[0], `url("${fragment}")`);
          continue;
        }
        value = value.replace(match[0], `url("${await resource(path, owner, base)}")`);
      }
      return value;
    }
    function liveStyle(element, owner, pseudo) {
      return owner.defaultView ? owner.defaultView.getComputedStyle(element, pseudo) : getComputedStyle(element, pseudo);
    }
    async function computedStyle(element, owner, pseudo) {
      const style = liveStyle(element, owner, pseudo);
      const chunks = [];
      let css = "";
      for (let index = 0; index < style.length; index++) {
        const name = style[index];
        let value = style.getPropertyValue(name);
        if (name === "font-family") {
          const families = capturedDocuments.get(owner).families;
          value = (value.match(/"[^"]*"|'[^']*'|[^,]+/g) || []).map((part) => {
            const family = part.trim().replace(/^(['"])(.*)\1$/, "$2");
            return families.has(family) ? `"${families.get(family)}"` : part;
          }).join(",");
        }
        css += `${name}:${await cssResources(value, owner)};`;
        if ((index + 1) % 8 === 0) {
          chunks.push(css);
          css = "";
        }
      }
      if (css) chunks.push(css);
      return chunks;
    }
    function shareStyle(result, chunks, pseudo = "") {
      const surface = styleSurfaces.get(pseudo);
      const tokens = chunks.map((css, index) => {
        const shared = surface.chunks[index] || (surface.chunks[index] = new Map());
        if (!shared.has(css)) shared.set(css, `${surface.prefix}${styleChunkCount++}`);
        return shared.get(css);
      });
      if (!pseudo) result.removeAttribute("style");
      // Original classes have no remaining stylesheet authority. Replacing
      // them prevents collisions with generated rules. Pseudo content is
      // resolved on the untouched original element.
      result.setAttribute("class", (pseudo ? `${result.getAttribute("class")} ` : "") + tokens.join(" "));
    }
    function layoutParent(element) {
      return element.assignedSlot || element.parentElement || element.getRootNode?.()?.host || null;
    }
    function freezeViewportPosition(node, result, owner) {
      const style = liveStyle(node, owner);
      if (!["fixed", "sticky"].includes(style.position)) return result;
      const view = owner.defaultView || window;
      const rect = node.getBoundingClientRect();
      const width = node.offsetWidth || rect.width;
      const height = node.offsetHeight || rect.height;
      let left = rect.left;
      let top = rect.top;
      let transformLeft = 0;
      let transformTop = 0;
      // DOMRect includes the element's transform. Keep that transform, but
      // remove its local bounding-box displacement before assigning layout
      // coordinates, including the common centered translate(-50%) toolbar.
      if (style.transform && style.transform !== "none") {
        const Matrix = view.DOMMatrixReadOnly || DOMMatrixReadOnly;
        const matrix = new Matrix(style.transform);
        const origin = style.transformOrigin.split(/\s+/).map(Number.parseFloat);
        const x = origin[0] || 0;
        const y = origin[1] || 0;
        const corners = [[0, 0], [width, 0], [0, height], [width, height]].map(([a, b]) => ({
          x: matrix.a * (a - x) + matrix.c * (b - y) + matrix.e + x,
          y: matrix.b * (a - x) + matrix.d * (b - y) + matrix.f + y,
        }));
        transformLeft = Math.min(...corners.map((point) => point.x));
        transformTop = Math.min(...corners.map((point) => point.y));
      }
      let container = layoutParent(node);
      while (container && container !== owner.documentElement) {
        const ancestor = liveStyle(container, owner);
        if ((ancestor.position && ancestor.position !== "static") ||
            (ancestor.transform && ancestor.transform !== "none") ||
            (ancestor.perspective && ancestor.perspective !== "none") ||
            (ancestor.filter && ancestor.filter !== "none") ||
            /\b(layout|paint|strict|content)\b/.test(ancestor.contain || "")) break;
        container = layoutParent(container);
      }
      if (container && container !== owner.documentElement) {
        const bounds = container.getBoundingClientRect();
        const scaleX = container.offsetWidth ? bounds.width / container.offsetWidth : 1;
        const scaleY = container.offsetHeight ? bounds.height / container.offsetHeight : 1;
        left = (left - bounds.left) / (scaleX || 1) - (container.clientLeft || 0) + (container.scrollLeft || 0);
        top = (top - bounds.top) / (scaleY || 1) - (container.clientTop || 0) + (container.scrollTop || 0);
      } else {
        left += view.scrollX || 0;
        top += view.scrollY || 0;
      }
      left -= transformLeft;
      top -= transformTop;
      let placeholder;
      if (style.position === "sticky") {
        placeholder = result.cloneNode(false);
        for (const attribute of Array.from(placeholder.attributes))
          if (!["rowspan", "colspan", "span"].includes(attribute.name))
            placeholder.removeAttribute(attribute.name);
        placeholder.setAttribute("class", result.getAttribute("class").split(" ")
          .filter((token) => token.startsWith("kb")).join(" "));
        if (result.hasAttribute("style"))
          placeholder.setAttribute("style", result.getAttribute("style"));
        for (const [name, value] of Object.entries({
          position: "static", inset: "auto", transform: "none", opacity: "0",
          "pointer-events": "none", "box-sizing": "border-box",
          width: `${width}px`, height: `${height}px`,
        })) placeholder.style.setProperty(name, value, "important");
      }
      for (const [name, value] of Object.entries({
        position: "absolute", left: `${left}px`, top: `${top}px`,
        right: "auto", bottom: "auto", margin: "0", "box-sizing": "border-box",
        width: `${width}px`, height: `${height}px`,
      })) result.style.setProperty(name, value, "important");
      if (!placeholder) return result;
      const fragment = document.createDocumentFragment();
      fragment.appendChild(placeholder);
      fragment.appendChild(result);
      return fragment;
    }
    async function clone(node, owner = document, depth = 0) {
      if (++nodes > 100000) throw new Error("The live page is too large to export.");
      if (node.nodeType === Node.TEXT_NODE) return document.createTextNode(node.nodeValue || "");
      if (node.nodeType !== Node.ELEMENT_NODE) return null;
      const tag = node.tagName;
      if (["SCRIPT", "STYLE", "LINK", "BASE", "META", "NOSCRIPT", "TEMPLATE"].includes(tag) ||
          node.hasAttribute("data-kiwix-overlay")) return null;
      if (owner !== document && tag === "HEAD") return null;
      const shadow = node.shadowRoot;
      const custom = (!node.namespaceURI || node.namespaceURI === "http://www.w3.org/1999/xhtml") &&
        (node.localName.includes("-") || node.hasAttribute("is"));
      if (custom && !shadow) {
        // DevDocs pages carry an empty, never-registered <devdocs-navbar>.
        // An undefined, childless *inline* element with a 0x0 box has no
        // in-flow content (inline ignores width/height, unlike block or
        // contents), so it is left out. Accepted residual: out-of-flow
        // content (absolute, fixed, float) inside a closed shadow root that a
        // script or declarative template attached to such a host cannot be
        // detected from page script and would be missing from the export.
        // Everything else that could render unreadable content fails.
        const box = node.getBoundingClientRect?.();
        const inert = node.matches?.(":defined") === false && !node.childNodes.length &&
          liveStyle(node, owner).display === "inline" && box && !box.width && !box.height;
        if (inert) return null;
        throw new Error("A custom rendered surface has no readable shadow DOM.");
      }
      let result;
      if (["IFRAME", "FRAME", "OBJECT", "EMBED"].includes(tag)) {
        const child = node.contentDocument;
        const view = child?.defaultView;
        if (!child?.documentElement || !view || depth >= 32 ||
            (view.origin || view.location.origin) !== location.origin)
          throw new Error("An embedded live document could not be accessed.");
        const identity = { frame: node, document: child, url: view.location.href };
        capturedFrames.push(identity);
        await collectDocument(child);
        const width = Math.round(view.innerWidth);
        const height = Math.round(view.innerHeight);
        const scrollX = view.scrollX;
        const scrollY = view.scrollY;
        if (width < 1 || width > 16384 || height < 1 || height > 10000000)
          throw new Error("The embedded live document is too large to export.");
        result = document.createElement("div");
        const viewport = document.createElement("div");
        viewport.setAttribute("style", `position:relative;overflow:hidden;isolation:isolate;transform:translate(0px,0px);width:${width}px;height:${height}px;`);
        const content = await clone(child.documentElement, child, depth + 1);
        content.style.setProperty("position", "relative", "important");
        content.style.setProperty("left", `${-scrollX}px`, "important");
        content.style.setProperty("top", `${-scrollY}px`, "important");
        content.style.setProperty("width", `${width}px`, "important");
        viewport.appendChild(content);
        result.appendChild(viewport);
      } else if (tag === "AUDIO") {
        const style = liveStyle(node, owner);
        // Audio has no paintable frame. Non-rendering elements contribute no
        // page surface; visible controls become an inert, source-free player.
        if (style.display === "none" || ["hidden", "collapse"].includes(style.visibility) ||
            !Array.from(node.getClientRects()).some((rect) => rect.width > 0 && rect.height > 0)) return null;
        const time = (seconds) => {
          if (!Number.isFinite(seconds) || seconds < 0) return "--:--";
          const whole = Math.floor(seconds);
          return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
        };
        result = document.createElement("div");
        result.setAttribute("role", "img");
        result.setAttribute("aria-label", node.getAttribute("aria-label") || node.title || "Audio player");
        result.textContent = `${node.paused ? "\u25b6" : "\u275a\u275a"} ${time(node.currentTime)} / ${time(node.duration)}`;
      } else if (tag === "CANVAS" || tag === "VIDEO") {
        result = document.createElement("img");
        if (tag === "CANVAS") result.src = node.toDataURL("image/png");
        else if (node.readyState >= 2) {
          const canvas = document.createElement("canvas");
          canvas.width = node.videoWidth;
          canvas.height = node.videoHeight;
          canvas.getContext("2d").drawImage(node, 0, 0);
          result.src = canvas.toDataURL("image/png");
        } else if (node.poster) result.src = await resource(node.poster, owner);
        else throw new Error("The video has no visible frame to export.");
        result.width = node.clientWidth;
        result.height = node.clientHeight;
      } else {
        const flattened = shadow || tag === "SLOT" ||
          (owner !== document && ["HTML", "BODY"].includes(tag));
        result = flattened ? document.createElement("div") : node.cloneNode(false);
        if (flattened)
          for (const attribute of Array.from(node.attributes))
            result.setAttribute(attribute.name, attribute.value);
        for (const attribute of Array.from(result.attributes)) {
          if (/^on/i.test(attribute.name) ||
              ["src", "srcset", "href", "action", "formaction", "ping", "autoplay", "nonce",
                "integrity", "crossorigin", "contenteditable"].includes(attribute.name.toLowerCase()))
            result.removeAttribute(attribute.name);
        }
        if (result.hasAttribute("id"))
          result.setAttribute("id", `${capturedDocuments.get(owner).prefix}-${result.getAttribute("id")}`);
        if (tag === "IMG" && (node.currentSrc || node.src))
          result.setAttribute("src", await resource(node.currentSrc || node.src, owner));
        if (["image", "use"].includes(node.localName)) {
          const href = node.getAttribute("href") || node.getAttribute("xlink:href");
          if (href) result.setAttribute("href", localFragment(href, owner) || await resource(href, owner));
          result.removeAttribute("xlink:href");
        }
        if (tag === "INPUT") {
          result.setAttribute("value", node.type === "password" ? "\u2022".repeat(node.value.length) : node.value);
          result.toggleAttribute("checked", node.checked);
        } else if (tag === "TEXTAREA") result.textContent = node.value;
        else {
          let children = node.childNodes;
          let childOwner = owner;
          let childDepth = depth;
          if (shadow) {
            if (depth >= 32) throw new Error("The live page is too deeply nested to export.");
            capturedShadows.push({ host: node, root: shadow });
            childOwner = {
              baseURI: owner.baseURI, location: owner.location,
              defaultView: owner.defaultView, documentElement: owner.documentElement,
              styleSheets: Array.from(shadow.styleSheets || []),
              adoptedStyleSheets: Array.from(shadow.adoptedStyleSheets || []),
            };
            slotOwners.set(childOwner, owner);
            await collectDocument(childOwner);
            children = shadow.childNodes;
            childDepth++;
          } else if (tag === "SLOT" && typeof node.assignedNodes === "function") {
            const assigned = node.assignedNodes({ flatten: true });
            if (assigned.length) {
              children = assigned;
              childOwner = slotOwners.get(owner) || owner;
            }
          }
          for (const child of children) {
            const copy = await clone(child, childOwner, childDepth);
            if (copy) result.appendChild(copy);
          }
        }
        if (tag === "OPTION") result.toggleAttribute("selected", node.selected);
        if (tag === "DETAILS") result.toggleAttribute("open", node.open);
      }
      shareStyle(result, await computedStyle(node, owner));
      if (tag === "AUDIO") {
        result.style.setProperty("display", "inline-flex", "important");
        result.style.setProperty("align-items", "center", "important");
        result.style.setProperty("box-sizing", "border-box", "important");
        result.style.setProperty("width", `${node.offsetWidth}px`, "important");
        result.style.setProperty("height", `${node.offsetHeight}px`, "important");
      }
      if (["IFRAME", "FRAME", "OBJECT", "EMBED"].includes(tag)) {
        result.style.setProperty("overflow", "hidden", "important");
        if (liveStyle(node, owner).display === "inline")
          result.style.setProperty("display", "inline-block", "important");
      }
      for (const pseudo of ["::before", "::after"]) {
        const content = liveStyle(node, owner, pseudo).content;
        if (content && content !== "none" && content !== "normal")
          shareStyle(result, await computedStyle(node, owner, pseudo), pseudo);
      }
      return freezeViewportPosition(node, result, owner);
    }
    try {
      await collectDocument(document);
      const root = await clone(document.documentElement);
      const style = document.createElement("style");
      // Equal-specificity chunks stay in their original declaration order,
      // including overlapping logical/physical properties. Only duplication
      // is removed; per-element geometry overrides remain inline.
      const sharedRules = Array.from(styleSurfaces, ([pseudo, surface]) =>
        surface.chunks.flatMap((chunks) => Array.from(chunks,
          ([css, token]) => `.${token}${pseudo}{${css}}`))).flat();
      style.textContent = (fonts.join("\n") + sharedRules.join("\n") +
        "*{animation:none!important;transition:none!important;caret-color:transparent!important}")
        .replaceAll("<", "\\3c ");
      (root.querySelector("head") || root).appendChild(style);
      const width = Math.round(document.documentElement.clientWidth);
      const height = Math.max(document.documentElement.scrollHeight, document.body?.scrollHeight || 0);
      root.style.setProperty("width", `${width}px`, "important");
      const html = "<!doctype html>\n" + root.outerHTML;
      if (html.length > limit || new TextEncoder().encode(html).length > limit ||
          width < 1 || width > 16384 || height < 1 || height > 10000000)
        throw new Error("The live page is too large to export.");
      for (const identity of capturedFrames)
        if (identity.frame.contentDocument !== identity.document ||
            identity.document.defaultView?.location.href !== identity.url)
          throw new Error("An embedded live document changed during export.");
      for (const identity of capturedShadows)
        if (identity.host.shadowRoot !== identity.root)
          throw new Error("A rendered shadow surface changed during export.");
      if (location.href !== documentUrl || sessionId !== requestSession) return;
      post("export-snapshot", { requestId: message.requestId, url: documentUrl,
        snapshot: { html, width, height, documentUrl } });
    } catch (error) {
      if (location.href === documentUrl && sessionId === requestSession)
        post("export-snapshot", { requestId: message.requestId, url: documentUrl,
          error: error instanceof Error ? error.message : "The live page could not be captured." });
    }
  }
  function command(message) {
    if (["scroll", "toc", "find", "zoom", "appearance", "expand-sections",
        "highlight-jump", "history-traverse"].includes(message.command))
      cancelReadingPan();
    switch (message.command) {
      case "reading-controls":
        readingControlsEnabled = message.enabled === true;
        if (Number.isFinite(message.minimumScroll) && message.minimumScroll >= 2 && message.minimumScroll <= 4096)
          readingControlsMinimumScroll = message.minimumScroll;
        // Host-side reveal must clear the bridge's hidden flag too; otherwise
        // the next downward pan is ignored as a duplicate hide.
        resetReadingPan();
        break;
      case "reading-viewport": {
        if (message.url !== location.href || !Number.isFinite(message.deltaTop) ||
            Math.abs(message.deltaTop) > 4096) break;
        const before = readingScrollY();
        if (message.deltaTop > 0 && before <= 2) break;
        scrollReadingViewport?.(0, message.deltaTop);
        if (readingTouch) {
          readingTouch.scrollY += readingScrollY() - before;
          readingTouch.anchorY -= message.deltaTop;
          readingTouch.y -= message.deltaTop;
        }
        break;
      }
      case "export-snapshot":
        void exportSnapshot(message);
        break;
      case "highlights-begin":
        if (
          typeof message.url !== "string" ||
          message.url !== location.href ||
          typeof message.documentPath !== "string" ||
          !message.documentPath ||
          message.documentPath.length > 16384
        )
          break;
        annotationDocument = documentKey(message.documentPath);
        annotationUrl = message.url;
        annotationReceiving = true;
        annotationItems = [];
        stopAnnotationObservation();
        annotationRanges = [];
        paintAnnotations();
        break;
      case "highlights":
        if (
          !annotationReceiving ||
          message.url !== annotationUrl ||
          message.url !== location.href ||
          !Array.isArray(message.highlights) ||
          message.highlights.length > 200 ||
          annotationItems.length + message.highlights.length >
            maxDocumentHighlights ||
          !message.highlights.every(
            (item) =>
              validAnnotation(item) &&
              documentKey(item.documentPath) === annotationDocument,
          )
        )
          break;
        annotationItems.push(...message.highlights);
        break;
      case "highlights-end":
        if (
          !annotationReceiving ||
          message.url !== annotationUrl ||
          message.url !== location.href
        )
          break;
        annotationReceiving = false;
        updateAnnotationObservation();
        renderAnnotations();
        break;
      case "highlight-jump": {
        if (
          annotationReceiving ||
          message.url !== location.href ||
          typeof message.bookmarkId !== "string" ||
          !uuid.test(message.bookmarkId) ||
          typeof message.id !== "string" ||
          !uuid.test(message.id)
        )
          break;
        // Re-resolve immediately: ranges may have drifted since the last render.
        renderAnnotations();
        const entry = annotationRanges.find(
          (item) =>
            item.id === message.id && item.bookmarkId === message.bookmarkId,
        );
        if (!entry) break;
        const element = entry.range.startContainer.parentElement;
        if (element) {
          reveal(element);
          element.scrollIntoView({ block: "center" });
          const rect = entry.range.getBoundingClientRect();
          window.scrollBy(0, rect.top - innerHeight / 3);
          scheduleAnnotationPaint();
        }
        break;
      }
      case "link-previews":
        if (typeof message.enabled === "boolean") setLinkPreviews(message.enabled);
        break;
      case "expand-sections":
        revealSearchableContent();
        break;
      case "find":
        if (typeof message.requestId !== "string" || !uuid.test(message.requestId) ||
            message.url !== location.href || typeof message.query !== "string" ||
            !message.query || message.query.length > 2000) break;
        revealSearchableContent();
        post("find-result", {
          requestId: message.requestId,
          query: message.query,
          url: location.href,
          found: Boolean(
            window.find(
              message.query,
              false,
              Boolean(message.backwards),
              true,
            ),
          ),
        });
        break;
      case "scroll":
        window.scrollTo(Number(message.x) || 0, Number(message.y) || 0);
        break;
      case "print":
        window.print();
        break;
      case "zoom": {
        const value = Number(message.value);
        if (Number.isFinite(value))
          document.documentElement.style.zoom = String(
            Math.max(0.25, Math.min(5, value)),
          );
        break;
      }
      case "appearance":
        applyReaderAppearance(message);
        break;
      case "speech-text":
        nativeSpeechText(message.requestId);
        break;
      case "speak": {
        let text = typeof message.text === "string" ? message.text : "";
        let mapping = null;
        if (text) {
          try {
            if (Number.isSafeInteger(message.start) && Number.isSafeInteger(message.end))
              mapping = textIndexMapping(text, message.start, message.end);
            if (!mapping) mapping = selectionSpeechMapping(text);
            if (!mapping) mapping = textSearchMapping(text);
          } catch (_) { mapping = null; }
        } else {
          try {
            const extraction = speechExtraction();
            text = extraction.text;
            mapping = extraction;
          } catch (_) {
            text = document.body?.innerText || "";
          }
        }
        speak(String(text), message, mapping);
        break;
      }
      case "speech-preferences": {
        // Never start stopped reading. Ignore cancelled utterances' late callbacks.
        const speech = activeSpeech;
        if (speech && (speechSynthesis.speaking || speechSynthesis.pending)) {
          const mapping = speech.state
            ? { ...speech.state, base: speech.base + speech.offset }
            : null;
          speak(speech.spoken.slice(speech.offset), message, mapping);
        }
        break;
      }
      case "speech-highlight": {
        if (message.clear === true) {
          try { clearReadAloudPaint(); } catch (_) { /* paint only */ }
          break;
        }
        const documentState = readAloudDocument;
        if (!documentState || !Number.isSafeInteger(message.offset) || message.offset < 0 ||
            message.offset > documentState.text.length) break;
        highlightReadAloud(message.offset);
        break;
      }
      case "speech-highlight-selection": {
        if (typeof message.text !== "string" || !message.text || message.text.length > 120000) break;
        let mapping = null;
        try {
          if (Number.isSafeInteger(message.start) && Number.isSafeInteger(message.end))
            mapping = textIndexMapping(message.text, message.start, message.end);
          if (!mapping) mapping = selectionSpeechMapping(message.text);
          if (!mapping) mapping = textSearchMapping(message.text);
        } catch (_) { mapping = null; }
        try { setReadAloudDocument(mapping); } catch (_) { /* paint only */ }
        break;
      }
      case "stop-speech":
        stopSpeech();
        resetReadAloud();
        break;
      case "pause-video":
        pauseVideosUnlessPictureInPicture();
        break;
      case "pause-media":
        pauseAllMedia();
        break;
      case "restore-media":
        if (message.url === location.href) restoreMediaPositions(message.requestId, message.positions);
        break;
      case "toc":
        if (typeof message.id === "string") navigateToHeading(message.id);
        break;
      case "history-traverse":
        traverseDocumentHistory(message);
        break;
    }
  }

  document.addEventListener("keydown", (event) => {
    if (!event.isTrusted) return;
    resetReadingPan();
    if (event.key === "Escape") {
      // A pending link-preview dwell must not reopen chrome after dismissal.
      clearTimeout(previewTimer);
      previewAnchor = null;
      post("dismiss-reader-ui", {});
    }
  });

  window.addEventListener("message", (event) => {
    if (!event.isTrusted || event.source !== parent || event.origin !== parentOrigin) return;
    const message = event.data;
    if (!message || message.channel !== channel || message.version !== 1)
      return;
    if (
      message.type === "reader-init" &&
      typeof message.sessionId === "string" &&
      message.sessionId
    ) {
      if (sessionId && sessionId !== message.sessionId) {
        resetDocumentHistory();
        stopSpeech();
        resetReadAloud();
        pauseAllMedia();
        retireMediaChildren();
        for (const frame of document.querySelectorAll("iframe,frame"))
          if (childFrame(frame.contentWindow)) mediaPost(frame.contentWindow, "probe", "");
        revoke("camera");
        revoke("location");
      }
      if (sessionId !== message.sessionId) observedLocationPermission = "prompt";
      sessionId = message.sessionId;
      // Desktop and not restricted: offer the app's photo chooser. A new
      // session never inherits a request made by the previous one.
      photoChooser = message.photoChooser === true;
      photoSecret = typeof message.photoSecret === "string" ? message.photoSecret : "";
      clearPhotoPending();
      // Archive scripts never see reader-init: it carries the session and the
      // photo secret. This listener was registered before any archive script.
      try { photoNatives?.stopImmediatePropagation(event); } catch (_) { /* Test doubles. */ }
      // Capture listener order matters: the bridge responds before any archive
      // listener can observe this fresh session. The shell pins only its first
      // proof; later archive-authored bind/pan messages cannot replace it.
      post("reading-pan-bind", { token: readingPanToken });
      resetReadingPan();
      setLinkPreviews(message.linkPreviews === true);
      if (!mediaReady) mediaRetired = false;
      mediaReady = true;
      reportMediaState(true);
      reportMediaCheckpoint();
      acceptSensorStatus(message.sensorStatus);
      acceptMediaNotice(message.mediaNotice);
      initializePermissions();
      if (document.readyState === "loading")
        document.addEventListener("DOMContentLoaded", ready, { once: true });
      else ready();
      for (const entry of pending.values())
        post("sensor-request", entry.message);
      return;
    }
    if (!sessionId || message.sessionId !== sessionId) return;
    if (message.type === "document-probe") {
      // Some engines fire the embedding frame's load event for a same-document
      // (fragment) navigation. Only this bridge instance, still holding the
      // probed session, can answer with its private bootstrap token, so the
      // shell can tell that case from a replaced document without trusting
      // anything an archive script could send.
      if (typeof message.nonce === "string" && uuid.test(message.nonce))
        post("document-proof", { nonce: message.nonce, token: readingPanToken });
      return;
    }
    if (message.type === "photo-result" || message.type === "photo-cancel" || message.type === "photo-accepted") {
      photoResult(message);
      return;
    }
    if (message.type === "sensor-response") {
      const entry = pending.get(message.requestId);
      if (!entry) return;
      pending.delete(message.requestId);
      clearTimeout(entry.timer);
      acceptSensorStatus(message.sensorStatus);
      if (nativeLocation && entry.sensor === "location") {
        // A fresh fix proves both grants; acquisition failure/timeout proves
        // neither a denial nor a grant. Never turn OS approval into archive consent.
        if (!message.allowed && message.cancelled === true && message.error?.code === 2)
          observedLocationPermission = "prompt";
        else if (message.allowed && message.position) observedLocationPermission = "granted";
        else if (message.error?.code === 1) observedLocationPermission = "denied";
        updateLocationPermission();
      }
      entry.resolve(message);
    } else if (message.type === "sensor-revoke") {
      if (message.sensor === "camera" || message.sensor === "location") {
        if (message.sensor === "location") {
          observedLocationPermission = "prompt";
          if (["notAsked", "denied"].includes(message.decision))
            archiveLocationDecision = message.decision;
          updateLocationPermission();
        }
        revoke(message.sensor);
      }
    } else if (message.type === "reader-command") command(message);
  }, true);
  // The shell and the native opener accept only http, https and mailto. Every
  // other scheme (tel:, sms:, geo:, data:, ...) stays blocked here; a genuine
  // activation is answered with a quiet shell notice instead of nothing at all.
  // javascript: pseudo-links are page controls, not destinations.
  function reportUnsupportedLink(event, url) {
    if (event.isTrusted && url.protocol !== "javascript:") post("unsupported-link", {});
  }
  document.addEventListener(
    "click",
    (event) => {
      const anchor = event.target?.closest?.("a[href]");
      if (!anchor || event.defaultPrevented || event.button !== 0) return;
      let url;
      try {
        url = new URL(linkHref(anchor), location.href);
      } catch (_) {
        return;
      }
      if (!["http:", "https:", "mailto:"].includes(url.protocol)) {
        event.preventDefault();
        reportUnsupportedLink(event, url);
        return;
      }
      const newTab =
        anchor.target === "_blank" ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey;
      if (url.origin !== location.origin || newTab) {
        event.preventDefault();
        post("external-link", { url: url.href, newTab });
      } else if (url.pathname === location.pathname && url.search === location.search &&
          url.hash !== location.hash) {
        expectHashNavigation(url.href);
      }
    },
    true,
  );
  document.addEventListener(
    "contextmenu",
    (event) => {
      const resource = event.target?.closest?.("video,audio,img");
      const anchor = event.target?.closest?.("a[href],area[href]");
      if ((!resource && !anchor) || event.defaultPrevented) return;
      // Archive links never offer browser-owned popup/window actions. Only a
      // genuine gesture may request shell chrome; scripts cannot activate it.
      event.preventDefault();
      if (!event.isTrusted) return;
      const target = resource
        ? resource.currentSrc || (resource.getAttribute("src")?.trim() ? resource.src : "")
        : linkHref(anchor);
      if (typeof target !== "string" || !target) return;
      let url;
      try {
        url = new URL(target, location.href);
      } catch (_) {
        return;
      }
      if (resource) {
        if (url.origin === location.origin &&
            ["http:", "https:"].includes(url.protocol) &&
            url.href.length <= 16384)
          post("resource-context", { url: url.href, documentUrl: location.href });
        return;
      }
      if (
        url.href.length <= 16384 &&
        ["http:", "https:", "mailto:"].includes(url.protocol)
      )
        post("link-context", { url: url.href, documentUrl: location.href });
    },
    true,
  );
  // Only URL hints cross this boundary. The shell validates the current archive
  // and obtains bounded text from libzim; archive HTML never enters a preview.
  let previewTimer;
  let previewAnchor = null;
  let linkPreviews = false;
  function setLinkPreviews(enabled) {
    linkPreviews = enabled;
    if (!enabled) {
      clearTimeout(previewTimer);
      previewAnchor = null;
    }
  }
  function previewLink(event) {
    if (!event.isTrusted || !linkPreviews) return;
    const anchor = event.target?.closest?.("a[href]");
    if (!anchor || anchor === previewAnchor) return;
    clearTimeout(previewTimer);
    previewAnchor = anchor;
    let url;
    try {
      url = new URL(linkHref(anchor), location.href);
    } catch (_) {
      return;
    }
    if (
      url.origin !== location.origin ||
      !["http:", "https:"].includes(url.protocol) ||
      url.href.length > 16384 ||
      // A footnote, section or back link of this page has nothing to preview.
      (url.pathname === location.pathname && url.search === location.search)
    )
      return;
    previewTimer = setTimeout(() => {
      if (!linkPreviews || previewAnchor !== anchor || !anchor.isConnected)
        return;
      post("article-preview", { url: url.href });
    }, 350);
  }
  function leavePreviewLink(event) {
    if (event.target?.closest?.("a[href]") !== previewAnchor) return;
    if (
      event.relatedTarget instanceof Node &&
      previewAnchor?.contains(event.relatedTarget)
    )
      return;
    clearTimeout(previewTimer);
    previewAnchor = null;
  }
  document.addEventListener("pointerover", previewLink, true);
  document.addEventListener("focusin", previewLink, true);
  document.addEventListener("pointerout", leavePreviewLink, true);
  document.addEventListener("focusout", leavePreviewLink, true);
  document.addEventListener(
    "mousedown",
    (event) => {
      if (
        event.isTrusted &&
        ([3, 4].includes(event.button) ||
          (event.button === 1 && event.target?.closest?.("a[href],area[href]")))
      )
        event.preventDefault();
    },
    true,
  );
  document.addEventListener(
    "mouseup",
    (event) => {
      if (!event.isTrusted || ![3, 4].includes(event.button)) return;
      event.preventDefault();
      post("history-travel", {
        direction: event.button === 3 ? "back" : "forward",
      });
    },
    true,
  );
  document.addEventListener(
    "auxclick",
    (event) => {
      if ([3, 4].includes(event.button)) {
        if (event.isTrusted) event.preventDefault();
        return;
      }
      if (event.button !== 1 || event.defaultPrevented) return;
      const anchor = event.target?.closest?.("a[href],area[href]");
      if (!anchor) return;
      event.preventDefault();
      if (!event.isTrusted) return;
      let url;
      try {
        url = new URL(linkHref(anchor), location.href);
      } catch (_) {
        return;
      }
      if (
        url.href.length <= 16384 &&
        ["http:", "https:", "mailto:"].includes(url.protocol)
      )
        post("external-link", { url: url.href, newTab: true });
      else reportUnsupportedLink(event, url);
    },
    true,
  );
  let selectionTimer;
  let pointerSelecting = false;
  let selectionDirty = false;
  const finishPointerSelection = () => {
    if (!pointerSelecting) return;
    pointerSelecting = false;
    if (selectionDirty) {
      selectionDirty = false;
      clearTimeout(selectionTimer);
      selectionTimer = setTimeout(selectionReport, 20);
    }
  };
  document.addEventListener("pointerdown", (event) => {
    if (!event.isPrimary || (event.pointerType === "mouse" && event.button !== 0)) return;
    pointerSelecting = true;
    selectionDirty = false;
  }, true);
  document.addEventListener("pointermove", (event) => {
    if (pointerSelecting && event.pointerType === "mouse" && event.buttons === 0) {
      finishPointerSelection();
    }
  }, true);
  window.addEventListener("pointerup", finishPointerSelection, true);
  window.addEventListener("pointercancel", finishPointerSelection, true);
  window.addEventListener("mouseup", finishPointerSelection, true);
  window.addEventListener("blur", finishPointerSelection);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") finishPointerSelection();
  });
  window.addEventListener("contextmenu", finishPointerSelection, true);
  window.addEventListener("dragstart", finishPointerSelection, true);
  window.addEventListener("dragend", finishPointerSelection, true);
  document.addEventListener("selectionchange", () => {
    clearTimeout(selectionTimer);
    if (!window.getSelection()?.isCollapsed) resetReadingPan();
    // Avoid full-document textIndex() scans every 90 ms while actively dragging
    // a pointer selection; compute the anchor once the pointer settles.
    if (pointerSelecting) {
      selectionDirty = true;
      return;
    }
    selectionTimer = setTimeout(selectionReport, 90);
  });
  window.addEventListener(
    "scroll",
    () => {
      if (scrollScheduled) return;
      scrollScheduled = true;
      requestAnimationFrame(() => {
        scrollScheduled = false;
        reportReadingPan();
        post("scroll", { x: window.scrollX, y: window.scrollY });
      });
    },
    { passive: true },
  );
  function navigationChanged(navigationKind = "push", historyRequestId, url = location.href) {
    resetReadingPan();
    stopSpeech();
    resetReadAloud();
    annotationItems = [];
    stopAnnotationObservation();
    annotationReceiving = false;
    annotationUrl = "";
    annotationRanges = [];
    paintAnnotations();
    selectionReport();
    post("navigation", {
      url,
      headingFragment: headingFragment(),
      title: document.title,
      toc: toc(),
      navigationKind,
      ...(historyRequestId ? { historyRequestId } : {}),
    });
  }
  window.addEventListener("hashchange", () => documentHistoryChanged("hash"));
  window.addEventListener("popstate", () => documentHistoryChanged("pop"));
  for (const method of ["pushState", "replaceState"]) {
    const original = history[method];
    history[method] = function (...args) {
      const before = location.href;
      const arriving = documentTraversal?.arrived && documentTraversal.url === before ? documentTraversal : null;
      const result = Reflect.apply(original, this, args);
      const url = location.href;
      if (arriving && documentTraversal === arriving) {
        if (method === "replaceState") {
          documentHistory.replace(url);
          arriving.url = url;
          return result;
        }
        // A router may push immediately from popstate. Publish the reached
        // entry first, then report the new push in native order.
        finishDocumentTraversal(arriving, before, false);
      }
      if (documentHistory.url !== before || documentTraversal) resetDocumentHistory();
      expectedHashNavigation = null;
      documentHistory[method === "replaceState" ? "replace" : "push"](url);
      queueMicrotask(() =>
        navigationChanged(method === "replaceState" ? "replace" : "push", undefined, url),
      );
      return result;
    };
  }
  document.addEventListener("visibilitychange", () => {
    resetReadingPan();
    // A hidden window only loses its read-along paint. Reading itself goes on,
    // like native speech does: stop, navigation and pagehide still end it.
    if (document.hidden) {
      try { clearReadAloudPaint(); } catch (_) { /* paint only */ }
    }
  });
  window.speechSynthesis?.addEventListener("voiceschanged", () =>
    post("voices", { voices: voices() }),
  );
  window.addEventListener("pagehide", () => {
    resetReadingPan();
    resetDocumentHistory();
    clearTimeout(previewTimer);
    clearTimeout(selectionTimer);
    clearTimeout(annotationTimer);
    cancelAnimationFrame(annotationFrame);
    stopAnnotationObservation();
    revoke("camera");
    revoke("location");
    stopSpeech();
    resetReadAloud();
  });
  // A MapLibre canvas can lose its WebGL context in WKWebView after navigating
  // away and back. Reload once (rate-limited) instead of leaving a blank map.
  function guardMapContext() {
    if (!document.querySelector(".maplibregl-map")) return;
    let reloaded = false;
    const reload = () => {
      if (reloaded) return;
      reloaded = true;
      try {
        const key = "kiwix:map-context-reload";
        const last = Number(sessionStorage.getItem(key) || 0);
        if (Date.now() - last < 15000) return;
        sessionStorage.setItem(key, String(Date.now()));
      } catch (_) { /* Storage may be unavailable; the in-document guard still bounds this. */ }
      location.reload();
    };
    document.addEventListener("webglcontextlost", (event) => {
      event.preventDefault();
      reload();
    }, true);
    window.addEventListener("pageshow", (event) => {
      if (event.persisted) reload();
    });
  }
  if (document.readyState === "complete") guardMapContext();
  else window.addEventListener("load", guardMapContext, { once: true });
})();
