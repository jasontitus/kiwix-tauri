import assert from "node:assert/strict";
import test from "node:test";
import {
  dismissStartupNotice,
  pruneStartupNoticeDismissals,
  visibleStartupNotices,
} from "../src/startup-notice.ts";

const CANCELLED = "Import cancelled because Kiwix closed, moved to the background, or switched privacy mode. Add the archive again to import it.";
const COPY = "Some saved items were kept in state.skipped-1.json.";

/** One snapshot arriving, as App.tsx applies it. */
const arrive = (warnings, dismissed) => pruneStartupNoticeDismissals(warnings, dismissed);

test("a closed startup band stays closed for the notices it showed", () => {
  const shown = [COPY];
  assert.deepEqual(visibleStartupNotices(shown, []), shown);
  const dismissed = dismissStartupNotice(shown);
  assert.deepEqual(visibleStartupNotices(shown, dismissed), []);
  assert.deepEqual(visibleStartupNotices(undefined, dismissed), []);
  // An unchanged snapshot keeps the very same dismissal list (no re-render).
  assert.equal(arrive(shown, dismissed), dismissed);
});

test("a later notice reopens the band with only the notices not yet closed", () => {
  const dismissed = dismissStartupNotice([COPY]);
  const later = [COPY, CANCELLED];
  assert.deepEqual(visibleStartupNotices(later, arrive(later, dismissed)), [CANCELLED]);
});

test("a withdrawn notice that is noted again is shown again", () => {
  let dismissed = dismissStartupNotice([COPY, CANCELLED]);
  // The import is added again, and the notice is withdrawn.
  dismissed = arrive([COPY], dismissed);
  assert.deepEqual(visibleStartupNotices([COPY], dismissed), []);
  // Another import is cancelled: the identical sentence is new now.
  dismissed = arrive([COPY, CANCELLED], dismissed);
  assert.deepEqual(visibleStartupNotices([COPY, CANCELLED], dismissed), [CANCELLED]);
});
