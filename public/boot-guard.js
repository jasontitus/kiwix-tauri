/* SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Installed before the application module. The native shell can call
 * window.__kiwixShowStartupFailure(...) from its 10-second watchdog, and the
 * page has its own later fallback in case that IPC/eval path is what broke.
 * Nothing here talks to the network or persists data: diagnostics stay in
 * this window until the user copies them.
 */
(function () {
  "use strict";

  var startedAt = Date.now();
  var state = {
    startedAt: new Date(startedAt).toISOString(),
    phase: "html",
    detail: "",
    ready: false,
    events: [],
  };
  var currentPayload = null;

  window.__KIWIX_BOOT_STATE__ = state;

  function stamp() {
    return new Date().toISOString();
  }

  function record(kind, message) {
    var text = String(message == null ? "" : message);
    if (text.length > 800) text = text.slice(0, 800) + "…";
    state.events.push({ at: stamp(), kind: kind, message: text });
    if (state.events.length > 30) state.events.shift();
  }

  window.__kiwixBootPhase = function (phase, detail) {
    state.phase = String(phase || "unknown");
    state.detail = detail == null ? "" : String(detail).slice(0, 800);
  };

  // Resource errors do not bubble, so capture them at the window.
  window.addEventListener(
    "error",
    function (event) {
      var target = event.target;
      if (target && target !== window && target.tagName) {
        var source =
          target.src || target.href || target.currentSrc || target.tagName;
        record("asset", "Failed to load " + source);
        return;
      }
      record("error", event.message || "Unknown script error");
    },
    true,
  );

  window.addEventListener("unhandledrejection", function (event) {
    var reason = event.reason;
    var message =
      reason && reason.message
        ? reason.message
        : reason == null
          ? "Unhandled promise rejection"
          : String(reason);
    record("rejection", message);
  });

  function diagnosticsText(payload) {
    var report = {
      application: "Kiwix",
      generatedAt: stamp(),
      elapsedMs: Date.now() - startedAt,
      location: window.location.href,
      userAgent: navigator.userAgent,
      boot: {
        phase: state.phase,
        detail: state.detail,
        ready: state.ready,
        startedAt: state.startedAt,
        events: state.events,
      },
      watchdog: payload || null,
      nativeIpc: Boolean(
        window.__TAURI_INTERNALS__ || window.__TAURI__ || window.ipc,
      ),
    };
    try {
      return JSON.stringify(report, null, 2);
    } catch (_) {
      return String(report);
    }
  }

  function flash(message) {
    var status = document.getElementById("kiwix-copy-status");
    if (!status) return;
    status.textContent = message;
    window.setTimeout(function () {
      if (status.textContent === message) status.textContent = "";
    }, 2400);
  }

  function copyDiagnostics() {
    var text = diagnosticsText(currentPayload);
    function fallback() {
      try {
        var area = document.createElement("textarea");
        area.value = text;
        area.setAttribute("readonly", "readonly");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        var copied = document.execCommand("copy");
        document.body.removeChild(area);
        flash(copied ? "Diagnostics copied." : "Could not copy diagnostics.");
      } catch (_) {
        flash("Could not copy diagnostics.");
      }
    }
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(
        function () {
          flash("Diagnostics copied.");
        },
        fallback,
      );
    } else {
      fallback();
    }
  }

  function addRow(list, term, value) {
    var dt = document.createElement("dt");
    dt.textContent = term;
    var dd = document.createElement("dd");
    dd.textContent = value;
    list.appendChild(dt);
    list.appendChild(dd);
  }

  function renderFailure(payload) {
    var existing = document.getElementById("kiwix-startup-failure");
    if (existing) existing.parentNode.removeChild(existing);

    var panel = document.createElement("div");
    panel.id = "kiwix-startup-failure";
    panel.setAttribute("role", "alertdialog");
    panel.setAttribute("aria-modal", "true");
    panel.setAttribute("aria-labelledby", "kiwix-failure-title");

    var card = document.createElement("div");
    card.className = "kiwix-failure-card";

    var eyebrow = document.createElement("p");
    eyebrow.className = "kiwix-failure-eyebrow";
    eyebrow.textContent = "Kiwix";
    card.appendChild(eyebrow);

    var title = document.createElement("h1");
    title.id = "kiwix-failure-title";
    title.textContent = "Kiwix could not finish starting";
    card.appendChild(title);

    var reason = document.createElement("p");
    reason.className = "kiwix-failure-reason";
    reason.textContent =
      (payload && payload.reason) ||
      "The interface did not report ready. The window is still usable: copy the diagnostics or reload.";
    card.appendChild(reason);

    var rows = document.createElement("dl");
    addRow(rows, "Stage", state.phase + (state.detail ? " — " + state.detail : ""));
    addRow(
      rows,
      "Window",
      (payload && payload.window) || "unknown",
    );
    addRow(
      rows,
      "Waited",
      Math.round((Date.now() - startedAt) / 1000) + " seconds",
    );
    addRow(rows, "IPC", window.__TAURI_INTERNALS__ ? "available" : "unavailable");
    var assetFailures = state.events.filter(function (event) {
      return event.kind === "asset";
    });
    addRow(
      rows,
      "Assets",
      assetFailures.length
        ? assetFailures.length + " failed to load"
        : "no load failure recorded",
    );
    card.appendChild(rows);

    var details = document.createElement("details");
    var summary = document.createElement("summary");
    summary.textContent = "Diagnostic details";
    details.appendChild(summary);
    var pre = document.createElement("pre");
    pre.textContent = diagnosticsText(payload);
    details.appendChild(pre);
    card.appendChild(details);

    var actions = document.createElement("div");
    actions.className = "kiwix-failure-actions";
    var copy = document.createElement("button");
    copy.type = "button";
    copy.textContent = "Copy diagnostics";
    copy.addEventListener("click", copyDiagnostics);
    actions.appendChild(copy);
    var reload = document.createElement("button");
    reload.type = "button";
    reload.className = "primary";
    reload.textContent = "Reload";
    reload.addEventListener("click", function () {
      window.location.reload();
    });
    if (!payload || payload.allowReload !== false) {
      actions.appendChild(reload);
    }
    if (Array.isArray(payload && payload.actions)) {
      payload.actions.forEach(function (action) {
        if (!action || !action.label || !action.action) return;
        var button = document.createElement("button");
        button.type = "button";
        button.textContent = String(action.label);
        button.addEventListener("click", function () {
          var pending;
          try {
            pending = window.__TAURI_INTERNALS__.invoke("startup_failure", {
              action: String(action.action),
            });
          } catch (error) {
            flash("Startup action failed: " + String(error));
            return;
          }
          // Tauri invoke returns a Promise; a try/catch alone never observes
          // a rejected native command.
          Promise.resolve(pending).then(
            function (result) {
              flash((result && result.message) || "Done.");
            },
            function (error) {
              flash("Startup action failed: " + String(error));
            },
          );
        });
        actions.appendChild(button);
      });
    }
    card.appendChild(actions);

    var status = document.createElement("p");
    status.id = "kiwix-copy-status";
    status.className = "kiwix-copy-status";
    status.setAttribute("role", "status");
    card.appendChild(status);

    panel.appendChild(card);
    document.body.appendChild(panel);
    state.failureShown = true;
    try {
      copy.focus();
    } catch (_) {
      /* Focus is a convenience; the panel is still announced. */
    }
  }

  window.__kiwixShowStartupFailure = function (payload) {
    if (state.ready) return;
    currentPayload = payload || {};
    record(
      "watchdog",
      (currentPayload.source || "native") +
        ": " +
        (currentPayload.reason || "frontend did not report ready"),
    );
    renderFailure(currentPayload);
  };

  window.__kiwixFrontendReady = function () {
    if (state.ready) return;
    state.ready = true;
    state.phase = "ready";
    window.clearTimeout(fallbackTimer);
    var panel = document.getElementById("kiwix-startup-failure");
    if (panel && panel.parentNode) panel.parentNode.removeChild(panel);
    window.dispatchEvent(new Event("kiwix-frontend-ready"));
  };

  // The native watchdog is authoritative on desktop, but native eval is
  // unavailable on iOS. The page fallback probes the mobile-only
  // `startup_failure` status command so the real native reason and recovery
  // actions reach the panel; absent/rejected probes keep the generic panel.
  function showFallbackPanel() {
    if (state.ready) return;
    var base = {
      source: "page",
      reason:
        "The interface has not reported ready within 9 seconds. The module bundle, IPC, or startup snapshot may have failed.",
    };
    if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
      var probe;
      try {
        probe = window.__TAURI_INTERNALS__.invoke("startup_failure", {
          action: "status",
        });
      } catch (_) {
        window.__kiwixShowStartupFailure(base);
        return;
      }
      Promise.resolve(probe).then(
        function (status) {
          if (state.ready) return;
          if (status && status.failed) {
            window.__kiwixShowStartupFailure(
              Object.assign({}, base, {
                source: "native",
                reason: status.reason || base.reason,
                actions: status.actions,
                allowReload: status.allowReload === false ? false : true,
              }),
            );
          } else {
            window.__kiwixShowStartupFailure(base);
          }
        },
        function () {
          window.__kiwixShowStartupFailure(base);
        },
      );
    } else {
      window.__kiwixShowStartupFailure(base);
    }
  }
  var fallbackTimer = window.setTimeout(showFallbackPanel, 9000);
})();
