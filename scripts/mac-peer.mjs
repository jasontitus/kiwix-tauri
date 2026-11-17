#!/usr/bin/env node
// Real, isolated browser-backed Mac peer. This is not native WKWebView evidence.
import fs from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import http from 'node:http';
import { isDeepStrictEqual } from 'node:util';

const SELF = fileURLToPath(import.meta.url);
const ROOT = path.dirname(path.dirname(SELF));
const LOCK = path.join(ROOT, 'package-lock.json');
const PUPPETEER = '25.16.0';
const HEX = /^[a-f0-9]{64}$/;
const NONCE = /^[a-f0-9]{32}$/;
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;
const REQUEST_KEYS = ['schema', 'mode', 'nonce', 'deadline', 'mobile_platform', 'mobile_device', 'mobile_artifact_sha256', 'archive_id', 'file_name', 'title', 'expected_text', 'source', 'size', 'sha256', 'peer'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const requireThat = (condition, message) => { if (!condition) throw new Error(message); };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function boundedText(value, name, maximum = 4096) {
  requireThat(typeof value === 'string' && value.length > 0 && value.length <= maximum && !/[\x00-\x1f\x7f]/.test(value), `Invalid ${name}`);
}
function exactKeys(value, keys) {
  requireThat(value && typeof value === 'object' && !Array.isArray(value) && isDeepStrictEqual(Object.keys(value).sort(), [...keys].sort()), 'Unexpected or missing fields');
}
export function validateRequest(value, now = Date.now() / 1000) {
  exactKeys(value, REQUEST_KEYS);
  requireThat(value.schema === 2 && ['publish', 'receive', 'observe'].includes(value.mode), 'Invalid request schema/mode');
  requireThat(NONCE.test(value.nonce) && HEX.test(value.mobile_artifact_sha256) && HEX.test(value.sha256), 'Invalid frozen identity');
  requireThat(Number.isFinite(value.deadline) && value.deadline > now && value.deadline <= now + 600, 'Deadline must be within ten minutes');
  requireThat(UUID.test(value.archive_id), 'Invalid archive UUID');
  requireThat(['ios', 'android'].includes(value.mobile_platform), 'Invalid mobile platform');
  const devicePattern = value.mobile_platform === 'ios' ? /^(?:[A-Fa-f0-9]{40}|[A-Fa-f0-9]{8}-[A-Fa-f0-9]{16})$/ : /^[A-Za-z0-9][A-Za-z0-9._:-]{0,159}$/;
  requireThat(typeof value.mobile_device === 'string' && devicePattern.test(value.mobile_device), 'Invalid physical device identity');
  for (const key of ['title', 'expected_text', 'source', 'file_name', 'peer']) boundedText(value[key], key, key === 'peer' ? 160 : 4096);
  // `source` names a file on the macOS peer, so validate it as a POSIX path
  // whatever host prepares or tests the request.
  requireThat(path.posix.isAbsolute(value.source) && path.posix.normalize(value.source) === value.source, 'Source must be an absolute normalized path');
  requireThat(path.basename(value.file_name) === value.file_name && !['.', '..'].includes(value.file_name) && value.file_name.endsWith('.zim'), 'Invalid archive file name');
  requireThat(Number.isSafeInteger(value.size) && value.size > 0, 'Invalid fixture size');
  return Object.freeze({ ...value });
}
export function parseFrozenRequest(bytes, sha256, now = Date.now() / 1000) {
  requireThat(HEX.test(sha256) && digest(bytes) === sha256, 'Frozen request bytes changed');
  return validateRequest(JSON.parse(bytes), now);
}
export function validateControl(value, request, started) {
  requireThat(value && value.nonce === request.nonce, 'Control nonce mismatch');
  if (value.command === 'stop') {
    exactKeys(value, ['command', 'nonce', 'complete']);
    requireThat(typeof value.complete === 'boolean', 'Invalid stop completion flag');
  } else {
    exactKeys(value, ['command', 'nonce', 'publisher_id', 'library_generation']);
    requireThat(['receive', 'observe'].includes(value.command) && value.command === request.mode && !started, `Unexpected ${value.command} command`);
    requireThat(UUID.test(value.publisher_id) && Number.isSafeInteger(value.library_generation) && value.library_generation > 0, 'Invalid publisher binding');
  }
  return value;
}
export function selectPeer(status, binding) {
  const peers = status.peers.filter(peer => peer.transport === 'quic-library' && peer.peerId === binding.publisher_id && peer.libraryGeneration === binding.library_generation);
  requireThat(peers.length <= 1, 'Ambiguous publisher discovery');
  if (!peers.length) return null;
  requireThat(NONCE.test(peers[0].librarySessionId), 'Missing real library session');
  return peers[0];
}
export function assertObservedSource(status, publication, selectedPeer) {
  // The app may refresh libraryGeneration during a route/rebind transition for
  // the same live share. Publisher + session + peer identity remain exact; a
  // generation refresh alone is not a different source.
  const peers = status.peers.filter(peer => peer.transport === 'quic-library'
    && peer.peerId === publication.publisher_id && peer.librarySessionId === publication.session_id);
  requireThat(peers.length <= 1, 'Ambiguous publisher discovery');
  requireThat(peers.length === 1 && peers[0].id === selectedPeer.id, 'Observed source disappeared or changed publication');
}
export function selectOffer(inventory, request, publication) {
  requireThat(inventory.sessionId === publication.session_id && UUID.test(inventory.receiptId), 'Browse receipt/session mismatch');
  const candidates = inventory.books.filter(book => book.archiveId === request.archive_id || book.fileName === request.file_name);
  requireThat(candidates.length === 1, 'Missing or ambiguous frozen archive');
  const book = candidates[0];
  requireThat(book.archiveId === request.archive_id && book.fileName === request.file_name && book.title === request.title && book.size === request.size, 'Shared fixture metadata mismatch');
  const url = new URL(book.downloadIdentity);
  requireThat(url.protocol === 'kiwix-library:' && url.host === 'public' && !url.search && !url.hash && isDeepStrictEqual(url.pathname.split('/').slice(1).map(decodeURIComponent), [publication.session_id, publication.publisher_id, book.id]), 'Offer publication identity mismatch');
  return book;
}
export function assertPreserved(before, after) {
  requireThat(isDeepStrictEqual(before.settings, after.settings), 'Settings changed during peer window');
  for (const key of ['books', 'tabs', 'bookmarks']) for (const item of before[key]) {
    requireThat(after[key].some(current => isDeepStrictEqual(current, item)), `Preexisting ${key} changed`);
  }
}
export function assertObserverPreserved(before, after, downloads, status) {
  assertPreserved(before, after);
  for (const key of ['books', 'tabs', 'bookmarks']) requireThat(isDeepStrictEqual(before[key], after[key]), `Observer changed ${key}`);
  requireThat(downloads.length === 0, 'Observer admitted a download');
  requireThat(!status.librarySharing?.running && status.seeds.length === 0, 'Observer published local content');
}
async function canonicalFile(name) {
  requireThat(path.isAbsolute(name) && path.normalize(name) === name && await fs.realpath(name) === name, 'Artifact must be a canonical absolute non-symlink file');
  const handle = await fs.open(name, constants.O_RDONLY | constants.O_NOFOLLOW);
  try { requireThat((await handle.stat()).isFile(), 'Artifact is not a regular file'); } finally { await handle.close(); }
  return name;
}
async function readBytes(name, maximum = 1024 * 1024) {
  await canonicalFile(name);
  const handle = await fs.open(name, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const before = await handle.stat();
    requireThat(before.isFile() && before.size <= maximum, 'Artifact exceeds size bound');
    const bytes = await handle.readFile();
    const after = await handle.stat();
    requireThat(before.ino === after.ino && before.size === after.size && before.mtimeMs === after.mtimeMs && before.ctimeMs === after.ctimeMs, 'Artifact changed during read');
    return bytes;
  } finally { await handle.close(); }
}
async function fileIdentity(name) {
  await canonicalFile(name);
  const handle = await fs.open(name, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const before = await handle.stat();
    const hash = createHash('sha256');
    for await (const bytes of handle.createReadStream({ autoClose: false })) hash.update(bytes);
    const after = await handle.stat();
    const named = await fs.lstat(name);
    requireThat(before.ino === after.ino && before.dev === after.dev && before.size === after.size && before.mtimeMs === after.mtimeMs && before.ctimeMs === after.ctimeMs && named.ino === after.ino && named.dev === after.dev, 'File authority changed during full hashing');
    return { path: name, sha256: hash.digest('hex'), size: after.size, device: after.dev, inode: after.ino, mtime_ms: after.mtimeMs, ctime_ms: after.ctimeMs };
  } finally { await handle.close(); }
}
async function pin(name) { const identity = await fileIdentity(name); return { path: name, sha256: identity.sha256 }; }
async function privateDirectory(name) {
  requireThat(path.isAbsolute(name) && path.normalize(name) === name, 'Output must be absolute and normalized');
  requireThat(await fs.realpath(path.dirname(name)) === path.dirname(name), 'Output parent must be canonical');
  await fs.mkdir(name, { mode: 0o700 }); // Existing output/profile is never reusable.
  return name;
}
async function jsonFile(directory, name, value) {
  const temp = path.join(directory, `${name}.part`);
  await fs.writeFile(temp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600, flag: 'wx' });
  await fs.rename(temp, path.join(directory, name));
}
async function runtime() {
  const metadata = JSON.parse(await readBytes(path.join(ROOT, 'node_modules/puppeteer-core/package.json')));
  requireThat(metadata.version === PUPPETEER, 'Pinned puppeteer-core is not installed');
  requireThat(Number(process.versions.node.split('.')[0]) >= 22, 'Node >=22 required');
  return { node_major: Number(process.versions.node.split('.')[0]), puppeteer_core: PUPPETEER, platform: process.platform, arch: process.arch, node: await pin(await fs.realpath(process.execPath)) };
}
async function prepare(options) {
  requireThat(process.platform === 'darwin', 'Mac peer requires macOS');
  const receipt = { schema: 1, helper: await pin(SELF), lockfile: await pin(LOCK), executable: await pin(options.executable), browser: await pin(options.browser), runtime: await runtime() };
  await privateDirectory(options.output);
  await jsonFile(options.output, 'prepare.json', receipt);
  process.stdout.write(`${JSON.stringify({ manifest_path: path.join(options.output, 'prepare.json') })}\n`);
}
async function loadPrepared(name, sha256) {
  requireThat(HEX.test(sha256), 'Frozen prepare SHA-256 required');
  const bytes = await readBytes(name);
  requireThat(digest(bytes) === sha256, 'Frozen prepare bytes changed');
  const receipt = JSON.parse(bytes);
  exactKeys(receipt, ['schema', 'helper', 'lockfile', 'executable', 'browser', 'runtime']);
  requireThat(receipt.schema === 1 && receipt.helper.path === SELF && receipt.lockfile.path === LOCK, 'Prepared helper identity mismatch');
  for (const key of ['helper', 'lockfile', 'executable', 'browser']) {
    exactKeys(receipt[key], ['path', 'sha256']);
    requireThat(HEX.test(receipt[key].sha256) && isDeepStrictEqual(await pin(receipt[key].path), receipt[key]), `Prepared ${key} changed`);
  }
  requireThat(process.platform === 'darwin' && isDeepStrictEqual(receipt.runtime, await runtime()), 'Prepared runtime changed');
  return receipt;
}
function ownedProcess(executable, args, onLine) {
  const child = spawn(executable, args, { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const state = { child, exited: false, code: null, signal: null, error: null };
  state.done = new Promise(resolve => {
    child.once('error', error => { state.error = error; state.exited = true; resolve(); });
    child.once('exit', (code, signal) => { state.exited = true; state.code = code; state.signal = signal; resolve(); });
  });
  for (const [stream, kind] of [[child.stdout, 'stdout'], [child.stderr, 'stderr']]) {
    let pending = '';
    stream.setEncoding('utf8');
    stream.on('data', text => {
      pending += text;
      if (pending.length > 65536) { state.error = new Error('Child log line exceeds bound'); pending = ''; }
      for (;;) { const end = pending.indexOf('\n'); if (end < 0) break; const line = pending.slice(0, end); pending = pending.slice(end + 1); onLine(kind, line, state); }
    });
  }
  return state;
}
async function reap(owned, signal = 'SIGINT') {
  if (!owned) return { graceful: true, reaped: true };
  if (!owned.child.pid) return { graceful: false, reaped: owned.exited, code: owned.code, signal: owned.signal };
  const alive = () => {
    try { process.kill(-owned.child.pid, 0); return true; }
    catch (error) { if (error.code === 'ESRCH') return false; throw error; }
  };
  const send = value => { try { process.kill(-owned.child.pid, value); } catch (error) { if (error.code !== 'ESRCH') throw error; } };
  const drain = async milliseconds => {
    const deadline = Date.now() + milliseconds;
    while ((!owned.exited || alive()) && Date.now() < deadline) await sleep(50);
  };
  if (!owned.exited) send(signal);
  await drain(6000);
  const graceful = owned.exited && owned.code === 0;
  if (!owned.exited || alive()) { send('SIGTERM'); await drain(2000); }
  if (!owned.exited || alive()) { send('SIGKILL'); await drain(2000); }
  return { graceful, reaped: owned.exited && !alive(), code: owned.code, signal: owned.signal };
}
export function parseReady(line) {
  const match = /^KIWIX_READY (http:\/\/kiwix-shell-[a-f0-9]{32}\.localhost:([0-9]+)\/#token=([a-f0-9]{32}))$/.exec(line);
  requireThat(match && match[0] === line && Number(match[2]) > 0 && Number(match[2]) <= 65535, 'Invalid exact KIWIX_READY');
  const url = new URL(match[1]);
  return { url: match[1], origin: url.origin, host: url.host, port: Number(match[2]), token: match[3] };
}
function apiClient(endpoint, timeout) {
  return (command, args = {}) => new Promise((resolve, reject) => {
    const body = JSON.stringify({ command, args });
    const request = http.request({ hostname: '127.0.0.1', port: endpoint.port, path: '/api', method: 'POST', headers: { Host: endpoint.host, Origin: endpoint.origin, Authorization: `Bearer ${endpoint.token}`, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) } }, response => {
      let text = '';
      response.setEncoding('utf8');
      response.on('data', chunk => { text += chunk; if (text.length > 16 * 1024 * 1024) response.destroy(new Error('API response exceeds bound')); });
      response.on('error', reject);
      response.on('end', () => { try { const parsed = JSON.parse(text); requireThat(response.statusCode === 200 && Object.hasOwn(parsed, 'result'), `API ${command} failed: ${parsed.error || response.statusCode}`); resolve(parsed.result); } catch (error) { reject(error); } });
    });
    request.on('error', reject);
    request.setTimeout(timeout(), () => request.destroy(new Error(`API ${command} timed out`)));
    request.end(body);
  });
}
function controls(request, fail, ready, completeAllowed) {
  const queue = [];
  let buffer = '', started = false, stopped = false;
  process.stdin.setEncoding('utf8');
  const data = chunk => {
    try {
      buffer += chunk;
      requireThat(buffer.length <= 4096, 'Control input exceeds bound');
      for (;;) {
        const end = buffer.indexOf('\n'); if (end < 0) break;
        const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
        requireThat(!stopped, 'Control after stop');
        const value = validateControl(JSON.parse(line), request, started);
        requireThat(ready(), 'Control arrived before readiness');
        if (value.command === 'stop' && (!value.complete || !completeAllowed())) fail(new Error('Controller stopped incomplete work'));
        if (['receive', 'observe'].includes(value.command)) started = true;
        if (value.command === 'stop') stopped = true;
        requireThat(queue.length < 2, 'Too many controls'); queue.push(value);
      }
    } catch (error) { fail(error); }
  };
  const end = () => { if (!stopped || buffer.length) fail(new Error('Control pipe closed before cleanup')); };
  process.stdin.on('data', data); process.stdin.on('end', end); process.stdin.resume();
  return { queue, close() { process.stdin.off('data', data); process.stdin.off('end', end); process.stdin.pause(); } };
}
async function run(options) {
  const prepared = await loadPrepared(options.prepared, options['prepared-sha256']);
  const bytes = await readBytes(options.request, 32768);
  const request = parseFrozenRequest(bytes, options['request-sha256']);
  await privateDirectory(options.output);
  const output = options.output;
  const envelope = { ...request, request_sha256: digest(bytes), prepared_sha256: options['prepared-sha256'], prepared_identity: prepared };
  const result = { ...envelope, status: 'failed', cleanup_complete: false, proof_surface: 'real-app-headless-chromium-not-native-wkwebview' };
  let app, chrome, browser, page, api, endpoint, baseline, sourceIdentity, retainedBook;
  const observed = [];
  let failure = null, cleanup = false, ready = false, witnessed = false, completed = false, cleanupDeadline = 0;
  const fail = error => { if (!failure) failure = error; };
  const control = controls(request, fail, () => ready, () => request.mode === 'publish' || (request.mode === 'observe' && witnessed));
  const interrupted = () => fail(new Error('Interrupted by controller'));
  process.on('SIGINT', interrupted); process.on('SIGTERM', interrupted);
  const remaining = () => Math.max(1, Math.min(cleanup ? 3000 : 10000, (cleanup ? cleanupDeadline : request.deadline * 1000) - Date.now()));
  const check = () => {
    if (failure && !cleanup) throw failure;
    requireThat(Date.now() < (cleanup ? cleanupDeadline : request.deadline * 1000), 'Mac peer deadline expired');
    for (const child of [app, chrome]) if (child) { if (child.error) throw child.error; requireThat(!child.exited, 'Owned child exited before cleanup'); }
  };
  const until = async (action, label, interval = 150) => { for (;;) { check(); const value = await action(); if (value) return value; await sleep(interval); check(); } };
  const capture = async name => {
    check();
    await page.screenshot({ path: path.join(output, `${name}.png`), fullPage: true });
    await fs.writeFile(path.join(output, `${name}.txt`), await page.evaluate(() => document.body.innerText), { mode: 0o600, flag: 'wx' });
  };
  const click = async (selector, text) => {
    const handle = await until(async () => {
      for (const candidate of await page.$$(selector)) {
        const matches = await candidate.evaluate((element, value) => !element.disabled && (element.innerText.trim() === value || element.getAttribute('aria-label') === value) && element.getClientRects().length > 0, text);
        if (matches) return candidate;
        await candidate.dispose();
      }
      return null;
    }, `UI ${text}`);
    check(); await handle.click(); await handle.dispose();
  };
  const publicationFiles = async () => {
    const publisher_id = (await readBytes(path.join(output, 'profile/localswarm/peer-id'), 64)).toString();
    const generation = (await readBytes(path.join(output, 'profile/localswarm/library-generation'), 32)).toString();
    requireThat(UUID.test(publisher_id) && /^[1-9][0-9]*$/.test(generation) && Number.isSafeInteger(Number(generation)), 'Invalid owned profile publication');
    return { publisher_id, library_generation: Number(generation) };
  };
  try {
    if (request.mode !== 'observe') {
      sourceIdentity = await fileIdentity(request.source);
      requireThat(sourceIdentity.sha256 === request.sha256 && sourceIdentity.size === request.size, 'Frozen source differs from request');
    }
    await jsonFile(output, 'request.json', envelope);
    for (const directory of ['profile', 'browser-profile', 'downloads']) await fs.mkdir(path.join(output, directory), { mode: 0o700 });
    app = ownedProcess(prepared.executable.path, ['--headless', '--data-dir', path.join(output, 'profile'), '--port', '0'], (kind, line, state) => {
      if (kind === 'stdout' && line.startsWith('KIWIX_READY')) {
        try { requireThat(!endpoint, 'Duplicate readiness'); endpoint = parseReady(line); } catch (error) { state.error = error; }
      }
    });
    await until(() => endpoint, 'Exact app readiness');
    api = apiClient(endpoint, remaining);
    const initial = await api('snapshot');
    requireThat(initial.books.length === 0 && (await api('downloads')).length === 0, 'Fresh profile is not empty');
    if (request.mode !== 'observe') await api('save_settings', { settings: { downloadDirectory: path.join(output, 'downloads'), sharingName: request.peer, locale: 'en' }, privacyGeneration: initial.privacyGeneration });
    baseline = await api('snapshot');
    requireThat(!baseline.settings.privateMode && !baseline.settings.readOnly && (request.mode === 'observe' || baseline.settings.downloadDirectory === path.join(output, 'downloads')), 'Owned configuration failed');
    await jsonFile(output, 'baseline.json', baseline);
    let websocket;
    chrome = ownedProcess(prepared.browser.path, ['--headless=new', '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', `--user-data-dir=${path.join(output, 'browser-profile')}`, '--no-first-run', '--no-default-browser-check', '--disable-sync', '--disable-background-networking', '--disable-features=MediaRouter', '--autoplay-policy=user-gesture-required', '--host-resolver-rules=MAP *.localhost 127.0.0.1', 'about:blank'], (kind, line, state) => {
      if (kind === 'stderr' && line.startsWith('DevTools listening on ')) {
        const match = /^DevTools listening on (ws:\/\/127\.0\.0\.1:[0-9]+\/devtools\/browser\/[A-Za-z0-9-]+)$/.exec(line);
        if (match && !websocket) websocket = match[1]; else state.error = new Error('Invalid owned browser readiness');
      }
    });
    await until(() => websocket, 'Owned browser readiness');
    const puppeteer = await import('puppeteer-core');
    browser = await puppeteer.connect({ browserWSEndpoint: websocket, defaultViewport: { width: 1280, height: 900 }, protocolTimeout: remaining() });
    const context = await browser.createBrowserContext();
    page = await context.newPage();
    page.setDefaultTimeout(remaining()); page.setDefaultNavigationTimeout(remaining());
    page.on('response', response => {
      if (response.url() !== `${endpoint.origin}/api`) return;
      try {
        const sent = JSON.parse(response.request().postData() || '{}');
        if (['shared_library_browse', 'shared_library_download', 'open_article', 'localswarm_stop_library'].includes(sent.command)) {
          const row = { command: sent.command, args: sent.args, body: null, error: null }; observed.push(row);
          response.json().then(body => { row.body = body; }, () => { row.error = true; });
        }
      } catch { fail(new Error('Malformed real browser API request')); }
    });
    await page.goto(endpoint.url, { waitUntil: 'domcontentloaded' });
    await until(() => page.$('.main-nav'), 'Real shell navigation');
    if (request.mode === 'publish') {
      await api('import_archives', { paths: [request.source], privacyGeneration: baseline.privacyGeneration });
      const installed = await api('snapshot');
      requireThat(installed.books.length === 1, 'Source intake did not create exactly one installation');
      retainedBook = installed.books[0];
      requireThat(retainedBook.id === request.archive_id && retainedBook.title === request.title && retainedBook.available && UUID.test(retainedBook.installationId) && retainedBook.path === request.source && retainedBook.managed === false && !retainedBook.sourceReference, 'Source installation authority differs from intake');
      requireThat(isDeepStrictEqual(sourceIdentity, await fileIdentity(retainedBook.path)), 'Source changed during intake');
      result.installed_identity = { ...retainedBook, file: sourceIdentity };
      await page.reload({ waitUntil: 'domcontentloaded' });
      await click('.main-nav button', 'Nearby');
      await click('.localswarm-share-card button', 'Share all');
      const status = await until(async () => { const value = await api('localswarm_status'); return value.librarySharing?.running ? value : null; }, 'Real Share all');
      requireThat(NONCE.test(status.librarySharing.sessionId) && status.librarySharing.libraryCount === 1, 'Invalid source sharing snapshot');
      result.publication = { ...await publicationFiles(), session_id: status.librarySharing.sessionId };
      result.source_identity = { ...retainedBook, file: sourceIdentity };
      result.source_status = status;
      await capture('publisher-ready');
    } else {
      await click('.main-nav button', 'Nearby');
    }
    await until(async () => (await api('localswarm_status')).discovery, 'Owned discovery ready');
    await capture('ready');
    await jsonFile(output, 'ready.json', { ...envelope, status: 'ready', publication: result.publication ?? null, source_identity: result.source_identity ?? null, proof_surface: result.proof_surface });
    ready = true;
    if (request.mode === 'publish') {
      const stop = await until(() => control.queue.shift(), 'Nonce-bound publisher stop');
      requireThat(stop.command === 'stop' && stop.complete, 'Publisher stopped without paired completion');
      requireThat(isDeepStrictEqual(sourceIdentity, await fileIdentity(request.source)), 'Source changed during publication');
      const live = await api('localswarm_status');
      requireThat(live.librarySharing?.running && live.librarySharing.sessionId === result.publication.session_id && isDeepStrictEqual(await publicationFiles(), { publisher_id: result.publication.publisher_id, library_generation: result.publication.library_generation }), 'Publication changed before paired completion');
      result.paired_complete = true;
    } else {
      const command = await until(() => control.queue.shift(), 'Source-bound browse command');
      requireThat(command.command === request.mode, 'Peer stopped before source observation');
      const selected = await until(async () => selectPeer(await api('localswarm_status'), command), 'Unique source discovery');
      result.publication = { publisher_id: command.publisher_id, library_generation: command.library_generation, session_id: selected.librarySessionId };
      result.selected_peer = selected;
      const chooseId = await until(() => page.evaluate(id => {
        const heading = [...document.querySelectorAll('.localswarm-peer strong')].find(node => node.id.endsWith(`-peer-${id}`));
        return heading?.closest('article')?.querySelector('button')?.id ?? null;
      }, selected.id), 'Bound discovery has no ordinary UI card');
      const choose = await page.$(`[id=${JSON.stringify(chooseId)}]`); requireThat(choose, 'Bound peer UI vanished'); await choose.click(); await choose.dispose();
      const browse = await until(() => observed.find(row => row.command === 'shared_library_browse' && row.body), 'Real UI browse response');
      requireThat(browse.args.peerId === selected.id && browse.body.result, 'UI browsed another publisher');
      const inventory = browse.body.result;
      const offer = selectOffer(inventory, request, result.publication);
      result.inventory = inventory; result.offer = offer;
      if (request.mode === 'observe') {
        const liveObservation = async () => {
          const status = await api('localswarm_status');
          assertObservedSource(status, result.publication, selected);
          assertObserverPreserved(baseline, await api('snapshot'), await api('downloads'), status);
          check();
        };
        await liveObservation();
        await capture('source-observed');
        check();
        await jsonFile(output, 'source-observed.json', { ...envelope, status: 'observed', publication: result.publication, selected_peer: selected, inventory, offer, proof_surface: result.proof_surface });
        witnessed = true;
        const stop = await until(async () => {
          await liveObservation();
          return control.queue.shift();
        }, 'Nonce-bound observer stop', 500);
        requireThat(stop.command === 'stop' && stop.complete, 'Observer stopped without paired completion');
        await liveObservation();
        result.paired_complete = true;
        result.observed_source_stable = true;
      } else {
        const priorJobs = await api('downloads');
        requireThat(priorJobs.length === 0 && !(await api('snapshot')).books.some(book => book.id === request.archive_id), 'Preexisting transfer or installation cannot prove this run');
        const rowIndex = inventory.books.findIndex(book => book.id === offer.id);
        await until(() => page.$$('.localswarm-library-book').then(rows => rows.length === inventory.books.length), 'Exact inventory UI');
        const rows = await page.$$('.localswarm-library-book');
        const download = await rows[rowIndex].$('button');
        requireThat(download && await download.evaluate(node => node.innerText.trim() === 'Download' && !node.disabled), 'Frozen archive lacks Download control');
        await capture('inventory'); await download.click(); await download.dispose();
        const admission = await until(() => observed.find(row => row.command === 'shared_library_download' && row.body), 'Real UI download receipt');
        requireThat(admission.args.receiptId === inventory.receiptId && admission.args.bookId === offer.id && admission.body.result, 'UI download receipt mismatch');
        const jobId = admission.body.result.id;
        requireThat(UUID.test(jobId), 'Invalid accepted job identity');
        result.job = await until(async () => {
          if (control.queue.length) throw new Error('Receiver stopped during transfer');
          const jobs = await api('downloads');
          requireThat(jobs.length === 1 && jobs[0].id === jobId, 'Ambiguous or changed accepted job');
          const job = jobs[0];
          requireThat(job.url === offer.downloadIdentity && job.transport === 'quic-library' && job.fileName === request.file_name && path.dirname(job.destination) === path.join(output, 'downloads'), 'Job publication/destination mismatch');
          requireThat(!['failed', 'cancelled', 'paused'].includes(job.status) && !job.error && !job.importError, 'Owned transfer failed or paused');
          return job.status === 'complete' && job.importState === 'imported' ? job : null;
        }, 'Transferred and imported bytes');
        const snapshot = await api('snapshot');
        const books = snapshot.books.filter(book => book.id === request.archive_id);
        requireThat(books.length === 1 && snapshot.books.length === 1, 'Received installation is missing or ambiguous');
        retainedBook = books[0];
        requireThat(retainedBook.managed === true && retainedBook.available && UUID.test(retainedBook.installationId) && retainedBook.path === result.job.destination && result.job.imports?.length === 1 && result.job.imports[0].receipt === result.job.id && result.job.imports[0].state === 'imported' && isDeepStrictEqual(result.job.imports[0].paths, [retainedBook.path]) && !retainedBook.sourceReference, 'New installation lacks owned managed-file authority');
        const identity = await fileIdentity(retainedBook.path);
        requireThat(identity.sha256 === request.sha256 && identity.size === request.size && result.job.downloaded === request.size && result.job.total === request.size, 'Full received-byte proof failed');
        result.installed_identity = { ...retainedBook, file: identity };
        result.receivedhash = identity.sha256; result.receivedsize = identity.size; result.managed = true;
        // Download focuses Downloads and unmounts Nearby; use ordinary navigation.
        await click('.main-nav button', 'Library');
        await click('.book-card button, .book-list-row button, button[aria-label]', `Open ${retainedBook.title}`);
        const reader = await until(() => observed.find(row => row.command === 'open_article' && row.args.bookId === retainedBook.id && row.body?.result), 'Real UI reader admission');
        const location = reader.body.result;
        requireThat(location.bookId === retainedBook.id && typeof location.url === 'string' && typeof location.origin === 'string', 'Reader archive authority mismatch');
        const frameElement = await until(() => page.$('iframe.rw-archive-frame:not(.is-pending)'), 'Readable real reader frame');
        const frame = await frameElement.contentFrame(); requireThat(frame, 'Reader frame unavailable');
        await until(async () => {
          requireThat(frame.url() === location.url && new URL(frame.url()).origin === location.origin, 'Reader frame URL/origin changed');
          const content = await frame.evaluate(() => ({ text: document.body.innerText, visible: document.visibilityState === 'visible' }));
          if (!content.visible || !content.text.includes(request.expected_text)) return null;
          return content;
        }, 'Rendered article readability');
        const text = await frame.evaluate(() => document.body.innerText);
        await fs.writeFile(path.join(output, 'reader.txt'), text, { mode: 0o600, flag: 'wx' });
        await page.screenshot({ path: path.join(output, 'reader.png'), fullPage: true });
        const active = await until(async () => { const current = await api('snapshot'); const tab = current.tabs.find(tab => tab.id === current.activeTab); return current.activeSurface === 'reader' && tab?.bookId === request.archive_id ? current : null; }, 'Persisted active reader UUID');
        requireThat(active.books.some(book => book.id === request.archive_id && book.installationId === retainedBook.installationId && book.path === retainedBook.path) && frame.url() === location.url, 'Active reader installation/frame identity changed');
        result.activebookid = active.tabs.find(tab => tab.id === active.activeTab).bookId;
        result.readable_ui = true; result.reader = { book_id: location.bookId, origin: location.origin, url: frame.url(), text_path: path.join(output, 'reader.txt'), screenshot_path: path.join(output, 'reader.png') };
        const finalPeer = selectPeer(await api('localswarm_status'), command);
        requireThat(finalPeer?.librarySessionId === result.publication.session_id, 'Source publication changed before completion');
      }
    }
    completed = true;
  } catch (error) { fail(error); }
  finally {
    cleanup = true;
    cleanupDeadline = Date.now() + 20000;
    control.close();
    const cleanupErrors = [];
    const attempt = async action => { try { return await action(); } catch (error) { cleanupErrors.push(error.message); return null; } };
    if (api && app && !app.exited) {
      // Fresh isolated profile owns every share/discovery. Never cancel/delete jobs.
      if (request.mode === 'publish' && result.publication) {
        await attempt(async () => {
          requireThat(page && chrome && !chrome.exited, 'Stop sharing UI unavailable');
          await click('.localswarm-share-card button', 'Stop sharing');
          const response = await until(() => observed.find(row => row.command === 'localswarm_stop_library' && row.body), 'Real Stop sharing response');
          requireThat(response.body.result && !response.body.result.librarySharing?.running, 'Stop sharing UI did not retire publication');
          result.stopped_via_ui = true;
        });
      }
      // API retirement also covers failure during Share all before readiness.
      if (request.mode !== 'observe') await attempt(() => api('localswarm_stop_library'));
      const snapshot = await attempt(() => api('snapshot'));
      if (snapshot) await attempt(() => api('localswarm_discovery', { enabled: false, privacyGeneration: snapshot.privacyGeneration }));
      const stopped = await attempt(() => api('localswarm_status'));
      result.stopped = Boolean(stopped && !stopped.librarySharing?.running && !stopped.discovery && stopped.seeds.length === 0);
      if (request.mode === 'publish') result.publication_stopped = Boolean(result.publication && stopped && !stopped.librarySharing?.running && result.stopped_via_ui);
      if (!result.stopped) cleanupErrors.push('Owned publication/discovery not proven stopped');
      const after = await attempt(() => api('snapshot'));
      if (baseline && after) await attempt(async () => {
        if (request.mode === 'observe') {
          requireThat(stopped, 'Observer retirement status unavailable');
          assertObserverPreserved(baseline, after, await api('downloads'), stopped);
        } else assertPreserved(baseline, after);
        result.preserved_before_books_settings_tabs = true;
        await jsonFile(output, 'after.json', after);
      });
    }
    if (browser) await attempt(async () => { await Promise.race([browser.close(), sleep(5000).then(() => { throw new Error('Browser close timed out'); })]); });
    result.browser_cleanup = await attempt(() => reap(chrome, 'SIGTERM'));
    result.app_cleanup = await attempt(() => reap(app));
    if (request.mode === 'publish') result.owned_sharing_stopped = Boolean(result.stopped && result.app_cleanup?.reaped);
    if (sourceIdentity) await attempt(async () => requireThat(isDeepStrictEqual(sourceIdentity, await fileIdentity(request.source)), 'Source file not preserved'));
    if (retainedBook) await attempt(async () => {
      const state = JSON.parse(await readBytes(path.join(output, 'profile/state.json'), 16 * 1024 * 1024));
      const saved = state.books.find(book => book.id === retainedBook.id);
      requireThat(saved && saved.installationId === retainedBook.installationId && saved.path === retainedBook.path, 'Installed archive not retained after shutdown');
      if (request.mode === 'receive') {
        requireThat(state.consumedAcquisitions?.includes(result.job.id), 'Received installation lost its consumed acquisition receipt');
        requireThat(isDeepStrictEqual(result.installed_identity.file, await fileIdentity(retainedBook.path)), 'Received file changed after shutdown');
        requireThat(state.tabs.some(tab => tab.bookId === request.archive_id), 'Received reader tab not retained after shutdown');
      }
    });
    result.cleanup_errors = cleanupErrors;
    result.cleanup_complete = cleanupErrors.length === 0 && Boolean(result.browser_cleanup?.reaped && result.app_cleanup?.reaped && result.app_cleanup.graceful && result.stopped && result.preserved_before_books_settings_tabs);
    result.status = completed && !failure && result.cleanup_complete ? 'complete' : 'failed';
    if (failure) result.error = failure.message.replace(/#token=[a-f0-9]+/g, '#token=[redacted]');
    if (result.status !== 'complete' && !result.error) result.error = 'Required cleanup/preservation proof failed';
    await jsonFile(output, 'result.json', result);
    process.off('SIGINT', interrupted); process.off('SIGTERM', interrupted);
    if (result.status !== 'complete') process.exitCode = 1;
  }
}
function options(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]; requireThat(flag.startsWith('--'), 'Expected named option');
    const key = flag.slice(2); requireThat(!Object.hasOwn(parsed, key), 'Duplicate option');
    if (key === 'prepare-only') parsed[key] = true;
    else { const value = argv[++index]; requireThat(value && !value.startsWith('--'), 'Missing option value'); parsed[key] = value; }
  }
  exactKeys(parsed, parsed['prepare-only'] ? ['prepare-only', 'executable', 'browser', 'output'] : ['prepared', 'prepared-sha256', 'request', 'request-sha256', 'output']);
  return parsed;
}
if (process.argv[1] && path.resolve(process.argv[1]) === SELF) {
  process.umask(0o077);
  try { const args = options(process.argv.slice(2)); if (args['prepare-only']) await prepare(args); else await run(args); }
  catch (error) { process.stderr.write(`Mac peer: ${error.message}\n`); process.exitCode = 1; }
}
