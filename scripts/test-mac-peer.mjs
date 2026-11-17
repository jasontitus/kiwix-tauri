import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { validateRequest, parseFrozenRequest, validateControl, selectPeer, selectOffer, assertObservedSource, assertObserverPreserved, assertPreserved, parseReady } from './mac-peer.mjs';

const publisher = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';
const archive = '11111111-2222-3333-4444-555555555555';
const session = 'a'.repeat(32);
const fixture = () => ({ schema: 2, mode: 'receive', nonce: 'b'.repeat(32), deadline: 1100, mobile_platform: 'ios', mobile_device: '00008110-000A11CE0000001E', mobile_artifact_sha256: 'c'.repeat(64), archive_id: archive, file_name: 'fixture.zim', title: 'Real archive', expected_text: 'Actual article content', source: '/private/fixtures/fixture.zim', size: 4096, sha256: 'd'.repeat(64), peer: 'Owned Mac' });
const binding = { publisher_id: publisher, library_generation: 7, session_id: session };
const peer = () => ({ id: 'real-discovery-service', transport: 'quic-library', peerId: publisher, libraryGeneration: 7, librarySessionId: session });
const offer = () => ({ id: 'opaque-publication-id', archiveId: archive, fileName: 'fixture.zim', title: 'Real archive', size: 4096, downloadIdentity: `kiwix-library://public/${session}/${publisher}/opaque-publication-id` });
const inventory = () => ({ receiptId: archive, sessionId: session, books: [offer()] });

test('frozen request hashes exact bytes before JSON parsing, not equivalent JSON', () => {
  const bytes = Buffer.from(JSON.stringify(fixture()));
  const hash = createHash('sha256').update(bytes).digest('hex');
  assert.equal(parseFrozenRequest(bytes, hash, 1000).archive_id, archive);
  assert.throws(() => parseFrozenRequest(Buffer.concat([bytes, Buffer.from('\n')]), hash, 1000), /Frozen request/);
  assert.throws(() => parseFrozenRequest(Buffer.from('{'), hash, 1000), /Frozen request/);
  assert.throws(() => parseFrozenRequest(bytes, hash.toUpperCase(), 1000), /Frozen request/);
});

test('request rejects scope expansion, identity/path tricks and unbounded lifetimes', () => {
  assert.ok(Object.isFrozen(validateRequest(fixture(), 1000)));
  for (const patch of [{ deadline: 1000 }, { deadline: 1601 }, { deadline: Infinity }, { source: '/private/fixtures/../fixture.zim' }, { file_name: '../fixture.zim' }, { file_name: 'fixture.zim/other' }, { size: 0 }, { size: Number.MAX_SAFE_INTEGER + 1 }, { archive_id: publisher.toUpperCase() }, { mobile_device: 'sim' }, { peer: 'name\ncommand' }, { mode: 'shell' }, { nonce: 'a'.repeat(31) }, { arbitrary_command: 'delete' }]) {
    assert.throws(() => validateRequest({ ...fixture(), ...patch }, 1000), JSON.stringify(patch));
  }
  const missing = fixture(); delete missing.expected_text;
  assert.throws(() => validateRequest(missing, 1000));
});

test('mobile identity cutover admits real Android serials without loosening iOS or retaining schema1 aliases', () => {
  assert.equal(validateRequest({ ...fixture(), mobile_platform: 'android', mobile_device: 'R5TESTSER01' }, 1000).mobile_device, 'R5TESTSER01');
  for (const patch of [{ schema: 1 }, { mobile_platform: 'simulator' }, { mobile_device: 'R5TESTSER01' }, { mobile_platform: 'android', mobile_device: '-serial' }, { mobile_platform: 'android', mobile_device: 'serial\ncommand' }, { mobile_platform: 'android', mobile_device: 'a'.repeat(161) }, { mobile_artifact_sha256: 'not-reviewed' }, { sender_device: '00008110-000A11CE0000001E' }, { iphone_manifest_sha256: 'c'.repeat(64) }]) {
    assert.throws(() => validateRequest({ ...fixture(), ...patch }, 1000), JSON.stringify(patch));
  }
});

test('controls accept only nonce-bound one-shot source binding and strict stop boolean', () => {
  const request = fixture();
  const receive = { command: 'receive', nonce: request.nonce, publisher_id: publisher, library_generation: 7 };
  assert.equal(validateControl(receive, request, false), receive);
  assert.throws(() => validateControl(receive, request, true), /Unexpected receive/);
  assert.throws(() => validateControl(receive, { ...request, mode: 'publish' }, false), /Unexpected receive/);
  for (const patch of [{ nonce: 'f'.repeat(32) }, { library_generation: 0 }, { library_generation: '7' }, { publisher_id: publisher.toUpperCase() }, { path: '/foreign' }, { command: 'delete' }]) {
    assert.throws(() => validateControl({ ...receive, ...patch }, request, false));
  }
  assert.equal(validateControl({ command: 'stop', nonce: request.nonce, complete: false }, request, false).complete, false);
  assert.throws(() => validateControl({ command: 'stop', nonce: request.nonce, complete: 'true' }, request, false));
  assert.throws(() => validateControl({ command: 'stop', nonce: request.nonce, complete: true, pid: 123 }, request, false));
});

test('observe authority is frozen, mode-specific and one-shot without granting receive', () => {
  const request = validateRequest({ ...fixture(), mode: 'observe' }, 1000);
  const command = { command: 'observe', nonce: request.nonce, publisher_id: publisher, library_generation: 7 };
  assert.equal(validateControl(command, request, false), command);
  assert.throws(() => validateControl(command, request, true), /Unexpected observe/);
  assert.throws(() => validateControl(command, fixture(), false), /Unexpected observe/);
  assert.throws(() => validateControl({ ...command, command: 'receive' }, request, false), /Unexpected receive/);
  for (const patch of [{ nonce: 'f'.repeat(32) }, { publisher_id: publisher.toUpperCase() }, { library_generation: 0 }, { library_generation: '7' }, { session_id: session }, { source: '/foreign.zim' }]) {
    assert.throws(() => validateControl({ ...command, ...patch }, request, false));
  }
  assert.throws(() => validateRequest({ ...request, deadline: 1000 }, 1000), /Deadline/);
  const bytes = Buffer.from(JSON.stringify(request));
  assert.equal(parseFrozenRequest(bytes, createHash('sha256').update(bytes).digest('hex'), 1000).mode, 'observe');
});

test('discovery requires unique exact source generation, never a display label', () => {
  assert.equal(selectPeer({ peers: [] }, binding), null);
  assert.equal(selectPeer({ peers: [{ ...peer(), libraryGeneration: 6 }] }, binding), null);
  assert.equal(selectPeer({ peers: [{ ...peer(), peerId: archive }] }, binding), null);
  assert.equal(selectPeer({ peers: [peer()] }, binding).id, 'real-discovery-service');
  assert.throws(() => selectPeer({ peers: [peer(), { ...peer(), id: 'duplicate-service' }] }, binding), /Ambiguous/);
  assert.throws(() => selectPeer({ peers: [{ ...peer(), librarySessionId: undefined }] }, binding), /session/);
});

test('receipt binds observed session, opaque publication and every fixture metadata field', () => {
  assert.equal(selectOffer(inventory(), fixture(), binding).id, 'opaque-publication-id');
  assert.throws(() => selectOffer({ ...inventory(), sessionId: 'e'.repeat(32) }, fixture(), binding), /session/);
  assert.throws(() => selectOffer({ ...inventory(), books: [offer(), offer()] }, fixture(), binding), /ambiguous/);
  for (const patch of [{ archiveId: publisher }, { fileName: 'wrong.zim' }, { title: 'Spoof' }, { size: 4095 }, { downloadIdentity: `kiwix-library://public/${'e'.repeat(32)}/${publisher}/opaque-publication-id` }, { downloadIdentity: `kiwix-library://public/${session}/${archive}/opaque-publication-id` }, { downloadIdentity: `kiwix-library://public/${session}/${publisher}/wrong` }, { downloadIdentity: `${offer().downloadIdentity}?extra=1` }, { downloadIdentity: `${offer().downloadIdentity}#fragment` }]) {
    assert.throws(() => selectOffer({ ...inventory(), books: [{ ...offer(), ...patch }] }, fixture(), binding), JSON.stringify(patch));
  }
});

test('observation binds a real browse to the unique discovery session and fails on lost or replaced source', () => {
  const request = { ...fixture(), mode: 'observe' };
  const selected = selectPeer({ peers: [peer()] }, binding);
  const publication = { publisher_id: binding.publisher_id, library_generation: binding.library_generation, session_id: selected.librarySessionId };
  const offered = selectOffer(inventory(), request, publication);
  assert.equal(offered.downloadIdentity, offer().downloadIdentity);
  assert.doesNotThrow(() => assertObservedSource({ peers: [peer()] }, publication, selected));
  // A route/rebind refresh may advance the publisher generation while the
  // exact publisher/session/peer identity remains live.
  assert.doesNotThrow(() => assertObservedSource({ peers: [{ ...peer(), libraryGeneration: 8 }] }, publication, selected));
  for (const peers of [[], [{ ...peer(), librarySessionId: 'f'.repeat(32) }], [{ ...peer(), peerId: archive }], [{ ...peer(), id: 'replacement-service' }], [peer(), { ...peer(), id: 'duplicate-service' }]]) {
    assert.throws(() => assertObservedSource({ peers }, publication, selected));
  }
  assert.throws(() => selectOffer({ ...inventory(), sessionId: 'f'.repeat(32) }, request, publication), /session/);
});

test('observer preservation forbids local intake, reader changes, downloads or publication', () => {
  const baseline = { settings: { sharingName: 'Keep', downloadDirectory: '/user/Downloads' }, books: [], tabs: [], bookmarks: [] };
  const status = { librarySharing: { running: false }, seeds: [] };
  assert.doesNotThrow(() => assertObserverPreserved(baseline, structuredClone(baseline), [], status));
  for (const key of ['books', 'tabs', 'bookmarks']) {
    assert.throws(() => assertObserverPreserved(baseline, { ...baseline, [key]: [{ id: archive }] }, [], status), /Observer changed/);
  }
  assert.throws(() => assertObserverPreserved(baseline, { ...baseline, settings: { ...baseline.settings, sharingName: 'Changed' } }, [], status), /Settings/);
  assert.throws(() => assertObserverPreserved(baseline, baseline, [{ id: archive, status: 'complete' }], status), /download/);
  assert.throws(() => assertObserverPreserved(baseline, baseline, [], { ...status, librarySharing: { running: true } }), /published/);
  assert.throws(() => assertObserverPreserved(baseline, baseline, [], { ...status, seeds: [{ id: archive }] }), /published/);
});

test('preservation retains exact existing installation, settings and tabs while allowing additions', () => {
  const baseline = { settings: { downloadDirectory: '/owned/downloads' }, books: [{ id: archive, installationId: publisher, path: '/existing.zim' }], tabs: [{ id: 'tab', bookId: archive, path: 'Article' }], bookmarks: [{ id: 'bookmark', title: 'Keep' }] };
  assert.doesNotThrow(() => assertPreserved(baseline, structuredClone(baseline)));
  assert.doesNotThrow(() => assertPreserved(baseline, { ...structuredClone(baseline), books: [...baseline.books, { id: publisher }] }));
  for (const key of ['books', 'tabs', 'bookmarks']) assert.throws(() => assertPreserved(baseline, { ...structuredClone(baseline), [key]: [] }), /Preexisting/);
  assert.throws(() => assertPreserved(baseline, { ...structuredClone(baseline), settings: { downloadDirectory: '/user/Downloads' } }), /Settings/);
  assert.throws(() => assertPreserved(baseline, { ...structuredClone(baseline), books: [{ ...baseline.books[0], installationId: archive }] }), /Preexisting/);
});

test('readiness admits only exact authenticated loopback shell line with a real bound port', () => {
  const line = `KIWIX_READY http://kiwix-shell-${session}.localhost:12345/#token=${'b'.repeat(32)}`;
  const parsed = parseReady(line);
  assert.equal(parsed.host, `kiwix-shell-${session}.localhost:12345`);
  assert.equal(parsed.port, 12345);
  for (const invalid of ['Forwarded to running profile', line.replace(':12345/', ':0/'), line.replace(':12345/', ':65536/'), line.replace('.localhost', '.example.com'), `${line}\n`, `prefix ${line}`, line.replace('#token=', '?token=')]) assert.throws(() => parseReady(invalid), /KIWIX_READY/);
});
