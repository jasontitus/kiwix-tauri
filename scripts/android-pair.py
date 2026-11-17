#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Frozen Fire -> Samsung sender-hotspot window (never installs the target app).

Off device:
  python3 scripts/android-pair.py prepare --config CONFIG.json --java-home JDK --output NEWDIR
Hardware owner only, after ordinary managed-source import and reviewed target installs:
  python3 scripts/android-pair.py run --prepared NEWDIR/prepare.json \
    --prepared-sha256 HEX --sender-device FIRE --receiver-device SAMSUNG \
    --start-device-window --output NEWRUN

Config has exactly schema:1, topology:"sender-hotspot", sender_device,
receiver_device, sender_prepared, receiver_prepared, source, archive_id,
file_name, title, expected_text, peer, window_seconds (120..600). Paths are
absolute canonical existing files. Each device's independently prepared helper
receipt freezes its own crash authority; no crash review is inherited across devices.
Readable screenshot content requires human review. The consumed receive journal
proves its own session and source publisher, NOT an independently copied source session.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import sysconfig
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / 'scripts'
HELPER = SCRIPTS / 'android-nearby.py'
HEX = re.compile(r'[0-9a-f]{64}')
CONFIG_KEYS = {'schema', 'topology', 'sender_device', 'receiver_device', 'sender_prepared',
               'receiver_prepared', 'source', 'archive_id', 'file_name', 'title',
               'expected_text', 'peer', 'window_seconds'}
SOURCE_KEYS = ('archive_id', 'file_name', 'title', 'peer', 'size', 'sha256')
TARGET = 'org.kiwix.workbench'
RUNNER = 'org.kiwix.nearby.test/org.kiwix.nearby.NearbyInstrumentation'

TIMING = None


def timed_operation(args, operation, category, prepared=None):
    global TIMING
    args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
    # Frozen run admission precedes this first transitive helper import.
    spec = importlib.util.spec_from_file_location("testing_timing", SCRIPTS / "testing_timing.py")
    module = importlib.util.module_from_spec(spec)
    timing_path = SCRIPTS / "testing_timing.py"
    content = timing_path.read_bytes()
    expected = next(entry['sha256'] for entry in prepared['inputs'] if entry['path'] == str(timing_path)) if prepared else pin(timing_path)['sha256']
    require(hashlib.sha256(content).hexdigest() == expected, 'Frozen timing dependency changed before import')
    exec(compile(content, str(timing_path), 'exec'), module.__dict__)
    timing = TIMING = module.TimingLog(
        args.output.resolve(), os.environ.get("KIWIX_TIMING_ITERATION") or uuid.uuid4().hex, "android-pair")
    previous_iteration = os.environ.get("KIWIX_TIMING_ITERATION")
    os.environ["KIWIX_TIMING_ITERATION"] = timing.iteration_id
    try:
        with timing.span(category, "prepare" if prepared is None else "paired-window") as measured:
            code = operation(args) if prepared is None else operation(args, prepared)
            if code:
                measured.fail()
            return code
    finally:
        timing.close()
        TIMING = None
        if previous_iteration is None:
            os.environ.pop("KIWIX_TIMING_ITERATION", None)
        else:
            os.environ["KIWIX_TIMING_ITERATION"] = previous_iteration


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def canonical_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except (ValueError, AttributeError):
        return False


def read_json(path, maximum=16 * 1024 * 1024):
    with Path(path).open('rb') as stream:
        raw = stream.read(maximum + 1)
    require(len(raw) <= maximum, 'JSON exceeds bound: ' + str(path))
    value = json.loads(raw)
    require(isinstance(value, dict), 'JSON object required: ' + str(path))
    return value, raw


def pin(path):
    path = Path(path)
    require(path.is_absolute() and path == path.resolve(strict=True), 'Canonical input required: ' + str(path))
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, 'rb') as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode), 'Regular input required: ' + str(path))
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
        after = os.fstat(stream.fileno())
    identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
    require(identity(before) == identity(after) == identity(path.lstat()), 'Input changed while hashing: ' + str(path))
    return {'path': str(path), 'size': before.st_size, 'sha256': digest.hexdigest()}


def tree_pin(path, python=False):
    """Include added files, link targets and native/JAR runtime dependencies too."""
    root = Path(path).resolve(strict=True)
    entries = []

    def visit(directory, ancestors):
        real = directory.resolve(strict=True)
        require(real not in ancestors, 'Runtime directory link cycle: ' + str(directory))
        for child in sorted(directory.iterdir()):
            if python and child.name in ('site-packages', 'dist-packages'):
                continue
            relative = str(child.relative_to(root))
            if child.is_symlink():
                entries.append([relative, 'link', os.readlink(child)])
            if child.is_dir():
                entries.append([relative, 'directory'])
                visit(child, ancestors | {real})
            else:
                entries.append([relative, pin(child.resolve(strict=True))])
    visit(root, set())
    return {'path': str(root), 'python': python,
            'sha256': hashlib.sha256(json.dumps(entries, sort_keys=True).encode()).hexdigest()}


def save(path, value):
    # Publish once, atomically: helpers must never observe a partial gate.
    path = Path(path)
    temporary = path.with_name(path.name + '.pending')
    with temporary.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    try:
        os.link(temporary, path)
    finally:
        temporary.unlink()


def validate_config(config):
    require(set(config) == CONFIG_KEYS and type(config['schema']) is int and config['schema'] == 1,
            'Unexpected or missing config fields')
    require(config['topology'] == 'sender-hotspot', 'Explicit sender-hotspot topology required')
    for role in ('sender', 'receiver'):
        value = config[role + '_device']
        require(isinstance(value, str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._:-]{0,159}', value),
                'Explicit physical ' + role + ' serial required')
    require(config['sender_device'] != config['receiver_device'], 'Sender and receiver physical serials must differ')
    require(type(config['window_seconds']) is int and 120 <= config['window_seconds'] <= 600,
            'Window must be 120..600 seconds including cleanup')
    require(canonical_uuid(config['archive_id']), 'Canonical archive UUID required')
    require(isinstance(config['file_name'], str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._ -]{0,239}\.zim', config['file_name']),
            'Single ASCII wire .zim filename required')
    for key in ('title', 'peer', 'expected_text'):
        value = config[key]
        require(isinstance(value, str) and value.strip() and len(value) <= (4096 if key == 'expected_text' else 500)
                and not any(ord(c) < 32 or ord(c) == 127 for c in value), 'Invalid ' + key)
    for key in ('source', 'sender_prepared', 'receiver_prepared'):
        require(isinstance(config[key], str) and str(Path(config[key]).resolve(strict=True)) == config[key],
                'Absolute canonical ' + key + ' required')
    require(config['sender_prepared'] != config['receiver_prepared'], 'Independent per-device helper receipts required')


def validate_fixture(config, source):
    actual = pin(Path(config['source']))
    require(actual['size'] == source['size'] and actual['sha256'] == source['sha256'], 'Frozen source bytes changed')
    with Path(config['source']).open('rb') as stream:
        header = stream.read(24)
    require(len(header) == 24 and header[:4] == b'ZIM\x04'
            and str(uuid.UUID(bytes=header[8:24])) == config['archive_id'], 'Source real ZIM header/archive identity mismatch')


def validate_tool_paths(tools):
    require(set(tools) == {'aapt2', 'd8', 'zipalign', 'apksigner', 'adb', 'android_jar'},
            'Unexpected helper SDK tool set')
    for value in tools.values():
        # Preserve the executable pathname, not just the old symlink target.
        pin(Path(value))
    sdk = Path(tools['android_jar']).parents[2]
    build_tools = sdk / 'build-tools' / Path(tools['apksigner']).parent.name
    for name in ('aapt2', 'd8', 'zipalign', 'apksigner'):
        require(Path(tools[name]) == build_tools / name,
                'Helper reconstructed SDK pathname differs: ' + name)
    require(Path(tools['android_jar']) == sdk / 'platforms/android-36/android.jar',
            'Helper reconstructed SDK platform differs')


def validate_helper(path):
    receipt, raw = read_json(path)
    require(receipt.get('schema') == 1 and receipt.get('target_package') == TARGET
            and receipt.get('instrumentation') == RUNNER, 'Invalid helper target/runner preparation')
    require(receipt.get('host_source_sha256') == pin(HELPER)['sha256'], 'Helper preparation has stale host source')
    require(receipt.get('timing_tool_sha256') == pin(SCRIPTS / 'testing_timing.py')['sha256'],
            'Helper preparation has stale or missing timing dependency')
    expected_sources = {name: pin((SCRIPTS / 'android-nearby' / name).resolve())['sha256']
                        for name in ('AndroidManifest.xml', 'NearbyInstrumentation.java', 'inspect.js')}
    require(receipt.get('source_sha256') == expected_sources, 'Helper preparation has stale instrumentation sources')
    for key in ('target_apk_sha256', 'tool_apk_sha256', 'certificate_sha256'):
        require(isinstance(receipt.get(key), str) and HEX.fullmatch(receipt[key]), 'Invalid helper ' + key)
    crash = receipt.get('reviewed_crash_sha256')
    require(isinstance(crash, str) and (not crash or HEX.fullmatch(crash)), 'Missing independent crash authority')
    for key in ('target_apk', 'tool_apk'):
        require(pin(Path(receipt[key]))['sha256'] == receipt[key + '_sha256'], 'Prepared APK changed: ' + key)
    tools = receipt.get('tools', {})
    validate_tool_paths(tools)
    return receipt, hashlib.sha256(raw).hexdigest()


def prepare(args):
    return timed_operation(args, prepare_impl, 'host_prepare')


def prepare_impl(args):
    config_path = args.config.resolve(strict=True)
    config, raw = read_json(config_path)
    validate_config(config)
    fixture = pin(Path(config['source']))
    source = {key: config[key] for key in SOURCE_KEYS if key in config}
    source.update(size=fixture['size'], sha256=fixture['sha256'])
    validate_fixture(config, source)
    identities = {}
    paths = {Path(__file__).resolve(), HELPER, SCRIPTS / 'testing_timing.py',
             SCRIPTS / 'test_android_pair.py', SCRIPTS / 'test_android_nearby.py',
             config_path, Path(config['source']), Path(sys.executable).resolve()}
    trees = {(Path(sysconfig.get_path('stdlib')).resolve(), True)}
    java_home = args.java_home.resolve(strict=True)
    require((java_home / 'bin/java').is_file() and (java_home / 'lib').is_dir(), 'Actual Java home (not /usr/bin/java launcher) required')
    trees.add((java_home, False))
    for role in ('sender', 'receiver'):
        receipt, digest = validate_helper(config[role + '_prepared'])
        identities[role] = {'prepared': config[role + '_prepared'], 'prepared_sha256': digest, 'receipt': receipt}
        paths.add(Path(config[role + '_prepared']))
        for key in ('target_apk', 'tool_apk'):
            paths.add(Path(receipt[key]))
        for value in receipt['tools'].values():
            paths.add(Path(value).resolve(strict=True))
        # Freeze launchers AND the Java verifier/native SDK dependencies they load.
        trees.add((Path(receipt['tools']['apksigner']).parent.resolve(), False))
        trees.add((Path(receipt['tools']['adb']).parent.resolve(), False))
    a, b = (identities[role]['receipt'] for role in ('sender', 'receiver'))
    require(a['target_apk_sha256'] == b['target_apk_sha256'] and a['certificate_sha256'] == b['certificate_sha256'],
            'Both devices require the same reviewed release and signing identity')
    for name in a['source_sha256']:
        paths.add(SCRIPTS / 'android-nearby' / name)
    libdir = Path(sysconfig.get_config_var('LIBDIR') or Path(sys.executable).resolve().parent)
    library = sysconfig.get_config_var('LDLIBRARY')
    if library and (libdir / library).is_file():
        paths.add((libdir / library).resolve())
    # CPython probes a versioned stdlib ZIP even when absent. Freeze absence
    # as well, so a newly added archive cannot precede the reviewed .py tree.
    zip_paths = {Path(value).resolve() for value in sys.path if value.endswith('.zip')}
    zip_paths.add((Path(sysconfig.get_path('stdlib')).parent /
                   ('python%d%d.zip' % sys.version_info[:2])).resolve())
    python_zips = []
    for path in sorted(zip_paths, key=str):
        present = path.exists()
        python_zips.append({'path': str(path), 'present': present})
        if present:
            paths.add(path)
    inputs = [pin(path.resolve(strict=True)) for path in sorted(paths, key=str)]
    runtime_trees = [tree_pin(path, python) for path, python in sorted(trees, key=lambda value: str(value[0]))]
    require(pin(config_path)['sha256'] == hashlib.sha256(raw).hexdigest(), 'Config changed during prepare')
    value = {'schema': 1, 'status': 'ready', 'config': config,
             'config_file': {'path': str(config_path), 'sha256': hashlib.sha256(raw).hexdigest()},
             'source': source, 'identities': identities, 'inputs': inputs, 'runtime_trees': runtime_trees,
             'python_zips': python_zips, 'java_home': str(java_home),
             'python': {'executable': str(Path(sys.executable).resolve()), 'version': sys.version}}
    path = args.output.resolve() / 'prepare.json'
    save(path, value)
    print(json.dumps({'status': 'ready', 'prepared': str(path), 'prepared_sha256': pin(path)['sha256'], 'device_contact': False}))
    return 0


def load_frozen(path, digest, sender, receiver):
    require(isinstance(digest, str) and HEX.fullmatch(digest), 'Exact --prepared-sha256 required before device contact')
    value, raw = read_json(path)
    require(hashlib.sha256(raw).hexdigest() == digest, 'Frozen preparation bytes changed; refusing device contact')
    require(value.get('schema') == 1 and value.get('status') == 'ready', 'Invalid frozen pair preparation')
    config = value['config']
    validate_config(config)
    require(sender == config['sender_device'] and receiver == config['receiver_device'], 'Physical serials differ from frozen pair')
    require(value['python'] == {'executable': str(Path(sys.executable).resolve()), 'version': sys.version}, 'Python runtime changed')
    for entry in value['inputs']:
        require(pin(Path(entry['path'])) == entry, 'Frozen input changed: ' + entry['path'])
    require(pin(SCRIPTS / 'testing_timing.py') in value['inputs'],
            'Frozen timing dependency missing or changed; prepare again')
    for entry in value['runtime_trees']:
        require(tree_pin(entry['path'], entry['python']) == entry, 'Frozen runtime/import tree changed: ' + entry['path'])
    for entry in value['python_zips']:
        require(Path(entry['path']).exists() == entry['present'], 'Python import ZIP presence changed: ' + entry['path'])
    actual, raw_config = read_json(value['config_file']['path'])
    require(actual == config and hashlib.sha256(raw_config).hexdigest() == value['config_file']['sha256'], 'Frozen config changed')
    validate_fixture(config, value['source'])
    for role in ('sender', 'receiver'):
        identity = value['identities'][role]
        actual, digest = validate_helper(config[role + '_prepared'])
        require(actual == identity['receipt'] and digest == identity['prepared_sha256'], 'Frozen helper identity changed: ' + role)
    return value


def validate_baseline(value):
    require(isinstance(value, dict) and value.get('sta_connected') is False and value.get('no_ap_verified') is True
            and value.get('current_ap_verified') is False, 'Initial topology requires disconnected station/current no AP')


def validate_publication(value):
    require(isinstance(value, dict) and set(value) == {'publisher_id', 'library_generation'}
            and canonical_uuid(value['publisher_id']) and type(value['library_generation']) is int
            and 0 < value['library_generation'] < 2**63, 'Missing persisted actual publisher/generation')
    return value


def validate_source(value, source):
    require(value.get('source_kind') == 'managed' and value.get('source_bytes_verified') is True
            and value.get('managed') is True and value.get('receivedhash') == source['sha256']
            and value.get('receivedsize') == source['size'] and value.get('source_sha256') == source['sha256']
            and canonical_uuid(value.get('installation_id')), 'Source is not an independently fully hashed available managed installation')


def validate_received_job(value, source, publication):
    journal = value.get('received_journal', {})
    job = journal.get('matched_job', {})
    match = re.fullmatch(r'kiwix-library://public/([0-9a-f]{32})/([0-9a-f-]{36})/([0-9a-f-]{36})', job.get('url', ''))
    require(journal.get('status') == 'matched' and journal.get('match_count') == 1
            and isinstance(journal.get('new_rows'), list) and job in journal['new_rows']
            and match and match.group(2) == publication['publisher_id'] and match.group(3) == source['archive_id']
            and canonical_uuid(job.get('id')) and job.get('journal_key') == job['id']
            and job.get('matches_independent_received_bytes') is True and job.get('transport') == 'quic-library'
            and job.get('error') is None and job.get('importError') is None and job.get('status') == 'complete'
            and job.get('fileName') == source['file_name'] and job.get('downloaded') == source['size']
            and job.get('total') == source['size'] and job.get('publication') == {'size': source['size'], 'sha256': source['sha256']}
            and isinstance(value.get('managed_path'), str) and bool(value['managed_path'])
            and job.get('destination') == value['managed_path'] and canonical_uuid(value.get('installation_id'))
            and job.get('consumed_acquisition') is True and job.get('installation_id') == value['installation_id'],
            'Receive job lacks original consumed acquisition/source publisher/full-byte authority')
    return job


class Window:
    def __init__(self, output, seconds):
        self.output, self.seconds, self.started = output, seconds, time.monotonic()
        self.deadline = int(time.time() + seconds - 15)

    def remaining(self, cleanup=False):
        left = self.seconds - (1 if cleanup else 65) - (time.monotonic() - self.started)
        require(left > 0, 'Finite paired window reached; clean owned work without retry')
        return left


class AndroidPeer:
    def __init__(self, window, prepared, role, nonce, sender):
        self.window, self.role = window, role
        self.mode = 'publish' if role == 'sender' else 'receive'
        self.source = prepared['source']
        self.identity = prepared['identities'][role]['receipt']
        self.output = window.output / role
        self.gate = window.output / (role + '-route.json')
        self.binding = {key: self.source[key] for key in SOURCE_KEYS}
        self.binding.update(schema=1, device=prepared['config'][role + '_device'], mode=self.mode,
                            topology='sender-hotspot', nonce=nonce, deadline=window.deadline,
                            sender_device=sender, reviewed_crash_sha256=self.identity['reviewed_crash_sha256'])
        identity = prepared['identities'][role]
        tools = self.identity['tools']
        argv = [str(Path(sys.executable).resolve()), '-I', '-S', '-B', str(HELPER),
                '--prepared', identity['prepared'], '--prepared-sha256', identity['prepared_sha256'],
                '--route-gate', str(self.gate), '--output', str(self.output), '--adb', tools['adb'],
                '--android-sdk', str(Path(tools['android_jar']).parents[2]),
                '--build-tools', Path(tools['apksigner']).parent.name]
        for key, value in self.binding.items():
            if key != 'schema' and (key != 'reviewed_crash_sha256' or value):
                argv.extend(['--' + key.replace('_', '-'), str(value)])
        # Do not admit JAVA_TOOL_OPTIONS, dynamic-loader hooks, BASH_ENV or
        # inherited adb endpoint overrides as unpinned executable inputs.
        environment = {'HOME': str(Path.home()), 'JAVA_HOME': prepared['java_home'],
                       'PYTHONDONTWRITEBYTECODE': '1', 'LANG': 'en_US.UTF-8',
                       'PATH': str(Path(prepared['java_home']) / 'bin') + os.pathsep + '/usr/bin:/bin'}
        if "KIWIX_TIMING_ITERATION" in os.environ:
            environment["KIWIX_TIMING_ITERATION"] = os.environ["KIWIX_TIMING_ITERATION"]
        self.log = (window.output / (role + '.log')).open('xb')
        try:
            self.process = subprocess.Popen(argv, env=environment, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        except BaseException:
            self.log.close()
            raise
        self.readiness = None
        self.instrumentation = None

    def receipt(self, name):
        path = self.output / name
        if not path.exists():
            return None
        value, _ = read_json(path)
        require(all(value.get(key) == expected for key, expected in self.binding.items()),
                self.role + ' receipt changed exact physical/source/run/topology/crash binding')
        return value

    def identity_check(self, value):
        installed = value.get('installed_identity', {})
        require(installed.get('package') == TARGET and installed.get('debuggable') is False
                and installed.get('apk_sha256') == self.identity['target_apk_sha256']
                and installed.get('certificate_sha256') == self.identity['certificate_sha256'],
                self.role + ' installed reviewed release identity mismatch')

    def fail_if_exited(self):
        if self.process.poll() is not None:
            value = self.receipt('result.json')
            raise RuntimeError(self.role + ': ' + str((value or {}).get('error') or
                               'helper exited without a successful bound result (see ' + str(self.log.name) + ')'))

    def admit_instrumentation(self, value):
        self.identity_check(value)
        require(value.get('status') == 'ready' and value.get('no_ap_at_instrumentation_ready') is True,
                self.role + ' lacks pre-route instrumentation admission')
        validate_baseline(value.get('initial_radio'))
        validate_baseline(value.get('station_admission'))
        if self.role == 'sender':
            require(canonical_uuid(value.get('publisher_id')), 'Sender pre-route persisted publisher identity missing')
        self.instrumentation = value

    def release_route(self, sender_network_sha256=None):
        require(self.instrumentation is not None, 'Cannot route before instrumentation readiness')
        gate = {key: self.binding[key] for key in ('schema', 'nonce', 'topology', 'device', 'mode', 'deadline')}
        if self.role == 'receiver':
            require(isinstance(sender_network_sha256, str) and HEX.fullmatch(sender_network_sha256),
                    'Receiver route requires actual source hotspot network identity')
            gate['sender_network_sha256'] = sender_network_sha256
        save(self.gate, gate)
        if self.role == 'receiver':
            self.sender_network_sha256 = sender_network_sha256

    def publisher_radio_check(self, value):
        radio = value.get('route_radio', {})
        require(radio.get('current_ap_verified') is True and radio.get('no_ap_verified') is False
                and radio.get('sta_connected') is False, 'Sender lacks current OS sender-AP proof')
        api = value.get('installed_identity', {}).get('android_os', {}).get('api')
        if type(api) is int and api == 28:
            require(radio.get('source') == 'API28 dumpsys wifi current WifiStateMachinePrime'
                    and radio.get('legacy_current_ap_manager') is True and radio.get('ap_frequency_mhz') == []
                    and isinstance(radio.get('dump_sha256'), str) and HEX.fullmatch(radio['dump_sha256']),
                    'API28 sender lacks reviewed current-manager AP evidence')
        else:
            frequencies = radio.get('ap_frequency_mhz')
            require(type(api) is int and api >= 29 and isinstance(frequencies, list) and frequencies
                    and all(type(value) is int and value > 0 for value in frequencies),
                    'Modern sender lacks current AP frequency evidence')

    def admit_publisher(self, value):
        self.identity_check(value)
        require(value.get('status') == 'ready' and value.get('publisher_ready') is True
                and value.get('product_owned_ap_at_publish') is True, 'Publisher lacks actual owned hotspot readiness')
        validate_source(value, self.source)
        publication = validate_publication(value.get('publication'))
        require(publication['publisher_id'] == self.instrumentation['publisher_id'], 'Persisted publisher changed across route admission')
        self.publisher_radio_check(value)
        require(isinstance(value.get('hotspot_network_sha256'), str) and HEX.fullmatch(value['hotspot_network_sha256']),
                'Publisher lacks actual native hotspot network hash')
        self.readiness = value
        return publication

    def cleanup_proof(self, value):
        require(self.process.poll() is not None and value is not None and value.get('cleanup_complete') is True,
                self.role + ' owned process/reap/cleanup unconfirmed')
        self.identity_check(value)
        require(value.get('final_radio', {}).get('no_ap_verified') is True, self.role + ' current no-AP cleanup proof missing')
        route_started = value.get(self.role + '_route_started')
        if route_started is False:
            require(value.get('final_radio', {}).get('sta_connected') is False,
                    self.role + ' pre-route cleanup lacks disconnected station proof')
        elif self.role == 'receiver':
            require(route_started is True and value.get('receiver_join_retired') is True
                    and value.get('receiver_consumer_retired') is True,
                    'Receiver retained join/consumer retirement unconfirmed')
        else:
            require(route_started is True and value.get('owned_sharing_stopped') is True,
                    'Sender exact owned sharing stop unconfirmed')
        return value

    def completed(self, publication=None):
        if self.process.poll() is None:
            return None
        value = self.receipt('result.json')
        require(value is not None, self.role + ' exited without exact result receipt; see ' + str(self.log.name))
        require(value.get('status') == 'complete' and self.process.returncode == 0,
                self.role + ': ' + str(value.get('error') or 'helper failed; see ' + str(self.log.name)))
        self.cleanup_proof(value)
        require(value.get('preserved_before_books_settings_tabs') is True and value.get('crash_report_preserved') is True
                and value.get('preserved_before_jobs') is True,
                self.role + ' preservation proof missing')
        reviewed = self.identity['reviewed_crash_sha256']
        if reviewed:
            closed = value.get('reviewed_crash_closed', {})
            require(closed.get('sha256') == reviewed and closed.get('ordinary_ui_close') is True
                    and all((self.output / name).is_file() for name in ('crash-before-close.png', 'crash-after-close.png')),
                    self.role + ' independent reviewed crash close proof missing')
        if self.role == 'sender':
            validate_source(value, self.source)
            require(self.readiness is not None and value.get('publication') == self.readiness['publication']
                    and value.get('publication_stop_reason') == 'STOP_PUBLISH'
                    and value.get('publication_stopped') is True, 'Sender deliberate post-receive STOP_PUBLISH missing')
        else:
            require(value.get('receivedhash') == self.source['sha256'] and value.get('receivedsize') == self.source['size']
                    and value.get('managed') is True and value.get('activebookid') == self.source['archive_id']
                    and value.get('readable_ui') is True and (self.output / 'reader.png').is_file(),
                    'Receiver lacks independent managed fullhash/active reader/screenshot proof')
            radio = value.get('route_radio', {})
            require(radio.get('sta_connected') is True and radio.get('no_ap_verified') is True
                    and radio.get('current_ap_verified') is False, 'Receiver was not joined station/no-AP')
            require(value.get('joined_network_sha256') == self.sender_network_sha256
                    and type(value.get('joined_network_handle')) is int and value['joined_network_handle'] > 0,
                    'Receiver actual native joined network is not this sender hotspot')
            validate_received_job(value, self.source, publication)
        return value

    def request_stop(self, successful=False):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM if successful and self.role == 'sender' else signal.SIGINT)

    def reap(self):
        forced = False
        try:
            if self.process.poll() is None:
                try:
                    self.process.wait(timeout=self.window.remaining(cleanup=True))
                except (subprocess.TimeoutExpired, RuntimeError):
                    if self.process.poll() is None:
                        forced = True
                        try:
                            os.killpg(self.process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    self.process.wait(timeout=1)
            require(not forced, self.role + ' forcibly reaped without device cleanup confirmation')
            return self.cleanup_proof(self.receipt('result.json'))
        finally:
            self.log.close()


def await_receipt(peer, name, peers, window):
    while True:
        # Preserve child causal failures before a generic readiness timeout.
        for other in peers:
            other.fail_if_exited()
        value = peer.receipt(name)
        if value is not None:
            return value
        window.remaining()
        time.sleep(.1)


def run(args):
    require(args.start_device_window, 'Explicit --start-device-window required')
    prepared = load_frozen(args.prepared, args.prepared_sha256, args.sender_device, args.receiver_device)
    return timed_operation(args, run_impl, 'device_automation', prepared)


def run_impl(args, prepared):
    config = prepared['config']
    window = Window(args.output.resolve(), config['window_seconds'])
    nonce = uuid.uuid4().hex
    result = {'schema': 1, 'topology': 'sender-hotspot', 'sender_device': args.sender_device,
              'receiver_device': args.receiver_device, 'nonce': nonce, 'deadline': window.deadline,
              'prepared_sha256': args.prepared_sha256, 'status': 'failed', 'transfer_complete': False,
              'cleanup_complete': False, 'device_released': False,
              'reader_content_review': 'required: ' + config['expected_text'],
              'source_session_correlation': 'unavailable: only consumed receiver session and persisted source publisher independently observable'}
    peers = []
    previous = {}
    spawning = False
    spawn_interrupted = False

    def interrupted(signum, frame):
        nonlocal spawn_interrupted
        if spawning:
            spawn_interrupted = True
            return
        raise RuntimeError('Paired window interrupted; cancel exact owned work')

    def start_peer(role, publisher):
        nonlocal spawning
        # Defer the host exception until ownership is registered, without
        # blocking signals in the child that must actually accept cancellation.
        spawning = True
        try:
            peer = AndroidPeer(window, prepared, role, nonce, publisher)
            peers.append(peer)
        finally:
            spawning = False
        if spawn_interrupted:
            raise RuntimeError('Paired window interrupted during owned child startup')
        return peer

    for signum in (signal.SIGINT, signal.SIGTERM):
        previous[signum] = signal.signal(signum, interrupted)
    try:
        sender = start_peer('sender', config['sender_device'])
        sender.admit_instrumentation(await_receipt(sender, 'instrumentation-ready.json', peers, window))
        receiver = start_peer('receiver', sender.instrumentation['publisher_id'])
        receiver.admit_instrumentation(await_receipt(receiver, 'instrumentation-ready.json', peers, window))
        # BOTH instrumentations now exist before either route is admitted.
        sender.release_route()
        publication = sender.admit_publisher(await_receipt(sender, 'ready.json', peers, window))
        result['publication'] = publication
        receiver.release_route(sender.readiness['hotspot_network_sha256'])
        while True:
            sender.fail_if_exited()
            received = receiver.completed(publication)
            if received is not None:
                result['receiver_result'] = received
                result['received_job'] = validate_received_job(received, prepared['source'], publication)
                break
            window.remaining()
            time.sleep(.1)
        sender.fail_if_exited()
        sender.request_stop(successful=True)
        while sender.process.poll() is None:
            window.remaining()
            time.sleep(.1)
        result['sender_result'] = sender.completed()
        result['transfer_complete'] = True
    except BaseException as error:
        result['error'] = type(error).__name__ + ': ' + str(error)
    finally:
        # Ignore repeated interrupts only while completing this bounded owned teardown.
        for signum in previous:
            signal.signal(signum, signal.SIG_IGN)
        errors = []
        # Fan cancellation out before waiting: never leave the second child publishing
        # while the first consumes its cleanup allowance.
        for peer in peers:
            try:
                peer.request_stop()
            except BaseException as error:
                errors.append(peer.role + ' cancel: ' + str(error))
        for peer in peers:
            try:
                with TIMING.span('cleanup', 'peer-reap'):
                    receipt = peer.reap()
                result[peer.role + '_cleanup'] = {'process_reaped': True, 'receipt': receipt}
            except BaseException as error:
                errors.append(peer.role + ': ' + str(error))
        duration = time.monotonic() - window.started
        if duration > config['window_seconds']:
            errors.append('Owned cleanup exceeded frozen window bound')
        result.update(cleanup_errors=errors, duration_seconds=duration,
                      cleanup_complete=not errors, device_released=not errors)
        result['status'] = 'complete' if result['transfer_complete'] and not errors else 'failed'
        result['timing'] = TIMING.summary()
        result['timing_summary_path'] = str(window.output / 'timing-summary.json')
        save(window.output / 'result.json', result)
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    print(json.dumps({key: result[key] for key in ('status', 'transfer_complete', 'cleanup_complete', 'device_released')}))
    return 0 if result['status'] == 'complete' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    preparing = commands.add_parser('prepare')
    preparing.add_argument('--config', type=Path, required=True)
    preparing.add_argument('--java-home', type=Path, required=True)
    preparing.add_argument('--output', type=Path, required=True)
    running = commands.add_parser('run')
    running.add_argument('--prepared', type=Path, required=True)
    running.add_argument('--prepared-sha256', required=True)
    running.add_argument('--sender-device', required=True)
    running.add_argument('--receiver-device', required=True)
    running.add_argument('--start-device-window', action='store_true')
    running.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        return prepare(args) if args.command == 'prepare' else run(args)
    except Exception as error:
        print('android-pair: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
