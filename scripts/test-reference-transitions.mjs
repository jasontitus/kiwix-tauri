import assert from "node:assert/strict";
import test from "node:test";
import { translateBookmarkDrafts, translateLocation, translateTabs } from "../src/reference-transitions.ts";

const transition = (revision, fromBookId, toBookId) => ({
  revision, fromBookId, toBookId, transactionId: `tx-${revision}`,
  fromInstallationId: `old-${revision}`, toInstallationId: `new-${revision}`,
});
const tab = () => ({ id: "live-tab", bookId: "old", path: "unsaved/article?draft=1#selection", title: "Unsaved", scroll: 91,
  back: [{bookId:"old",path:"previous#anchor",title:"Back",scroll:13}],
  forward:[{bookId:"other",path:"unrelated",title:"Forward",scroll:25}] });

test("live, closed and pending references preserve unsaved locations and selection identity", () => {
  const original = tab();
  const transitions = [transition(1,"old","new")];
  const [live] = translateTabs([original],transitions,0);
  const [closed] = translateTabs([original],transitions,0);
  const pending = translateLocation(original.back[0],transitions,0);
  assert.equal(live.id,original.id);
  assert.equal(live.path,original.path);
  assert.equal(live.scroll,91);
  assert.equal(live.bookId,"new");
  assert.equal(live.back[0].bookId,"new");
  assert.equal(live.back[0].path,"previous#anchor");
  assert.equal(live.forward[0],original.forward[0]);
  assert.deepEqual(closed,live);
  assert.equal(pending.bookId,"new");
  assert.equal(original.bookId,"old");
});

test("ordered replay handles multiple cutovers and explicit rollback without stale-ID resurrection", () => {
  const transitions = [transition(1,"old","new"),transition(2,"new","third"),transition(3,"third","old")];
  const [replayed] = translateTabs([tab()],transitions,0);
  assert.equal(replayed.bookId,"old");
  assert.equal(replayed.back[0].bookId,"old");
  const [delayed] = translateTabs([{...tab(),bookId:"new",back:[]}],transitions,1);
  assert.equal(delayed.bookId,"old");
  assert.equal(delayed.path,tab().path);
});

test("acknowledged revisions do not replay older transitions over fresh navigation", () => {
  const current = {...tab(),bookId:"old",back:[]};
  const transitions = [transition(1,"old","new"),transition(2,"new","old")];
  assert.equal(translateTabs([current],transitions,2)[0],current);
  assert.equal(translateLocation(current,transitions,2),current);
});

test("empty library tabs and unrelated archives remain intact", () => {
  const empty = {...tab(),bookId:null,back:[],forward:[]};
  const unrelated = {...empty,id:"unrelated",bookId:"other"};
  const result = translateTabs([empty,unrelated],[transition(1,"old","new")],0);
  assert.equal(result[0],empty);
  assert.equal(result[1],unrelated);
});

test("unsaved bookmark draft keys follow cutover and rollback without changing draft text", () => {
  const document = "article?draft=1";
  const draft = {title:"Unsaved title",note:"A note not yet saved",tags:"work, keep"};
  const unrelated = {title:"Other",note:"Different archive",tags:"other"};
  const originalKey = JSON.stringify(["old",document]);
  const replacementKey = JSON.stringify(["new",document]);
  const otherKey = JSON.stringify(["other",document]);
  const drafts = {[originalKey]:draft,[otherKey]:unrelated};
  const cutover = translateBookmarkDrafts(drafts,[transition(1,"old","new")],0);
  assert.equal(cutover[replacementKey],draft);
  assert.equal(cutover[otherKey],unrelated);
  assert.equal(cutover[originalKey],undefined);
  assert.deepEqual(draft,{title:"Unsaved title",note:"A note not yet saved",tags:"work, keep"});
  const rollback = translateBookmarkDrafts(cutover,[transition(1,"old","new"),transition(2,"new","old")],1);
  assert.equal(rollback[originalKey],draft);
  assert.equal(rollback[replacementKey],undefined);
  assert.equal(rollback[otherKey],unrelated);
});
