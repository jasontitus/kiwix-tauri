#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build a same-certificate, framework-only test APK; drive real release Nearby UI.

Prepare (no device):
  python3 scripts/android-nearby.py --prepare-only --target-apk REVIEWED_RELEASE.apk
Run (starts/restarts instrumentation BEFORE establishing the transfer route):
  python3 scripts/android-nearby.py --device SERIAL --mode receive \
    --archive-id UUID --file-name WIRE --title TITLE --size BYTES --sha256 HEX \
    --peer PEER --sender-device UDID --nonce 32HEX --deadline UNIX --output DIR

No production APK/Gradle edits, root, run-as, DevTools, app backend, or DOM writes.
A receive success requires imported Open, active UUID and independently hashed
managed bytes. reader.png is actual screen evidence requiring visual review;
iframe existence is not an expected-article-content assertion. publish owns only
sharing it starts, advertises ready only after current OS topology proof, and stops
that share on a post-ready SIGTERM or finite deadline. --topology defaults to
routerless (STA off, product AP); lan requires an unchanged connected IPv4 station.
sender-hotspot requires disconnected stations initially, a publisher-owned AP,
and the receiver joining that AP (never a reverse AP). Its --route-gate JSON
admits route input only after the pair controller has started both runners.
Fire API28 publisher uses current WifiStateMachinePrime manager evidence, not
unsupported cmd wifi status or historical metrics. Actual LocationManager-off
fails preflight; secure location_mode is not authority (Fire may return null).
This helper never toggles global settings. verify permits a private-file pass.
Receive UI requires Android 11+ to hide and verify the native IME before Download;
this controls system UI only and never changes DOM values or application state.
Crash capture (no activity launch or input):
  python3 scripts/android-nearby.py --prepared RECEIPT --mode capture-crash \
    --device SERIAL --nonce 32HEX --deadline UNIX --output DIR
After reviewing DIR/crash-report.json, prepare a fresh tool receipt with
--reviewed-crash-sha256 FULL_FILE_SHA256. Sharing consumes that frozen identity,
opens View report and closes only the matching ordinary startup prompt. The
local report is neither acknowledged nor deleted and prompts on the next launch.
Preparation freezes testing_timing.py before execution. Runtime timing separates
ADB command wall (transport plus device execution), long instrumentation, observed
UI waits and cleanup; exact inference time is unavailable, never residual-derived.
"""
import argparse
import base64
from contextlib import nullcontext
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import types
import uuid
import zipfile

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'scripts' / 'android-nearby'
TIMING_SOURCE = ROOT / 'scripts' / 'testing_timing.py'
TIMING = None
DEFAULT_PREPARED = ROOT / '.native' / 'android-nearby-tool'
TARGET = 'org.kiwix.workbench'
RUNNER = 'org.kiwix.nearby.test/org.kiwix.nearby.NearbyInstrumentation'
PREFIX = 'KIWIX_ANDROID_NEARBY '
STOP = threading.Event()
STOP_SIGNAL = None


def stop_requested(signum, frame):
    global STOP_SIGNAL
    if STOP_SIGNAL is None or signum == signal.SIGINT:
        STOP_SIGNAL = signum
    # Repeated paired process-group signals must not interrupt the cleanup path.
    STOP.set()


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as source:
        for data in iter(lambda: source.read(131072), b''):
            h.update(data)
    return h.hexdigest()


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    temporary.replace(path)


def load_timing_dependency(expected_sha256=None, *, preparing=False, path=None):
    """Validate before execution and compile the verified bytes, never re-open by name."""
    if not preparing and (not isinstance(expected_sha256, str) or not re.fullmatch(r'[0-9a-f]{64}', expected_sha256)):
        raise ValueError('Prepare receipt lacks frozen timing_tool_sha256; prepare again')
    path = TIMING_SOURCE if path is None else Path(path)
    data = path.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if expected_sha256 is not None and digest != expected_sha256:
        raise ValueError('Timing dependency differs from frozen source; refusing import/device access')
    module = types.ModuleType('kiwix_android_testing_timing')
    module.__file__ = str(path)
    exec(compile(data, str(path), 'exec'), module.__dict__)
    return module.TimingLog, digest


def timed(category, label):
    return TIMING.span(category, label) if TIMING is not None else nullcontext()


def timed_subprocess(command, *, timing_category='build', timing_label='test_apk_build', **kwargs):
    with timed(timing_category, timing_label):
        return subprocess.run(command, **kwargs)


def finish_timing_span(span, failed=False):
    if span is not None:
        error = RuntimeError('Observed operation failed') if failed else None
        span.__exit__(type(error) if error is not None else None, error, None)


def begin_cleanup_timing(ui_span):
    # Leaving the decoder aborts its UI wait; subsequent cancellation/reap is
    # cleanup even if the native worker later emits readiness while draining.
    finish_timing_span(ui_span, failed=True)
    cleanup = timed('cleanup', 'owned_instrumentation_cleanup')
    cleanup.__enter__()
    return cleanup


def run(command, *, timeout=60, binary=False, interruptible=False,
        timing_category='host_prepare', timing_label='host_command'):
    with timed(timing_category, timing_label):
        return _run(command, timeout=timeout, binary=binary, interruptible=interruptible)


def _run(command, *, timeout=60, binary=False, interruptible=False):
    process = subprocess.Popen([str(v) for v in command], stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    end = time.monotonic() + timeout
    try:
        while True:
            if interruptible and STOP.is_set():
                raise RuntimeError('Host cancellation requested during bounded command')
            remaining = end - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(command, timeout)
            try:
                stdout, stderr = process.communicate(timeout=min(.2, remaining))
                break
            except subprocess.TimeoutExpired:
                continue
        if process.returncode:
            raise RuntimeError(f'{command[0]} failed ({process.returncode}): ' +
                               stderr.decode(errors='replace')[-6000:])
        return stdout if binary else stdout.decode(errors='strict')
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.communicate(timeout=1)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate(timeout=1)


def sdk_path(value):
    path = Path(value or os.environ.get('ANDROID_SDK_ROOT') or
                os.environ.get('ANDROID_HOME') or Path.home() / 'Library/Android/sdk').expanduser().resolve()
    if not (path / 'platforms/android-36/android.jar').is_file():
        raise ValueError('SDK 36 android.jar required; provide --android-sdk')
    return path


def toolset(args):
    sdk = sdk_path(args.android_sdk)
    if args.build_tools:
        tools = sdk / 'build-tools' / args.build_tools
    else:
        candidates = [p for p in (sdk / 'build-tools').iterdir()
                      if re.fullmatch(r'36\.\d+\.\d+', p.name)]
        if not candidates:
            raise ValueError('Android SDK 36 build tools required; provide --build-tools')
        tools = max(candidates, key=lambda p: tuple(map(int, p.name.split('.'))))
    paths = {name: tools / name for name in ('aapt2', 'd8', 'zipalign', 'apksigner')}
    paths['adb'] = Path(args.adb).expanduser().resolve() if args.adb else sdk / 'platform-tools/adb'
    paths['android_jar'] = sdk / 'platforms/android-36/android.jar'
    for name, path in paths.items():
        if not path.is_file():
            raise ValueError(f'Missing {name}: {path}')
    return paths


def certificate(apksigner, apk, timeout=60):
    text = run([apksigner, 'verify', '--verbose', '--print-certs', apk], timeout=timeout)
    values = re.findall(r'^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]{64})$', text, re.M)
    if len(values) != 1:
        raise ValueError('Exactly one APK signer required')
    return values[0].lower()


def sources():
    return {name: sha(SOURCE / name) for name in
            ('AndroidManifest.xml', 'NearbyInstrumentation.java', 'inspect.js')}


def reviewed_crash_identity(args, receipt=None):
    supplied = args.reviewed_crash_sha256
    if supplied is not None and not re.fullmatch(r'[0-9a-f]{64}', supplied):
        raise ValueError('--reviewed-crash-sha256 requires one exact lowercase full-file SHA256')
    if receipt is None:
        return supplied or ''
    frozen = receipt.get('reviewed_crash_sha256')
    if not isinstance(frozen, str) or (frozen and not re.fullmatch(r'[0-9a-f]{64}', frozen)):
        raise ValueError('Prepare receipt lacks a valid frozen reviewed crash identity; prepare again')
    if supplied is not None and supplied != frozen:
        raise ValueError('Reviewed crash identity differs from prepare receipt; prepare again')
    return frozen


def retain_crash_report(out, evidence, installed_identity):
    """Retain only the fixed native producer's bounded redacted JSON, not a path supplied by the runner."""
    if evidence.get('present') is False and set(evidence) == {'present'}:
        save(out / 'crash-report-receipt.json', dict(evidence, installed_identity=installed_identity))
        return
    if (evidence.get('present') is not True or evidence.get('native_redacted') is not True or
            type(evidence.get('size')) is not int or not 0 < evidence['size'] <= 32768 or
            not isinstance(evidence.get('sha256'), str) or
            not re.fullmatch(r'[0-9a-f]{64}', evidence['sha256']) or
            not isinstance(evidence.get('base64'), str) or len(evidence['base64']) > 43692):
        raise ValueError('Invalid bounded native crash evidence')
    data = base64.b64decode(evidence['base64'], validate=True)
    if len(data) != evidence['size'] or hashlib.sha256(data).hexdigest() != evidence['sha256']:
        raise ValueError('Native crash full-file identity mismatch')
    record = json.loads(data.decode('utf-8'))
    if not isinstance(record, dict) or type(record.get('schema')) is not int or record['schema'] != 1:
        raise ValueError('Unsupported native crash record')
    (out / 'crash-report.json').write_bytes(data)
    save(out / 'crash-report-receipt.json', dict(
        {key: value for key, value in evidence.items() if key != 'base64'},
        installed_identity=installed_identity))


def load_prepare_receipt(path, expected_sha256=None):
    if expected_sha256 is not None and not re.fullmatch(r'[0-9a-f]{64}', expected_sha256):
        raise ValueError('--prepared-sha256 requires an exact lowercase SHA256')
    with Path(path).open('rb') as source:
        data = source.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError('Prepare receipt exceeds size bound')
    if expected_sha256 is not None and hashlib.sha256(data).hexdigest() != expected_sha256:
        raise ValueError('Prepare receipt differs from frozen SHA256; refusing device access')
    receipt = json.loads(data)
    if not isinstance(receipt, dict):
        raise ValueError('Prepare receipt must be an object')
    return receipt


def prepare(args):
    global TIMING
    output = Path(args.output).expanduser().resolve() if args.output else DEFAULT_PREPARED
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'prepare.json').exists():
        raise ValueError('Prepare output already contains receipt; use a fresh --output directory')
    TimingLog, timing_sha = load_timing_dependency(preparing=True)
    TIMING = TimingLog(output, os.environ.get('KIWIX_TIMING_ITERATION') or uuid.uuid4().hex, 'android_prepare')
    try:
        with timed('host_prepare', 'prepare_test_apk'):
            return _prepare(args, output, timing_sha)
    finally:
        TIMING.close()
        TIMING = None


def _prepare(args, output, timing_sha):
    tools = toolset(args)
    reviewed_crash = reviewed_crash_identity(args)
    if not args.target_apk:
        raise ValueError('--target-apk reviewed optimized release APK is required for preparation')
    target_apk = Path(args.target_apk).expanduser().resolve()
    if not target_apk.is_file():
        raise ValueError('Explicit reviewed target APK does not exist')
    target_sha = sha(target_apk)
    target_certificate = certificate(tools['apksigner'], target_apk)
    target_badging = run([tools['aapt2'], 'dump', 'badging', target_apk])
    package = re.search(r"^package: name='([^']+)'", target_badging, re.M)
    if not package or package.group(1) != TARGET or 'application-debuggable' in target_badging:
        raise ValueError('Reviewed target must be the exact non-debuggable org.kiwix.workbench APK')
    java_home = args.java_home or os.environ.get('JAVA_HOME')
    if not java_home and sys.platform == 'darwin':
        java_home = run(['/usr/libexec/java_home', '-v', '17']).strip()
    javac = Path(java_home) / 'bin/javac' if java_home else shutil.which('javac')
    if not javac:
        raise ValueError('Java 17 javac required; provide --java-home')
    version = timed_subprocess([str(javac), '-version'], capture_output=True, text=True, check=True,
                               timing_category='host_prepare', timing_label='java_version')
    if not re.search(r'javac 17\.', version.stdout + version.stderr):
        raise ValueError('Use Java 17, not the system default JDK')
    keystore = Path(args.keystore).expanduser().resolve()
    if not keystore.is_file():
        raise ValueError('Existing debug.keystore required; this tool never creates/replaces signing identity')
    frozen_sources = sources()
    classes = output / 'classes'
    dex = output / 'dex'
    classes.mkdir()
    dex.mkdir()
    env = os.environ.copy()
    if java_home:
        env['JAVA_HOME'] = str(java_home)
        env['PATH'] = str(Path(java_home) / 'bin') + os.pathsep + env.get('PATH', '')
    # No Gradle, Kotlin, AndroidX, application classes, JNI, or production build.
    timed_subprocess([str(javac), '--release', '17', '-classpath', str(tools['android_jar']),
                    '-d', str(classes), str(SOURCE / 'NearbyInstrumentation.java')], env=env, check=True)
    class_files = sorted(classes.rglob('*.class'))
    timed_subprocess([str(tools['d8']), '--release', '--min-api', '26', '--lib', str(tools['android_jar']),
                    '--output', str(dex), *map(str, class_files)], env=env, check=True)
    resources = output / 'resources.apk'
    timed_subprocess([str(tools['aapt2']), 'link', '-I', str(tools['android_jar']),
                    '--manifest', str(SOURCE / 'AndroidManifest.xml'), '--min-sdk-version', '26',
                    '--target-sdk-version', '36', '-A', str(SOURCE), '-o', str(resources)], env=env, check=True)
    # aapt2 assets are the reviewed sources. Only classes.dex is added before signing.
    with zipfile.ZipFile(resources, 'a') as apk:
        apk.write(dex / 'classes.dex', 'classes.dex')
    aligned = output / 'aligned.apk'
    timed_subprocess([str(tools['zipalign']), '-f', '4', str(resources), str(aligned)], env=env, check=True)
    apk = output / 'nearby-test.apk'
    # apksigner reads secrets from the child environment; argv is world-readable.
    sign_env = dict(env, KIWIX_KS_PASS=args.keystore_password, KIWIX_KEY_PASS=args.key_password)
    timed_subprocess([str(tools['apksigner']), 'sign', '--ks', str(keystore), '--ks-key-alias', args.key_alias,
                    '--ks-pass', 'env:KIWIX_KS_PASS', '--key-pass', 'env:KIWIX_KEY_PASS',
                    '--out', str(apk), str(aligned)], env=sign_env, check=True)
    test_certificate = certificate(tools['apksigner'], apk)
    if test_certificate != target_certificate:
        raise ValueError('Test signer does not match reviewed release; production signature is never changed')
    if sha(target_apk) != target_sha:
        raise ValueError('Reviewed target APK changed during preparation')
    receipt = {'schema': 1, 'target_package': TARGET, 'instrumentation': RUNNER,
               'tool_apk': str(apk), 'tool_apk_sha256': sha(apk),
               'target_apk': str(target_apk), 'target_apk_sha256': target_sha,
               'certificate_sha256': test_certificate,
               'reviewed_crash_sha256': reviewed_crash,
               'source_sha256': frozen_sources, 'host_source_sha256': sha(Path(__file__)),
               'timing_tool_sha256': timing_sha, 'timing_ledger': str(TIMING.path),
               'timing_summary': str(output / 'timing-summary.json'),
               'android_jar_sha256': sha(tools['android_jar']), 'java_version': (version.stdout + version.stderr).strip(),
               'tools': {name: str(value) for name, value in tools.items()},
               'created_unix': time.time()}
    save(output / 'prepare.json', receipt)
    print(json.dumps(receipt, sort_keys=True))
    return 0


def validate(args):
    required = ('device', 'mode', 'deadline', 'output', 'nonce')
    if args.mode != 'capture-crash':
        required += ('archive_id', 'file_name', 'title', 'size', 'sha256', 'peer', 'sender_device')
    for key in required:
        if getattr(args, key) is None:
            raise ValueError('--' + key.replace('_', '-') + ' is required')
    if not re.fullmatch(r'[A-Za-z0-9._:-]{1,160}', args.device):
        raise ValueError('Explicit device identity contains unsupported characters')
    if not re.fullmatch(r'[0-9a-f]{32}', args.nonce):
        raise ValueError('Nonce must be lowercase 32 hex')
    if not 15 <= args.deadline - time.time() <= 3600:
        raise ValueError('Explicit deadline must be 15..3600 seconds away')
    if args.mode == 'capture-crash':
        return
    if args.topology in ('lan', 'sender-hotspot') and args.mode == 'receive' and str(uuid.UUID(args.sender_device)) != args.sender_device:
        raise ValueError('Receive requires actual canonical publisher peer UUID')
    if args.topology == 'sender-hotspot' and args.mode in ('publish', 'receive') and not args.route_gate:
        raise ValueError('Sender-hotspot requires an explicit controller-owned --route-gate')
    if str(uuid.UUID(args.archive_id)) != args.archive_id:
        raise ValueError('Canonical lowercase archive UUID required')
    if not re.fullmatch(r'[0-9a-f]{64}', args.sha256) or not re.fullmatch(r'[0-9a-f]{32}', args.nonce):
        raise ValueError('SHA256/nonce must be lowercase 64/32 hex')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._ -]{0,239}\.zim', args.file_name):
        raise ValueError('Wire filename must be a single ASCII .zim leaf for native keyboard input')
    for name in ('device', 'sender_device'):
        if not re.fullmatch(r'[A-Za-z0-9._:-]{1,160}', getattr(args, name)):
            raise ValueError('Explicit device identities contain unsupported characters')
    if not 0 < args.size < 2**63:
        raise ValueError('Positive source byte size required')
    if not 15 <= args.deadline - time.time() <= 3600:
        raise ValueError('Explicit deadline must be 15..3600 seconds away')
    for name in ('title', 'peer'):
        value = getattr(args, name)
        if not value.strip() or len(value) > 500 or any(ord(c) < 32 for c in value):
            raise ValueError('Bounded nonempty title/peer required')


def parse_station(text):
    """Keep network identity hashed; never retain SSID/BSSID or credentials."""
    connected = bool(re.search(r'(?im)^Wi-?fi is connected\b|Supplicant state:\s*COMPLETED', text))
    disabled = bool(re.search(r'(?im)^Wi-?fi is disabled\s*$', text))
    bssids = {v.lower() for v in re.findall(r'\bBSSID:\s*([0-9a-fA-F:]{17})(?=\s|,|$)', text)}
    addresses = set()
    for value in re.findall(r'\bIP(?: address)?:\s*/?(\d+\.\d+\.\d+\.\d+)\b', text, re.I):
        try:
            address = ipaddress.IPv4Address(value)
            if not (address.is_unspecified or address.is_loopback or address.is_multicast or address.is_link_local):
                addresses.add(str(address))
        except ValueError:
            pass
    valid_bssid = len(bssids) == 1 and next(iter(bssids)) not in ('02:00:00:00:00:00', '00:00:00:00:00:00', 'ff:ff:ff:ff:ff:ff')
    identity = hashlib.sha256(next(iter(bssids)).encode()).hexdigest() if valid_bssid else None
    disconnected = disabled or bool(re.search(r'(?im)^Wi-?fi is (?:not connected|disconnected)\b|Supplicant state:\s*(?:DISCONNECTED|INACTIVE|INTERFACE_DISABLED)\b', text))
    return dict(source='cmd wifi status', sta_connected=connected, sta_disabled=disabled and not connected,
                sta_disconnected=disconnected and not connected,
                station_identity_sha256=identity, station_ipv4=next(iter(addresses)) if len(addresses) == 1 else None,
                status_sha256=hashlib.sha256(text.encode()).hexdigest())


def parse_legacy_radio(dump, stage):
    """API28 current manager dump only; logs/metrics never establish live state."""
    section = re.search(r'(?m)^Dump of WifiStateMachinePrime\n(.*?)(?=^WifiStateMachine:|\Z)', dump, re.S)
    if not section:
        raise ValueError('Unknown API28 current WifiStateMachinePrime schema')
    current = section.group(1)
    counts = re.findall(r'(?m)^NumActiveModeManagers: (\d+)\s*$', current)
    blocks = re.findall(r'(?m)^--Dump of (\w+)--\n(.*?)(?=^--Dump of |\Z)', current, re.S)
    if len(counts) != 1 or int(counts[0]) != len(blocks) or any(
            name not in ('ClientModeManager', 'SoftApManager') for name, _ in blocks):
        raise ValueError('Unknown API28 current manager list')
    ap = []
    for name, body in blocks:
        states = re.findall(r'(?m)^current StateMachine mode: (\w+)\s*$', body)
        interfaces = re.findall(r'(?m)^m(?:Client|Ap)InterfaceName: ([A-Za-z0-9_.-]+)\s*$', body)
        up = re.findall(r'(?m)^mIfaceIsUp: (true|false)\s*$', body)
        if states != ['StartedState'] or len(interfaces) != 1 or up != ['true']:
            raise ValueError('Unknown/transitional API28 current manager state')
        if name == 'SoftApManager':
            ap.append(interfaces[0])
    info = re.findall(r'(?m)^mNetworkInfo \[type: WIFI\[\], state: (\w+)/[^\n]*$', dump)
    wifi = re.findall(r'(?m)^Wi-Fi is (enabled|disabled)\s*$', dump)
    if len(info) != 1 or info[0] not in ('DISCONNECTED', 'CONNECTED') or len(wifi) != 1:
        raise ValueError('Unknown API28 current station schema')
    station = parse_station('\n'.join(re.findall(r'(?m)^mWifiInfo [^\n]*$', dump)))
    connected = info[0] == 'CONNECTED'
    if connected != station['sta_connected']:
        raise ValueError('Conflicting API28 current station evidence')
    return dict(station, source='API28 dumpsys wifi current WifiStateMachinePrime',
                stage=stage, sta_connected=connected, sta_disconnected=not connected,
                sta_disabled=wifi == ['disabled'] and not connected,
                current_ap_verified=len(ap) == 1, no_ap_verified=not ap,
                ap_frequency_mhz=[], legacy_current_ap_manager=len(ap) == 1,
                dump_sha256=hashlib.sha256(dump.encode()).hexdigest())


def validate_legacy_location(value):
    expected = dict(schema=1, android_api=28, source='LocationManager.isLocationEnabled',
                    location_services_enabled=True)
    if (not isinstance(value, dict) or value != expected or type(value.get('schema')) is not int
            or type(value.get('android_api')) is not int or value.get('location_services_enabled') is not True):
        raise ValueError('API28 Nearby requires actual current LocationManager enabled evidence; no settings toggled')

def validate_publication_retirement(value):
    if not isinstance(value, dict) or type(value.get('schema')) is not int or value.get('schema') != 1:
        raise ValueError('Missing exact publication retirement schema')
    if any(value.get(key) is not True for key in ('publication_nonstarting', 'publication_nonsharing', 'hotspot_retired')):
        raise ValueError('Publication has not reached explicit terminal retirement')
    native = value.get('native_hotspot')
    if (not isinstance(native, dict) or type(native.get('schema')) is not int or native.get('schema') != 1
            or native.get('source') != 'fixed retained KiwixMobilePlugin/LocalHotspot fields on main thread'
            or native.get('plugin_identity_bound') is not True or native.get('retired') is not True
            or type(native.get('generation')) is not int or native['generation'] < 0
            or any(type(native.get(key)) is not bool for key in ('owner_present', 'closing_callback_marker'))
            or any(native.get(key) is not False for key in ('plugin_invoke_pending', 'system_ui_pending',
                'start_pending', 'reservation_active', 'quic_route_retained', 'cancelled_start_pending'))):
        raise ValueError('Publication native hotspot pending/reservation/route retirement is unproven')


def normalize_native_result(native, bound, installed_identity, api_level, host_failure=None):
    """Normal and cancellation-drain finals share exactly the same authority gate."""
    if (not isinstance(native, dict) or type(native.get('schema')) is not int or native.get('schema') != 1
            or any(type(native.get(key)) is not type(value) or native.get(key) != value for key, value in bound.items())
            or installed_identity is None or native.get('installed_identity') != installed_identity
            or type(native.get('android_api')) is not int or native.get('android_api') != api_level
            or native.get('status') not in ('complete', 'failed')
            or any(type(native.get(key)) is not bool for key in
                   ('cleanup_complete', 'sender_route_started', 'receiver_route_started'))):
        raise ValueError('Native final result lacks exact binding/API/installed identity/ownership schema')
    if 'primary_error' in native and (not isinstance(native['primary_error'], str)
            or not native['primary_error'] or native['primary_error'] != native.get('error')):
        raise ValueError('Native primary failure identity changed')
    mode = bound['mode']
    if ((mode != 'publish' and native['sender_route_started']) or
            (mode != 'receive' and native['receiver_route_started'])):
        raise ValueError('Native final claims another role ownership')
    if native['cleanup_complete']:
        if native.get('crash_report_preserved') is not True:
            raise ValueError('Native final lacks crash preservation')
        if native['sender_route_started']:
            stopped = native.get('publication_stop_ui', {})
            if (native.get('owned_sharing_stopped') is not True or native.get('publication_stopped') is not True
                    or not isinstance(stopped, dict) or stopped.get('ready') is not True
                    or any(stopped.get(key) is not True for key in ('publication_nonstarting', 'publication_nonsharing'))
                    or stopped.get('sharing_active') is not False or stopped.get('hotspot_active') is not False
                    or not isinstance(stopped.get('publication_errors'), list)):
                raise ValueError('Native publication cleanup lacks explicit stopped UI receipt')
            if bound['topology'] == 'sender-hotspot':
                validate_publication_retirement(native.get('publication_retirement'))
        if native['receiver_route_started']:
            retirement = native.get('receiver_retirement', {})
            if (native.get('receiver_join_retired') is not True or native.get('receiver_consumer_retired') is not True
                    or not isinstance(retirement, dict) or retirement.get('ordinary_stop_all_completed') is not True
                    or retirement.get('source') != 'awaited ordinary Stop all plus captured Network disappearance'):
                raise ValueError('Native receiver cleanup lacks admitted join/consumer retirement receipt')
        if (native['sender_route_started'] or native['receiver_route_started']) and any(
                native.get(key) is not True for key in ('preserved_before_jobs', 'preserved_before_books_settings_tabs')):
            raise ValueError('Native owned-route cleanup lacks preservation receipts')
    normalized = dict(native)
    normalized['device_cleanup_result'] = native
    if host_failure is not None:
        normalized['host_failure'] = dict(host_failure)
        normalized['status'] = 'failed'
        normalized['error'] = native.get('primary_error') or host_failure.get('error') or native.get('error') or 'Host execution failed'
    return normalized



def sender_hotspot_initial(sample):
    return (sample.get('sta_disconnected') is True and sample.get('sta_connected') is False
            and sample.get('no_ap_verified') is True and sample.get('current_ap_verified') is False)


def sender_hotspot_route(sample, mode):
    if mode == 'publish':
        return (sample.get('sta_connected') is False and sample.get('current_ap_verified') is True
                and sample.get('no_ap_verified') is False)
    return lan_radio_admitted(sample)


def load_route_gate(path, args):
    expected = dict(schema=1, nonce=args.nonce, topology=args.topology, device=args.device,
                    mode=args.mode, deadline=args.deadline)
    with Path(path).open('rb') as source:
        data = source.read(4097)
    gate = json.loads(data)
    if args.mode == 'receive':
        network = gate.get('sender_network_sha256') if isinstance(gate, dict) else None
        if not isinstance(network, str) or not re.fullmatch(r'[0-9a-f]{64}', network):
            raise ValueError('Receiver route gate lacks actual sender AP identity')
        expected['sender_network_sha256'] = network
    if (len(data) > 4096 or gate != expected or type(gate.get('schema')) is not int
            or type(gate.get('deadline')) is not int):
        raise ValueError('Unbound or unknown controller route gate')
    return expected


def graceful_publish_stop(*, graceful, mode, ready, signum, host_error, cancel_sent):
    return (graceful and mode == 'publish' and ready and signum == signal.SIGTERM
            and host_error is None and not cancel_sent)


def lan_radio_admitted(sample, baseline=None):
    """LAN cannot stand in for the separate routerless STA-off/AP predicates."""
    admitted = (sample.get('sta_connected') is True and sample.get('sta_disabled') is False and
                sample.get('no_ap_verified') is True and sample.get('current_ap_verified') is False and
                bool(sample.get('station_identity_sha256')) and bool(sample.get('station_ipv4')))
    return admitted and (baseline is None or all(sample.get(key) == baseline.get(key)
                        for key in ('station_identity_sha256', 'station_ipv4')))


def valid_publication(value):
    if not isinstance(value, dict):
        return False
    try:
        return (str(uuid.UUID(value['publisher_id'])) == value['publisher_id'] and
                type(value['library_generation']) is int and 0 < value['library_generation'] < 2**63)
    except (KeyError, ValueError, TypeError, AttributeError):
        return False


def validate_received_job(result, args):
    journal = result.get('received_journal', {})
    job = journal.get('matched_job', {})
    match = re.fullmatch(r'kiwix-library://public/([0-9a-f]{32})/([0-9a-f-]{36})/([0-9a-f-]{36})', job.get('url', ''))
    if (journal.get('status') != 'matched' or journal.get('match_count') != 1 or
            job.get('matches_independent_received_bytes') is not True or not match or
            any(str(uuid.UUID(value)) != value for value in match.groups()[1:]) or
            match.group(2) != args.sender_device or match.group(3) != args.archive_id or
            job.get('journal_key') != job.get('id') or not job.get('id') or
            job.get('fileName') != args.file_name or job.get('status') != 'complete' or
            job.get('downloaded') != args.size or job.get('total') != args.size or
            job.get('destination') != result.get('managed_path') or not result.get('managed_path') or
            job.get('transport') != 'quic-library' or job.get('error') is not None or job.get('importError') is not None or
            job.get('consumed_acquisition') is not True or not result.get('installation_id') or
            job.get('installation_id') != result.get('installation_id') or
            job.get('publication') != dict(size=args.size, sha256=args.sha256)):
        raise ValueError('Receive requires one actual persisted job bound to sender session/peer and full managed bytes')

def parse_radio(station, dump, stage):
    """Read current service fields; OEM dumps may separate a field with spaces."""
    separator = r'(?:[ \t]*[:=][ \t]*|[ \t]+)'
    maps = re.findall(r'(?im)^[ \t]*m?CurrentSoftApInfo(?:Map)?' + separator + r'([^\n]*)', dump)
    managers = re.findall(r'(?im)^[ \t]*mActiveModeManagers' + separator + r'([^\n]*)', dump)
    states = [int(v) for v in re.findall(
        r'(?im)^[ \t]*mSoftAp(?:Tethered|LocalOnly)?State' + separator + r'(\d+)[ \t]*$', dump)]
    frequencies = sorted({int(v) for row in maps for v in
                          re.findall(r'(?i)\bfrequency\s*[:=]\s*(\d+)', row) if int(v) > 0})
    fields = [value for row in maps for value in re.findall(
        r'(?i)\b(?:frequency|bandwidth|channelwidth|channel width|band)\s*[:=]\s*\d+(?:\s*(?:MHz|GHz))?', row)]
    empty_maps = bool(maps) and all(row.strip() in ('{}', '[]', 'null') for row in maps)
    has_ap_manager = any('SoftApManager' in row for row in managers)
    no_managers = bool(managers) and not has_ap_manager
    disabled_states = bool(states) and all(value == 11 for value in states)
    return dict(station, stage=stage, source='cmd wifi status + dumpsys -t 2 wifi',
                current_ap_verified=bool(frequencies),
                no_ap_verified=not frequencies and not has_ap_manager and not any(value in (12, 13) for value in states)
                    and (empty_maps or no_managers or disabled_states),
                ap_frequency_mhz=frequencies, ap_numeric_fields=fields,
                ap_states=states, current_info_maps_observed=len(maps),
                dump_sha256=hashlib.sha256(dump.encode()).hexdigest(),
                note='Current SoftApInfo fields only; numeric bandwidth enums are not decoded.')



def execute(args):
    global TIMING
    validate(args)
    prepared = Path(args.prepared).expanduser().resolve()
    receipt = load_prepare_receipt(prepared, args.prepared_sha256)
    if (receipt.get('schema') != 1 or receipt.get('target_package') != TARGET or
            receipt.get('instrumentation') != RUNNER or receipt.get('source_sha256') != sources() or
            receipt.get('host_source_sha256') != sha(Path(__file__))):
        raise ValueError('Prepare receipt does not match reviewed current sources/target; prepare again')
    reviewed_crash = reviewed_crash_identity(args, receipt)
    if not re.fullmatch(r'[0-9a-f]{64}', receipt.get('target_apk_sha256', '')):
        raise ValueError('Prepare receipt must freeze an explicit reviewed target APK SHA256')
    if args.target_apk and sha(Path(args.target_apk).expanduser()) != receipt['target_apk_sha256']:
        raise ValueError('Explicit target APK differs from prepare receipt')
    apk = Path(receipt['tool_apk'])
    if sha(apk) != receipt['tool_apk_sha256']:
        raise ValueError('Prepared test APK changed')
    TimingLog, _ = load_timing_dependency(receipt.get('timing_tool_sha256'))
    tools = toolset(args)
    if certificate(tools['apksigner'], apk) != receipt['certificate_sha256']:
        raise ValueError('Prepared APK certificate mismatch')
    out = Path(args.output).expanduser().resolve()
    out.mkdir(parents=True, exist_ok=False)
    TIMING = TimingLog(out, os.environ.get('KIWIX_TIMING_ITERATION') or uuid.uuid4().hex, 'android_' + args.mode)
    adb = [str(tools['adb']), '-s', args.device]
    process = None
    keys = ('mode', 'topology', 'nonce', 'deadline', 'device', 'sender_device', 'archive_id',
            'file_name', 'title', 'size', 'sha256', 'peer')
    bound = {key: getattr(args, key) for key in keys if getattr(args, key) is not None}
    bound['reviewed_crash_sha256'] = reviewed_crash
    result = dict(bound, schema=1, status='failed', cleanup_complete=True,
                  error='Runner did not produce a final result')
    screenshots = {}
    deadline = args.deadline
    radio = []
    timings = {}
    control_ready = False
    cancel_sent = False
    stop_publish_sent = False
    publisher_ready = False
    ready_publication = None
    final_radio = None
    station_admission = None
    initial_radio = None
    route_radio = None
    host_error = None
    stream_complete = False
    installed_identity = None
    crash_evidence = None
    location_preflight = None
    api_level = None
    publisher_id = None
    automation_span = None
    ui_span = None
    ui_wait_operation = None
    retirement_failure = None

    def call(command, binary=False):
        if STOP.is_set() and process is None:
            raise RuntimeError('Host cancellation requested before instrumentation')
        remaining = deadline - time.time()
        if remaining <= 0:
            raise TimeoutError('Explicit device deadline reached')
        return run(adb + command, timeout=min(60, remaining), binary=binary, interruptible=True,
                   timing_category='adb_command', timing_label='adb_request')

    def station_status(*, interruptible=True):
        text = run(adb + ['shell', 'cmd', 'wifi', 'status'],
                   timeout=max(.1, min(3, deadline - time.time())), interruptible=interruptible,
                   timing_category='adb_command', timing_label='wifi_station_status')
        return parse_station(text)

    def capture_radio(stage, *, interruptible=True):
        """Admit current state only, never historical logs or raw credentials."""
        station = None if api_level == 28 else station_status(interruptible=interruptible)
        dump = run(adb + ['shell', 'dumpsys', '-t', '2', 'wifi'],
                   timeout=max(.1, min(3, deadline - time.time())), interruptible=interruptible,
                   timing_category='adb_command', timing_label='wifi_radio_state')
        sample = parse_legacy_radio(dump, stage) if api_level == 28 else parse_radio(station, dump, stage)
        radio.append(sample)
        return sample

    def send_control(command, evidence=None):
        values = adb + ['shell', 'am', 'broadcast', '--receiver-foreground',
                        '-a', 'org.kiwix.nearby.CONTROL.' + args.nonce, '-p', TARGET,
                        '--es', 'nonce', args.nonce, '--es', 'command', command]
        if evidence is not None:
            encoded = base64.urlsafe_b64encode(json.dumps(evidence, sort_keys=True).encode()).decode()
            values.extend(['--es', 'evidence', encoded])
        return run(values, timeout=max(.1, min(3, deadline - time.time())),
                   timing_category='adb_command', timing_label='instrumentation_control')

    def cancel_owned(*, graceful=False):
        nonlocal cancel_sent, stop_publish_sent
        if control_ready and process is not None and process.poll() is None:
            if graceful_publish_stop(graceful=graceful, mode=args.mode, ready=publisher_ready,
                                     signum=STOP_SIGNAL, host_error=host_error, cancel_sent=cancel_sent):
                if not stop_publish_sent:
                    send_control('STOP_PUBLISH')
                    stop_publish_sent = True
            elif not cancel_sent:
                send_control('cancel')
                cancel_sent = True

    try:
        if call(['get-state']).strip() != 'device':
            raise ValueError('Explicit adb device is not available')
        api_level = int(call(['shell', 'getprop', 'ro.build.version.sdk']).strip())
        result['android_api'] = api_level
        if api_level < 29 and args.mode in ('receive', 'publish'):
            if api_level != 28 or args.mode != 'publish' or args.topology != 'sender-hotspot':
                raise ValueError('Legacy OS radio evidence is reviewed only for API28 sender-hotspot publisher')
        if args.mode in ('receive', 'publish'):
            station_admission = capture_radio('before-instrumentation')
            if args.topology == 'lan':
                if not lan_radio_admitted(station_admission):
                    raise ValueError('LAN requires an actual connected IPv4 station and no AP; network state is never changed by this tool')
            elif args.topology == 'sender-hotspot':
                if not sender_hotspot_initial(station_admission):
                    raise ValueError('Sender-hotspot requires disconnected station and current no-AP baseline')
            elif not station_admission['sta_disabled'] or station_admission['sta_connected']:
                raise ValueError('Routerless transfer requires STA disabled before instrumentation; network state is never changed by this tool')
        paths = call(['shell', 'pm', 'path', TARGET]).splitlines()
        if len(paths) != 1 or not paths[0].startswith('package:/data/app/') or not paths[0].endswith('/base.apk'):
            raise ValueError('Expected installed single optimized base APK; split APKs require a separately reviewed identity contract')
        installed_apk = out / 'installed-base.apk'
        call(['pull', paths[0][len('package:'):], str(installed_apk)])
        expected_apk_sha = sha(installed_apk)
        if expected_apk_sha != receipt['target_apk_sha256']:
            raise ValueError('Installed APK differs from reviewed prepared target; refusing instrumentation, with no production install')
        installed_cert = certificate(tools['apksigner'], installed_apk, timeout=min(60, deadline - time.time()))
        if installed_cert != receipt['certificate_sha256']:
            raise ValueError('Installed release certificate does not match existing test signer; production is untouched')
        save(out / 'request.json', dict(bound, target_package=TARGET, target_apk_sha256=expected_apk_sha,
                                       certificate_sha256=installed_cert, prepare_receipt=receipt,
                                       station_admission=station_admission))
        call(['install', '-r', '-t', str(apk)])  # only the separate test package
        if args.mode != 'capture-crash':
            call(['shell', 'input', 'keyevent', 'KEYCODE_WAKEUP'])
            call(['shell', 'wm', 'dismiss-keyguard'])
        command = adb + ['shell', 'am', 'instrument', '-w', '-r']
        wire_args = dict(bound, target_apk_sha256=expected_apk_sha, certificate_sha256=installed_cert)
        if args.mode in ('receive', 'publish'):
            wire_args['sta_disabled'] = str(station_admission['sta_disabled']).lower()
        for key, value in wire_args.items():
            # adb shell joins argv remotely: quote every instrumentation value.
            import shlex
            command.extend(['-e', key, shlex.quote(str(value))])
        command.append(RUNNER)
        if STOP.is_set():
            raise RuntimeError('Host cancellation requested before instrumentation')
        result['cleanup_complete'] = False
        automation_span = timed('device_automation', 'native_instrumentation')
        automation_span.__enter__()
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding='utf-8', errors='replace', bufsize=1,
                                   start_new_session=True)
        lines = queue.Queue()

        def consume():
            try:
                for line in process.stdout:
                    lines.put(line)
            finally:
                lines.put(None)

        threading.Thread(target=consume, daemon=True).start()
        with (out / 'instrumentation.log').open('w') as log, (out / 'events.jsonl').open('w') as event_log:
            while True:
                remaining = deadline - time.time()
                if remaining <= 0:
                    raise TimeoutError('Explicit instrumentation deadline reached')
                if STOP.is_set():
                    cancel_owned(graceful=True)
                try:
                    line = lines.get(timeout=min(.25, remaining))
                except queue.Empty:
                    continue
                if line is None:
                    stream_complete = True
                    break
                log.write(line)
                log.flush()
                if PREFIX not in line:
                    continue
                payload = line.split(PREFIX, 1)[1].strip()
                if not payload.startswith('{'):
                    continue
                row = json.loads(payload)
                if row.get('nonce') != args.nonce or row.get('topology') != args.topology:
                    raise ValueError('Unbound instrumentation event nonce/topology')
                event_log.write(json.dumps(row, sort_keys=True) + '\n')
                event_log.flush()
                data = row['data']
                if row['event'] == 'waiting' and ui_span is None:
                    ui_span = timed('ui_wait', 'host_observed_native_ui_wait')
                    ui_span.__enter__()
                    ui_wait_operation = data.get('operation')
                elif ui_span is not None and (
                        (row['event'] == 'ui-ready' and data.get('operation') == ui_wait_operation)
                        or row['event'] in ('failure', 'result')):
                    finish_timing_span(ui_span, row['event'] != 'ui-ready')
                    ui_span = None
                    ui_wait_operation = None
                if row['event'] == 'identity':
                    if (installed_identity is not None or data.get('apk_sha256') != expected_apk_sha or
                            data.get('certificate_sha256') != installed_cert or data.get('package') != TARGET or
                            data.get('debuggable') is not False or data.get('android_os', {}).get('api') != api_level):
                        raise ValueError('Crash producer installed-app identity mismatch')
                    installed_identity = data
                if row['event'] == 'location-preflight':
                    if (api_level != 28 or args.mode != 'publish' or installed_identity is None
                            or location_preflight is not None):
                        raise ValueError('Unbound or duplicate native location preflight')
                    location_preflight = data
                    validate_legacy_location(data)
                if row['event'] == 'crash-report':
                    if installed_identity is None or crash_evidence is not None:
                        raise ValueError('Unbound or duplicate native crash report')
                    retain_crash_report(out, data, installed_identity)
                    crash_evidence = data
                    if args.mode != 'capture-crash' and (
                            (data.get('present') is True and data.get('sha256') != reviewed_crash) or
                            (data.get('present') is False and reviewed_crash)):
                        raise ValueError('Owned native crash identity differs from frozen review; report retained, no dismissal admitted')
                if row['event'] in ('instrumentation-ready', 'publisher-ready', 'publishing', 'catalog-route', 'reverse-hotspot-verified',
                                    'imported_open', 'verified_bytes', 'reader', 'result'):
                    timings[row['event']] = row['elapsed_ms']
                elif row['event'] == 'input':
                    timings['input:' + data['operation']] = row['elapsed_ms']
                if row['event'] == 'screenshot':
                    name = data['name']
                    if name not in ('reader.png', 'publishing.png', 'failure.png', 'crash-before-close.png', 'crash-after-close.png'):
                        raise ValueError('Unadmitted screenshot filename')
                    item = screenshots.setdefault(name, {'chunks': data['chunks'], 'sha256': data['sha256'], 'parts': {}})
                    if (item['chunks'] != data['chunks'] or item['sha256'] != data['sha256'] or
                            not 0 <= data['index'] < data['chunks'] <= 1000 or data['index'] in item['parts']):
                        raise ValueError('Invalid/duplicate screenshot chunk')
                    item['parts'][data['index']] = data['base64']
                else:
                    print(PREFIX + json.dumps(row, sort_keys=True), flush=True)
                if row['event'] == 'control-ready':
                    control_ready = True
                    if STOP.is_set():
                        cancel_owned()
                if row['event'] == 'instrumentation-ready':
                    if any(str(data.get(key)) != str(value) for key, value in bound.items()) or data.get('status') != 'ready':
                        raise ValueError('Readiness source/device binding mismatch')
                    if api_level == 28 and args.mode == 'publish':
                        validate_legacy_location(data.get('location_preflight'))
                        if data['location_preflight'] != location_preflight:
                            raise ValueError('Readiness lacks exact native location preflight receipt')
                    if args.mode in ('receive', 'publish'):
                        if data.get('no_ap_at_instrumentation_ready') is not True:
                            raise ValueError('Readiness lacks fresh no-AP product admission')
                        if args.topology == 'routerless':
                            if data.get('sta_disabled') is not True:
                                raise ValueError('Readiness lacks fresh routerless STA-off admission')
                            if args.mode == 'receive' and (data.get('reverse_hotspot_required') is not True or data.get('reverse_hotspot_verified') is not False):
                                raise ValueError('Receive readiness lacks reverse-hotspot admission')
                        initial_radio = capture_radio('instrumentation-ready')
                        admitted = (lan_radio_admitted(initial_radio, station_admission) if args.topology == 'lan'
                                    else sender_hotspot_initial(initial_radio) if args.topology == 'sender-hotspot'
                                    else initial_radio['sta_disabled'] and initial_radio['no_ap_verified'])
                        if not admitted:
                            host_error = 'Fresh topology start lacks authoritative unchanged station/no-AP evidence'
                            STOP.set()
                            cancel_owned()
                        else:
                            data.update(station_admission=station_admission, initial_radio=initial_radio)
                            if args.topology == 'sender-hotspot':
                                if data.get('installed_identity') != installed_identity:
                                    raise ValueError('Initial installed identity mismatch')
                                if args.mode == 'publish':
                                    publisher_id = data.get('publisher_id')
                                    if str(uuid.UUID(publisher_id)) != publisher_id:
                                        raise ValueError('Initial publisher identity is not canonical')
                                save(out / 'instrumentation-ready.json', data)
                                with timed('ui_wait', 'controller_route_gate'):
                                    while not Path(args.route_gate).exists():
                                        if STOP.is_set():
                                            cancel_owned()
                                            raise RuntimeError('Cancelled before controller route gate')
                                        if time.time() >= deadline - 12:
                                            raise TimeoutError('Controller gate missed cleanup-reserved deadline')
                                        time.sleep(.05)
                                    gate = load_route_gate(args.route_gate, args)
                                fresh = capture_radio('controller-gate-admission')
                                if not sender_hotspot_initial(fresh):
                                    raise ValueError('Topology changed while waiting controller route gate')
                                initial_radio = fresh
                            evidence = dict(initial_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline))
                            if args.topology == 'sender-hotspot' and args.mode == 'receive':
                                evidence['sender_network_sha256'] = gate['sender_network_sha256']
                            send_control('ready_ack', evidence)
                    if args.mode == 'receive' and not STOP.is_set():
                        save(out / 'ready.json', data)
                if row['event'] == 'catalog-route':
                    route_radio = capture_radio('selected-sender-catalog-before-download')
                    admitted = (lan_radio_admitted(route_radio, initial_radio) if args.topology == 'lan' else
                                sender_hotspot_route(route_radio, 'receive') if args.topology == 'sender-hotspot' else
                                route_radio['sta_disabled'] and route_radio['current_ap_verified'] and initial_radio is not None and initial_radio['no_ap_verified'])
                    if not admitted:
                        host_error = 'Selected sender catalog lacks current admitted topology OS proof'
                        STOP.set()
                        cancel_owned()
                    elif not STOP.is_set():
                        evidence = dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline))
                        send_control('route_ack', evidence)
                if row['event'] == 'publishing':
                    if args.mode != 'publish' or any(str(data.get(key)) != str(value) for key, value in bound.items()):
                        raise ValueError('Publishing source/device binding mismatch')
                    radio_end = min(deadline, time.time() + 10)
                    while True:
                        route_radio = capture_radio('publishing')
                        if args.topology == 'lan' or route_radio['current_ap_verified'] or time.time() >= radio_end:
                            break
                        time.sleep(.2)
                    if args.topology == 'lan':
                        if not initial_radio or not lan_radio_admitted(route_radio, initial_radio) or data.get('lan_publication', {}).get('hotspot_active') is not False:
                            raise ValueError('LAN publisher lacks unchanged connected station/no-AP proof')
                    elif args.topology == 'sender-hotspot':
                        if (not initial_radio or not sender_hotspot_initial(initial_radio) or
                                not sender_hotspot_route(route_radio, 'publish') or
                                data.get('hotspot', {}).get('product_owned_ap_at_publish') is not True or
                                data.get('source_kind') != 'managed' or data.get('source_bytes_verified') is not True or
                                data.get('source_sha256') != args.sha256):
                            raise ValueError('Sender-hotspot publication lacks actual managed source/current owned AP')
                    elif (not initial_radio or not initial_radio['no_ap_verified'] or
                            not route_radio['sta_disabled'] or not route_radio['current_ap_verified'] or
                            not route_radio['ap_frequency_mhz'] or data.get('hotspot', {}).get('product_owned_ap_at_publish') is not True):
                        raise ValueError('Publisher lacks current OS and product-owned temporary AP proof')
                    if not STOP.is_set():
                        send_control('publish_ack', dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline)))
                if row['event'] == 'publisher-ready':
                    if (args.mode != 'publish' or data.get('status') != 'ready' or
                            any(str(data.get(key)) != str(value) for key, value in bound.items()) or
                            data.get('publisher_ready') is not True or data.get('no_ap_at_instrumentation_ready') is not True or
                            (args.topology == 'routerless' and any(data.get(key) is not True for key in ('sta_disabled', 'product_owned_ap_at_publish'))) or
                            not valid_publication(data.get('publication')) or data.get('installed_identity') != installed_identity or
                            (args.topology == 'sender-hotspot' and data.get('publication', {}).get('publisher_id') != publisher_id) or
                            route_radio is None or data.get('route_radio') != dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline))):
                        raise ValueError('Publisher-ready receipt lacks exact binding/topology/publication proof')
                    if not STOP.is_set():
                        data.update(station_admission=station_admission, initial_radio=initial_radio)
                        save(out / 'ready.json', data)
                        publisher_ready = True
                        ready_publication = data.get('publication')
                if row['event'] == 'result':
                    result = normalize_native_result(data, bound, installed_identity, api_level)
                if row['event'] == 'finish-retirement':
                    # The native receipt exists but the run never retired its
                    # instrumentation; never leave an earlier complete verdict.
                    retirement_failure = dict(data) if isinstance(data, dict) else {'error': str(data)}
                    result['status'] = 'failed'
                    result['finish_retirement'] = retirement_failure
                    if not result.get('error'):
                        result['error'] = retirement_failure.get('error') or 'Instrumentation retirement failed'
            code = process.wait(timeout=max(.1, deadline - time.time()))
            finish_timing_span(automation_span, code != 0 or result.get('status') != 'complete')
            automation_span = None
            if code != 0:
                raise RuntimeError('adb instrumentation failed: ' + str(code))
        for name, item in screenshots.items():
            if len(item['parts']) != item['chunks']:
                raise ValueError('Incomplete screenshot evidence')
            image = base64.b64decode(''.join(item['parts'][i] for i in range(item['chunks'])), validate=True)
            if hashlib.sha256(image).hexdigest() != item['sha256'] or not image.startswith(b'\x89PNG\r\n\x1a\n'):
                raise ValueError('Screenshot integrity mismatch')
            (out / name).write_bytes(image)
        if 'device_cleanup_result' not in result:
            raise ValueError('Missing exact native final result')
        if result.get('status') == 'complete':
            if api_level == 28 and args.mode == 'publish':
                validate_legacy_location(result.get('location_preflight'))
                if result['location_preflight'] != location_preflight:
                    raise ValueError('Final native location preflight receipt changed')
            if result.get('cleanup_complete') is not True:
                raise ValueError('Instrumentation owned cleanup incomplete')
            if args.mode != 'capture-crash' and result.get('preserved_before_books_settings_tabs') is not True:
                raise ValueError('Final books/settings/tabs preservation receipt missing')
            if crash_evidence is None or result.get('crash_report_preserved') is not True:
                raise ValueError('Final native crash preservation receipt missing')
            if args.mode == 'capture-crash':
                if (crash_evidence.get('present') is not True or result.get('ui_launched') is not False or
                        result.get('profile_read_only') is not True):
                    raise ValueError('Crash capture requires an owned record and no UI launch')
            elif reviewed_crash and args.mode in ('receive', 'publish'):
                closed = result.get('reviewed_crash_closed', {})
                if (crash_evidence.get('sha256') != reviewed_crash or closed.get('sha256') != reviewed_crash or
                        closed.get('ordinary_ui_close') is not True or
                        not all((out / name).is_file() for name in ('crash-before-close.png', 'crash-after-close.png'))):
                    raise ValueError('Reviewed crash lacks exact identity and ordinary UI close evidence')
            if args.mode in ('receive', 'verify') or (args.mode == 'publish' and result.get('source_kind') == 'managed'):
                if (result.get('receivedhash') != args.sha256 or result.get('receivedsize') != args.size or
                        result.get('managed') is not True):
                    raise ValueError('Final managed bytes receipt missing')
                if args.mode == 'publish' and (result.get('source_bytes_verified') is not True or
                                               result.get('source_sha256') != args.sha256):
                    raise ValueError('Managed publisher requires independent full actual source SHA256')
            elif args.mode == 'publish' and (result.get('source_kind') != 'provider' or result.get('source_bytes_verified') is not False or
                    result.get('managed') is not False or result.get('source_metadata') !=
                    dict(archive_id=args.archive_id, title=args.title, size=args.size, installation_id=result.get('installation_id')) or
                    not result.get('installation_id')):
                raise ValueError('Provider publication requires explicit limited installation metadata, not a claimed source hash')
            if args.mode == 'publish':
                if (not publisher_ready or cancel_sent or
                        any(result.get(key) is not True for key in ('publisher_ready',
                            'no_ap_at_instrumentation_ready', 'publication_stopped', 'owned_sharing_stopped')) or
                        (args.topology == 'routerless' and any(result.get(key) is not True for key in ('sta_disabled', 'product_owned_ap_at_publish'))) or
                        (not valid_publication(result.get('publication')) or result.get('publication') != ready_publication) or
                        result.get('route_radio') != dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline)) or
                        result.get('publication_stop_reason') not in ('STOP_PUBLISH', 'finite_deadline')):
                    raise ValueError('Publish completion requires bound readiness and actual owned publication stop')
                if stop_publish_sent and result.get('publication_stop_reason') != 'STOP_PUBLISH':
                    raise ValueError('Publisher did not acknowledge deliberate STOP_PUBLISH')
            if args.mode == 'receive' and (result.get('activebookid') != args.archive_id or result.get('readable_ui') is not True or not (out / 'reader.png').is_file()):
                raise ValueError('Receive requires active UUID, real reader UI and screenshot')
            if args.mode == 'receive' and args.topology == 'lan':
                if (not route_radio or not lan_radio_admitted(route_radio, initial_radio) or
                        result.get('route_radio') != dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline)) or
                        result.get('lan_route_verified') is not True):
                    raise ValueError('LAN receive requires unchanged connected station/no-AP route proof')
                validate_received_job(result, args)
            if args.mode == 'receive' and args.topology == 'sender-hotspot':
                if (not route_radio or not sender_hotspot_route(route_radio, 'receive') or
                        result.get('route_radio') != dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline)) or
                        any(result.get(key) is not True for key in ('sender_hotspot_route_verified',
                            'receiver_join_retired', 'receiver_consumer_retired', 'preserved_before_jobs'))):
                    raise ValueError('Sender-hotspot receive lacks admitted station route/owned retirement/job preservation')
                validate_received_job(result, args)
            if args.mode == 'receive' and args.topology == 'routerless' and (not initial_radio or not initial_radio['no_ap_verified'] or
                    not route_radio or not route_radio['current_ap_verified'] or not route_radio['sta_disabled'] or
                    any(result.get(key) is not True for key in ('sta_disabled', 'no_ap_at_instrumentation_ready',
                        'reverse_hotspot_required', 'reverse_hotspot_verified', 'product_owned_ap_at_catalog')) or
                    result.get('route_radio') != dict(route_radio, topology=args.topology, nonce=args.nonce, deadline=str(args.deadline))):
                raise ValueError('Receive requires fresh product reverse-hotspot topology, not direct LAN')
    except BaseException as error:
        if 'device_cleanup_result' in result and result.get('status') == 'failed' and result.get('error'):
            result['host_error'] = str(error)
        else:
            result.update(status='failed', error=str(error))
        host_error = host_error or str(error)
    finally:
        cleanup_span = begin_cleanup_timing(ui_span)
        ui_span = None
        ui_wait_operation = None
        if process is not None:
            try:
                cancel_owned()
            except Exception as error:
                result['cancellation_error'] = type(error).__name__
            if not stream_complete:
                # A cancelled bounded command may have interrupted decoding.
                # Drain the owned runner's final cleanup receipt before relaunch.
                with (out / 'instrumentation.log').open('a') as cleanup_log:
                    while time.time() < deadline and not stream_complete:
                        try:
                            line = lines.get(timeout=min(.25, max(.01, deadline - time.time())))
                        except queue.Empty:
                            continue
                        if line is None:
                            stream_complete = True
                            break
                        cleanup_log.write(line)
                        cleanup_log.flush()
                        if PREFIX not in line:
                            continue
                        try:
                            row = json.loads(line.split(PREFIX, 1)[1].strip())
                            if row.get('nonce') != args.nonce or row.get('topology') != args.topology:
                                continue
                            if row['event'] == 'control-ready':
                                control_ready = True
                                cancel_owned()
                            if row['event'] == 'result':
                                native = row['data']
                                result = normalize_native_result(native, bound, installed_identity, api_level, host_failure=result)
                            if row['event'] == 'finish-retirement':
                                retirement_failure = dict(row['data']) if isinstance(row['data'], dict) else {'error': str(row['data'])}
                                result['status'] = 'failed'
                                result['finish_retirement'] = retirement_failure
                                if not result.get('error'):
                                    result['error'] = retirement_failure.get('error') or 'Instrumentation retirement failed'
                        except (ValueError, KeyError, RuntimeError):
                            continue
            if retirement_failure is not None and result.get('status') == 'complete':
                # ADB can still exit zero after the target retired itself.
                result['status'] = 'failed'
                result['finish_retirement'] = retirement_failure
                if not result.get('error'):
                    result['error'] = retirement_failure.get('error') or 'Instrumentation retirement failed'
            # Let only our in-process watchdog finish this owned instrumentation;
            # do not force-stop the app or cancel any transfer. Its lifetime is
            # bounded even if the host decoder encountered an earlier failure.
            try:
                process.wait(timeout=max(.1, deadline - time.time() + .5))
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)
        finish_timing_span(automation_span, result.get('status') != 'complete')
        # Instrumentation finish stops its target. Reopen the ordinary app without
        # force-stop, data clearing, tab closure, or resetting any network/settings.
        if process is not None and args.mode != 'capture-crash':
            try:
                relaunched = run(adb + ['shell', 'am', 'start', '-n', TARGET + '/.MainActivity'], timeout=5,
                                 timing_category='adb_command', timing_label='normal_app_relaunch')
                result['cleanup'] = {'normal_app_relaunched': 'Error' not in relaunched}
                result['cleanup_complete'] = bool(result.get('cleanup_complete') and 'Error' not in relaunched)
                if (args.mode == 'publish' or args.topology in ('lan', 'sender-hotspot')) and args.mode in ('receive', 'publish'):
                    radio_end = min(deadline, time.time() + 4)
                    while True:
                        final_radio = capture_radio('after-publication-stop-normal-relaunch', interruptible=False)
                        retired = (sender_hotspot_initial(final_radio) if args.topology == 'sender-hotspot'
                                   and args.mode == 'receive' else final_radio['no_ap_verified'])
                        if retired or time.time() >= radio_end:
                            break
                        time.sleep(.2)
                    result['final_radio'] = final_radio
                    if (not final_radio['no_ap_verified'] or
                            (args.topology == 'lan' and not lan_radio_admitted(final_radio, station_admission)) or
                            (args.topology == 'sender-hotspot' and args.mode == 'receive' and not sender_hotspot_initial(final_radio))):
                        result.update(status='failed', cleanup_complete=False,
                                      cleanup_error='Final current OS no-AP/unchanged LAN station was not proven')
                if 'Error' in relaunched:
                    result.update(status='failed', cleanup_error='Normal app relaunch rejected')
            except Exception as error:
                result.update(status='failed', cleanup_complete=False, cleanup_error=str(error))
        result['output'] = str(out)
        if STOP.is_set() and not (args.mode == 'publish' and STOP_SIGNAL == signal.SIGTERM and
                                  stop_publish_sent and not cancel_sent and host_error is None and
                                  result.get('status') == 'complete'):
            result.update(status='failed', cancellation_requested=True)
            result.setdefault('error', host_error or 'Host cancellation requested')
            if host_error:
                result['host_error'] = host_error
        result['station_admission'] = station_admission
        result['initial_radio'] = initial_radio
        result['radio_evidence'] = radio
        result['timings_ms'] = timings
        result['timings_ms_clock'] = 'native elapsedRealtime stage snapshots; separate from host timing'
        finish_timing_span(cleanup_span, result.get('cleanup_complete') is not True)
        TIMING.close()
        result['timing'] = TIMING.summary()
        result['timing_scope'] = 'After frozen host admission through owned cleanup; ADB wall includes device execution; UI waits are host-observed'
        TIMING = None
        save(out / 'result.json', result)
        print(PREFIX + json.dumps({'event': 'host_result', 'nonce': args.nonce, 'topology': args.topology, 'data': result}, sort_keys=True), flush=True)
    return 0 if result.get('status') == 'complete' else 1


def main():
    global STOP_SIGNAL
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--prepared-sha256', help='Expected frozen prepare-receipt SHA256; checked before any device access')
    parser.add_argument('--prepared', default=str(DEFAULT_PREPARED / 'prepare.json'))
    parser.add_argument('--target-apk', help='Reviewed optimized release APK; required for prepare, never installed')
    parser.add_argument('--android-sdk')
    parser.add_argument('--build-tools')
    parser.add_argument('--java-home')
    parser.add_argument('--adb')
    parser.add_argument('--keystore', default='~/.android/debug.keystore')
    parser.add_argument('--key-alias', default='androiddebugkey')
    parser.add_argument('--keystore-password', default='android')
    parser.add_argument('--key-password', default='android')
    parser.add_argument('--device')
    parser.add_argument('--mode', choices=('receive', 'publish', 'verify', 'capture-crash'))
    parser.add_argument('--topology', choices=('routerless', 'lan', 'sender-hotspot'), default='routerless')
    parser.add_argument('--route-gate', help='Exact controller-owned sender-hotspot gate JSON; never creates it')
    parser.add_argument('--reviewed-crash-sha256',
                        help='Exact reviewed pending.json full-file SHA256; frozen at prepare, inherited at run')
    parser.add_argument('--archive-id')
    parser.add_argument('--file-name')
    parser.add_argument('--title')
    parser.add_argument('--size', type=int)
    parser.add_argument('--sha256')
    parser.add_argument('--peer')
    parser.add_argument('--deadline', type=int)
    parser.add_argument('--nonce')
    parser.add_argument('--sender-device')
    parser.add_argument('--output')
    args = parser.parse_args()
    previous = {}
    if not args.prepare_only:
        STOP.clear()
        STOP_SIGNAL = None
        for value in (signal.SIGTERM, signal.SIGINT):
            previous[value] = signal.signal(value, stop_requested)
    try:
        return prepare(args) if args.prepare_only else execute(args)
    except Exception as error:
        print('android-nearby: ' + str(error), file=sys.stderr)
        return 1
    finally:
        for value, handler in previous.items():
            signal.signal(value, handler)


if __name__ == '__main__':
    raise SystemExit(main())
