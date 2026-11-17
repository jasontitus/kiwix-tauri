# Potential design: StreetZim turn-by-turn Live Activity

**Status: design proposal only. Not implemented, not scheduled, and not a
shipping commitment.** This document records feasibility research so a future
change does not repeat the investigation. It deliberately does not add a
JavaScript poller or a foreground-only navigation card.

## Research snapshot

StreetZim repository: `https://github.com/jasontitus/streetzim`

- Researched `main` at `a682ab0c1ee0ce9fde9e515e8a468cf8c5f6446d`.
- Turn-by-turn driving lives in `resources/viewer/index.html`.
- `driveMode` owns driving lifecycle and the HUD.
- `augmentRouteForDriving()` builds `route.turns`.
- `projectOnRoute()` computes route position and next-turn distance.
- `onPosition()` updates the HUD and computes remaining distance/time/ETA.
- HUD elements: `#drive-hud`, `#drive-arrow`, `#drive-dist`,
  `#drive-street`, `#drive-remaining`, `#drive-eta`, `#drive-status`.
- `window.streetzimRouting` is a public routing API, but it exposes
  `hasRoute`/`driveActive`/mode methods, not the current instruction or the
  route geometry needed by native code.
- `window.__streetzim_lastRoute` contains the internal route:
  `coords`, `distance`, `time`, `roads`, `cumM`, `turns`.
- Position tracking uses `navigator.geolocation.watchPosition`; driving mode
  also requests a screen wake lock.

## Why a JavaScript-only Live Activity is not enough

A bridge that posts HUD text to ActivityKit would be small, but it would not be
honest navigation:

1. When iOS suspends the app, WKWebView JavaScript stops. A lock-screen Live
   Activity would freeze on the last turn/distance/ETA.
2. `navigator.geolocation.watchPosition` in a background WKWebView is not a
   reliable background-location source.
3. StreetZim's driving mode is designed for a visible page; its wake lock
   confirms that assumption.
4. A stale next-turn card is more dangerous than no card.

A useful lock-screen turn-by-turn Live Activity therefore needs **native
background location following**, with the archive supplying the route and the
native side owning position updates while backgrounded.

## Options considered

### A. Observe the HUD DOM from Kiwix's injected reader bridge

- Add a `MutationObserver` for `#drive-hud` and post parsed text to the shell.
- Pros: no StreetZim changes; works with existing map archives.
- Cons: localized DOM scraping, brittle, and still foreground-only.
- Verdict: useful only as a diagnostic/fallback; not recommended for shipping.

### B. StreetZim posts structured navigation state

- Add a public `navigationState`/event API in StreetZim and post state to the
  parent Kiwix frame.
- Pros: stable contract, no DOM parsing, easy foreground integration.
- Cons: JavaScript is suspended in the background; Live Activity freezes.
- Verdict: necessary as the foreground control/route handoff, insufficient
  alone.

### C. Native background follower plus StreetZim route handoff (recommended)

- StreetZim sends one bounded route/turn package when driving starts and a
  stop event when it ends.
- Kiwix native code owns `CLLocationManager` background updates, route
  projection, next-turn distance, ETA and the ActivityKit lifecycle.
- StreetZim remains the route calculator and map renderer; native does not
  duplicate the router.
- Verdict: the only option that can honestly update a locked-screen Live
  Activity.

## Proposed contract

### StreetZim → Kiwix

A narrow message channel, validated by the injected reader bridge and bound to
the current archive session:

```json
{
  "channel": "kiwix-navigation",
  "version": 1,
  "type": "route-start",
  "sessionId": "<reader session>",
  "route": {
    "travelMode": "drive",
    "distanceM": 8400,
    "timeS": 620,
    "coords": [[30.5234, 50.4501], [30.5240, 50.4510]],
    "turns": [
      {
        "distM": 1200,
        "coordIdx": 42,
        "name": "Main Street",
        "flags": 0,
        "fromFlags": 0,
        "toCls": 1,
        "fromCls": 1
      }
    ]
  }
}
```

Bounds and validation:

- Maximum coordinate count, turn count, string lengths and total JSON size.
- Coordinates must be finite WGS84 ranges; distances monotonic.
- The message is accepted only from the active archive document and current
  session; stale route messages are ignored.
- `route-stop` or driving-mode exit ends the activity.
- A replacement route atomically replaces the old route and resets state.

A foreground state message can still carry `instruction`, `distanceLabel`,
`remainingLabel`, `etaLabel`, `glyph`, `offRoute`, and `arrived` for UI parity
and debugging. Native must not depend on those localized strings for the
background calculation.

### Native route follower

Inputs from the route package:

- `coords`: ordered `[lon, lat]`.
- `cumM`: cumulative distance per coordinate.
- `turns`: `distM`, `coordIdx`, `name`, flags/class for glyph selection.
- Planned distance/time for initial ETA.

On each `CLLocation` update:

1. Project the coordinate onto the polyline; reject impossible jumps.
2. Find the next turn with `distM > routePosition + threshold`.
3. Compute distance to turn and remaining distance.
4. Estimate ETA from planned speed or a bounded recent speed.
5. Detect off-route beyond a threshold; in the background, show
   **Off route — open map** rather than pretending to reroute.
6. Update the Live Activity; end on arrival/stop/route replacement.

Rerouting remains a StreetZim foreground operation. If the app returns to the
foreground, the archive may calculate a new route and send a replacement
package.

## ActivityKit model

A new `NavigationActivityAttributes` (separate from downloads and read-aloud):

Immutable attributes:

- `sessionId`
- `destination`/area label, if available and non-private
- `startedAt`

Mutable state:

- `phase`: `driving` | `offRoute` | `arrived` | `stopped`
- `instruction`
- `distanceToTurn`
- `remaining`
- `eta`
- `glyph`/maneuver kind
- `offRoute`
- optional progress fraction

Lock screen and Dynamic Island:

- next maneuver glyph/instruction
- distance to turn
- remaining distance and ETA
- compact: arrow/glyph and distance
- expanded: instruction, distance, ETA, open-map/stop links
- no map tile rendering in the widget

The activity ends immediately on stop/arrival/source retirement. It is
suppressed in private mode; no archive title or route is shown there.

## Privacy and permissions

This is a material permission expansion, not a cosmetic widget:

- Current StreetZim location use is foreground `When In Use`.
- Background following requires `Always` location authorization and the iOS
  `location` background mode, plus a clear consent screen explaining that
  position continues while the screen is locked.
- A denied/limited permission keeps the existing foreground HUD and does not
  start a navigation Live Activity.
- The route package is local archive data; do not upload it.
- The Live Activity must not expose route history, search text, or credentials.
- The archive is untrusted content: validate every field and bind messages to
  the active reader session. A forged activity can only display content; it
  must not acquire location, speech, storage, or navigation authority.

## Lifecycle and failure states

| Event | Result |
|---|---|
| Driving starts, route valid | Request activity, start native location |
| App backgrounds | Native location continues; JS stops |
| App foregrounds | JS resumes; HUD and native state reconcile |
| Route replaced | Native route atomically replaced |
| Off route | Activity shows off-route; reroute only in foreground |
| Arrival | Activity ends |
| User exits driving mode | Activity ends |
| Archive/session retires | Activity ends |
| Location permission revoked | Activity ends; foreground HUD remains |
| Activity request refused | Navigation continues in-app; no error banner |
| Process death | Stale activity ends/dismisses; do not claim live guidance |

## Android parity

Android already has foreground-service/MediaSession patterns for read-aloud and
a progress-centric Live Update for downloads. An equivalent navigation feature
would use a location foreground service and a navigation notification, with
the same route/turn contract. It is not part of this proposal.

## Verification plan if implemented

- Unit tests for route projection, next-turn selection, ETA, off-route
  thresholds, malformed route rejection and activity state transitions.
- Host bridge tests for route-start/route-stop/replacement and stale sessions.
- Simulator UI tests for lock-screen and Dynamic Island rendering.
- Physical iPhone: background location, lock screen, screen-off, permission
  denial/revocation, arrival, process death and Live Activity cleanup.
- Battery/thermal and location-accuracy measurements; do not claim acceptance
  from simulator screenshots.
- Android foreground-service/location acceptance separately.

## Open decisions

1. Is `Always` background location acceptable for this app's privacy posture?
2. Should navigation be opt-in per archive or per session?
3. Which route fields are stable public StreetZim API versus internal?
4. Should native estimate ETA from planned time, recent speed, or both?
5. Should the Live Activity offer only **Open map** and **Stop**, or also
   **Pause guidance**?
6. How should off-route/reroute be represented when the webview is suspended?

## Non-goals

- No JavaScript-only Live Activity presented as live background guidance.
- No map tiles or route polyline inside the widget.
- No native router in Kiwix.
- No change to StreetZim's route algorithm as part of this proposal.
- No claim that the current HUD text is a stable machine API.
