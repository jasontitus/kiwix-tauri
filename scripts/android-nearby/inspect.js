/* SPDX-License-Identifier: GPL-3.0-or-later
 * Read-only top-frame inspection. The instrumentation performs all input using
 * Android MotionEvent/KeyEvent injection, never JavaScript UI mutations.
 */
(function (request) {
  "use strict";
  const text = (e) => (e.textContent || "").replace(/\s+/g, " ").trim();
  const shown = (e) => {
    const s = getComputedStyle(e), r = e.getBoundingClientRect();
    return s.visibility !== "hidden" && s.display !== "none" && r.width > 0 && r.height > 0;
  };
  const all = (selector, root = document) => Array.from(root.querySelectorAll(selector)).filter(shown);
  const name = (e) => e.getAttribute("aria-label") || text(e);
  const one = (items, what) => {
    if (items.length > 1) throw Error("Ambiguous " + what + ": " + items.length);
    return items[0];
  };
  const button = (root, label) => one(all("button", root).filter((e) => name(e) === label), label);
  const onScreen = (e) => {
    const r = e.getBoundingClientRect();
    return shown(e) && r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
  };
  const dialog = one(all("dialog[open]"), "active dialog");
  const crashOperation = request.op.startsWith("crash-");
  const cleanupInput = request.cleanup_owned === true &&
    ["nearby", "stop-share", "stop-receive", "receive-close"].includes(request.op);
  const cleanupObservation = request.cleanup_owned === true && request.op === "publication-stopped";
  const crashPrompt = dialog && dialog.querySelector(".modal-heading h2") &&
    text(dialog.querySelector(".modal-heading h2")) === "Crash report available" &&
    one(all(".crash-report", dialog), "startup crash report");
  if (crashPrompt && !crashOperation && !cleanupInput) return { error: "Unclosed native crash report prompt; no automatic dismissal" };
  const errors = all(".global-error").filter(onScreen).map(text).filter(Boolean);
  const pageErrors = all(".inline-error,[role='alert']").filter((x) =>
    onScreen(x) && !x.closest("article.download-card,.swipe-map-job") &&
    !(crashOperation && crashPrompt && crashPrompt.contains(x)) && (!dialog || dialog.contains(x)));
  errors.push(...pageErrors.map(text).filter(Boolean));
  if (errors.length && !cleanupInput && !cleanupObservation) return { error: "Current page/dialog error", errors };
  if (all(".localswarm-notice").some((e) => /unavailable/.test(text(e)))) {
    return { error: "Nearby blocked by private/read-only policy" };
  }
  let e;
  const nav = (label) => one(all("nav.main-nav button").filter((x) => name(x) === label), "navigation " + label);
  switch (request.op) {
    case "crash-prompt":
    case "crash-view":
    case "crash-record":
    case "crash-close":
    case "crash-closed": {
      if (!request.crash_record || request.crash_record.schema !== 1 || request.crash_record.reportedAt !== undefined)
        return { error: "No exact unreported owned crash record admitted" };
      if (request.op === "crash-closed") {
        if (dialog) return { error: "A dialog remains after reviewed crash Close" };
        return nav("Nearby") ? { ready: true, crash_prompt_visible: false } : { pending: "Waiting app navigation after Close" };
      }
      if (!dialog) return { pending: "Waiting startup crash prompt classification" };
      if (!crashPrompt) return { error: "Unexpected startup dialog; no input admitted" };
      if (request.op === "crash-prompt") return { ready: true, crash_prompt_visible: true };
      if (request.op === "crash-view") {
        e = button(crashPrompt, "View report");
        if (!e) return { error: "Startup report View control missing" };
        break;
      }
      const field = one(all("textarea[readonly]", crashPrompt), "displayed crash JSON");
      if (!field) return { pending: "Waiting ordinary View report JSON" };
      if (field.value.length > 65536) return { error: "Displayed crash report exceeds bound" };
      let record;
      try { record = JSON.parse(field.value); }
      catch (_) { return { error: "Displayed crash report is not JSON" }; }
      const canonical = (v) => v && typeof v === "object"
        ? Array.isArray(v) ? "[" + v.map(canonical).join(",") + "]"
          : "{" + Object.keys(v).sort().map((k) => JSON.stringify(k) + ":" + canonical(v[k])).join(",") + "}"
        : JSON.stringify(v);
      if (canonical(record) !== canonical(request.crash_record))
        return { error: "Displayed native crash report differs from exact reviewed owned record" };
      if (request.op === "crash-record") return { ready: true, record };
      const actions = one(all(":scope > .modal-actions", dialog), "startup crash actions");
      e = actions && button(actions, "Close");
      if (!e) return { error: "Scoped startup crash Close control missing" };
      break;
    }
    case "nearby": e = nav("Nearby"); break;
    case "downloads": e = nav("Downloads"); break;
    case "receiving-ready":
      return { ready: !!one(all(".localswarm"), "Nearby surface") && !!nav("Nearby") && nav("Nearby").getAttribute("aria-current") === "page",
        hotspot_active: !!one(all(".localswarm-hotspot"), "temporary Wi-Fi"),
        sharing_active: !!button(document.querySelector(".localswarm-share-card") || document, "Stop sharing"),
        other_shares_active: all(".localswarm-seed").length > 0 };
    case "lan-route":
    case "sender-hotspot-route":
    case "reverse-route": {
      const library = one(all(".localswarm-library"), "selected sender catalog");
      if (!dialog || !library || !dialog.contains(library)) return { pending: "Waiting selected sender catalog" };
      const heading = dialog.querySelector(".modal-heading h2");
      const senderName = heading && text(heading);
      const expandedHint = new TextEncoder().encode(request.peer).length === 8 && senderName && senderName.startsWith(request.peer);
      if (!senderName || (senderName !== request.peer && !expandedHint)) return { error: "Selected sender catalog identity mismatch" };
      const connected = one(all(":scope > p[role='status']", library).filter((x) => /^Connected(?: ·|$)/.test(text(x))), "Connected sender catalog");
      if (!connected) return { pending: "Waiting Connected sender catalog" };
      if (request.op === "lan-route") {
        if (one(all(".localswarm-hotspot"), "unexpected temporary Wi-Fi"))
          return { error: "LAN catalog must not use product temporary Wi-Fi" };
        return { ready: true, selected_sender: senderName, catalog_connection: text(connected), hotspot_active: false };
      }
      if (request.op === "sender-hotspot-route") {
        if (one(all(".localswarm-hotspot"), "unexpected receiver AP"))
          return { error: "Sender-hotspot receiver must not create a reverse AP" };
        return { ready: true, selected_sender: senderName, catalog_connection: text(connected), hotspot_active: false };
      }
      const hotspot = one(all(".localswarm-hotspot"), "product temporary Wi-Fi");
      if (!hotspot) return { pending: "Connected catalog missing required product reverse hotspot" };
      const hotspotTitle = hotspot.querySelector("h2");
      if (!hotspotTitle || text(hotspotTitle) !== "Temporary Wi-Fi") return { error: "Unexpected product hotspot surface" };
      return { ready: true, product_owned_ap_at_catalog: true, selected_sender: text(heading),
        catalog_connection: text(connected), hotspot_title: text(hotspotTitle) };
    }
    case "peer": {
      const card = one(all("article.localswarm-peer").filter((x) => {
        const strong = x.querySelector("strong");
        return strong && text(strong) === request.peer;
      }), "peer " + request.peer);
      if (card) e = one(all("button", card).filter((x) => text(x) === "Choose"), "Choose expected peer");
      break;
    }
    case "filter": {
      const label = one(all(".localswarm-library label.field").filter((x) => text(x) === "Find archives"), "Find archives");
      if (label) e = one(all("input[type='search']", label), "archive filter");
      break;
    }
    case "download": {
      const input = one(all(".localswarm-library input[type='search']"), "archive filter");
      if (!input || input.value !== request.file_name) return { pending: "Exact wire filename not in filter" };
      const cards = all("article.localswarm-library-book");
      if (cards.length > 1) return { error: "Ambiguous wire filename filter", count: cards.length };
      const card = cards[0];
      if (card) {
        const title = card.querySelector("strong");
        if (!title || text(title) !== request.title) return { error: "Archive title mismatch", actual: title && text(title) };
        if (all("button", card).some((x) => /^Open |^Library$/.test(text(x)))) return { error: "Archive already installed in offered library" };
        e = button(card, "Download");
      }
      break;
    }
    case "open": {
      if (dialog || !nav("Downloads") || nav("Downloads").getAttribute("aria-current") !== "page") return { pending: "Waiting automatic Downloads navigation after admission" };
      const card = one(all("article.download-card").filter((x) => x.getAttribute("aria-label") === request.file_name), "exact wire download card");
      if (card) {
        const targetErrors = all(".inline-error,[role='alert']", card).map(text).filter(Boolean);
        if (targetErrors.length) return { error: "Selected download/import error", errors: targetErrors };
        const bad = all(".download-status", card).map(text).filter((s) => /failed|error|cancelled|paused/i.test(s));
        if (bad.length) return { error: "Download not healthy", errors: bad };
        e = button(card, "Open " + request.title);
        if (!e) return { pending: "Waiting imported Open", card: text(card).slice(0, 2400) };
      }
      break;
    }
    case "publish": e = button(document.querySelector(".localswarm-share-card") || document,
      request.topology === "lan" ? "Share all" : "Use temporary Wi-Fi"); break;
    case "stop-share": e = button(document.querySelector(".localswarm-share-card") || document, "Stop sharing"); break;
    case "receive-close": {
      if (!dialog) return { ready: true, no_dialog: true };
      const heading = dialog.querySelector(".modal-heading h2"), label = heading && text(heading);
      if (label !== request.peer && !(new TextEncoder().encode(request.peer).length === 8 && label && label.startsWith(request.peer)))
        return { error: "Unexpected dialog during owned receive cleanup" };
      const header = one(all(".modal-heading", dialog), "owned catalog heading");
      e = header && button(header, "Close");
      break;
    }
    case "stop-receive": {
      const footer = one(all(".localswarm-footer"), "Nearby stop footer");
      e = footer && button(footer, "Stop all");
      break;
    }
    case "receive-stopped": {
      const footer = one(all(".localswarm-footer"), "Nearby stop footer");
      const receive = one(all(".localswarm-receive"), "receive controls");
      return !dialog && footer && button(footer, "Stop all") && receive && button(receive, "Find nearby") &&
        !button(receive, "Stop finding") && !one(all(".localswarm-hotspot"), "receiver AP")
        ? { ready: true, ordinary_stop_all_completed: true } : { pending: "Waiting owned receive retirement" };
    }
    case "publication-stopped": {
      const share = one(all(".localswarm-share-card"), "library sharing surface");
      const heading = share && share.querySelector("h2");
      const stop = share && button(share, "Stop sharing"), start = share && button(share, "Share all");
      const hotspot = !!one(all(".localswarm-hotspot"), "temporary Wi-Fi");
      const idle = share && share.getAttribute("aria-busy") === "false" && start && !start.disabled &&
        heading && text(heading) === "Share your library";
      return !dialog && idle && !stop && !hotspot
        ? { ready: true, publication_nonstarting: true, publication_nonsharing: true,
            sharing_active: false, hotspot_active: false, publication_errors: errors }
        : { pending: "Waiting owned library sharing and temporary Wi-Fi to stop",
            sharing_active: !!stop, hotspot_active: hotspot, publication_errors: errors };
    }
    case "lan-publication": {
      if (one(all(".localswarm-hotspot"), "unexpected temporary Wi-Fi"))
        return { error: "LAN publication must not use product temporary Wi-Fi" };
      const share = one(all(".localswarm-share-card"), "library sharing surface");
      const heading = share && share.querySelector("h2");
      return share && heading && text(heading) === "You’re sharing your library" && button(share, "Stop sharing")
        ? { ready: true, sharing_active: true, hotspot_active: false, sharing: text(share) }
        : { pending: "Waiting ordinary Share all publication" };
    }
    case "hotspot": {
      const section = one(all(".localswarm-hotspot"), "active temporary Wi-Fi");
      const stop = document.querySelector(".localswarm-share-card");
      const heading = section && section.querySelector("h2");
      const network = section && one(all(":scope > p", section).filter((p) => /^Network: /.test(text(p))), "owned AP network");
      return section && heading && text(heading) === "Temporary Wi-Fi" && stop && button(stop, "Stop sharing")
        ? { ready: request.topology !== "sender-hotspot" || !!network, hotspot_active: true, product_owned_ap_at_publish: true,
            network_name: network && text(network).slice("Network: ".length),
            hotspot_title: text(heading), sharing: text(stop) }
        : { pending: "Waiting active temporary Wi-Fi and sharing" };
    }
    case "reader": {
      const frame = one(all("iframe[title^='Article: ']"), "reader iframe");
      const region = one(all("[aria-label='Article content']"), "Article content");
      if (!frame || !region) return { pending: "Waiting reader surface" };
      if (region.getAttribute("aria-busy") === "true" || frame.classList.contains("is-pending")) return { pending: "Waiting rendered article" };
      const r = frame.getBoundingClientRect();
      let content = null, cross_origin = false;
      try {
        const doc = frame.contentDocument;
        cross_origin = !doc;
        content = doc && doc.body && doc.body.innerText.slice(0, 8000);
      }
      catch (_) { cross_origin = true; }
      return { ready: r.width > 0 && r.height > 0, frame_title: frame.title, frame_src: frame.getAttribute("src"), content, cross_origin,
        viewport: { width: innerWidth, height: innerHeight }, frame: { x: r.x, y: r.y, width: r.width, height: r.height } };
    }
    default: return { error: "Unknown inspection operation" };
  }
  if (!e) return { pending: "Waiting " + request.op };
  if (e.disabled || e.getAttribute("aria-disabled") === "true") return { pending: "Disabled " + request.op };
  // A moving bottom sheet can put Send by email under View's sampled point
  // before native input arrives. Never admit a control or ancestor in motion.
  for (let p = e; p; p = p.parentElement) {
    if (typeof p.getAnimations !== "function") return { error: "Native input requires animation-state inspection" };
    if (p.getAnimations().some((a) => a.pending || a.playState === "running"))
      return { pending: "Waiting target motion to settle" };
  }
  const r = e.getBoundingClientRect();
  let clip = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
  let scroll = clip;
  for (let p = e.parentElement; p; p = p.parentElement) {
    const s = getComputedStyle(p), pr = p.getBoundingClientRect();
    if (/(auto|scroll|hidden|clip)/.test(s.overflowY + s.overflowX)) {
      clip = { left: Math.max(clip.left, pr.left), top: Math.max(clip.top, pr.top), right: Math.min(clip.right, pr.right), bottom: Math.min(clip.bottom, pr.bottom) };
      if (/(auto|scroll)/.test(s.overflowY) && p.scrollHeight > p.clientHeight) scroll = { left: pr.left, top: pr.top, right: pr.right, bottom: pr.bottom };
    }
  }
  const x = (Math.max(r.left, clip.left) + Math.min(r.right, clip.right)) / 2;
  const y = (Math.max(r.top, clip.top) + Math.min(r.bottom, clip.bottom)) / 2;
  const hit = document.elementFromPoint(x, y);
  const tappable = r.right > clip.left && r.left < clip.right && r.bottom > clip.top && r.top < clip.bottom && hit && (hit === e || e.contains(hit));
  return { ready: true, tappable: !!tappable, name: name(e), value: e.value,
    point: { x, y }, rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom }, clip, scroll,
    viewport: { width: innerWidth, height: innerHeight } };
})
