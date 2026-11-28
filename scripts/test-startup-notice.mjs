import assert from "node:assert/strict";
import test from "node:test";
import { dismissStartupNotice, startupNoticeVisible } from "../src/startup-notice.ts";

test("a closed startup band stays closed for the notices it showed", () => {
  const shown = ["Some saved items were kept in state.skipped-1.json."];
  assert.equal(startupNoticeVisible(shown, []), true);
  const dismissed = dismissStartupNotice(shown, []);
  assert.equal(startupNoticeVisible(shown, dismissed), false);
  assert.equal(startupNoticeVisible([], dismissed), false);
  assert.equal(startupNoticeVisible(undefined, dismissed), false);
});

test("a notice that arrives after the band was closed brings it back", () => {
  const first = ["Kiwix could not remove some PDF temporary files."];
  const dismissed = dismissStartupNotice(first, []);
  const later = [...first, "Import cancelled because Kiwix closed, moved to the background, or switched privacy mode. Add the archive again to import it."];
  assert.equal(startupNoticeVisible(later, dismissed), true);
  // Withdrawing that newer notice does not reopen what was already closed.
  const again = dismissStartupNotice(later, dismissed);
  assert.equal(startupNoticeVisible(first, again), false);
  assert.deepEqual(again, later);
});
