#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Process-cold physical Android memory comparison; Python 3 + Pillow.

Prepare BOTH apps at the same actual Animal article/scroll position first. Supply
one trusted screenshot of that prepared state per app (not interchangeable UI).
Example (output MUST NOT exist):
  python3 scripts/profile-android-memory.py --adb /path/to/adb \
    --serial SERIAL --model 'EXACT MODEL' --output /tmp/memory-new \
    --reference-ours /path/ours.png --reference-official /path/official.png

Default ABBA: four cold launches, four samples each (16 samples TOTAL, eight per
app), 30 seconds settling per launch, two seconds between samples. --blocks 8
uses ABBAABBA (32 total). No installation, navigation, AX, grants, settings,
cache clearing, forced GC, or instrumentation. Stops ONLY the two named apps;
finally stops both and presses Home, including on failure. Existing reading
state must survive process-cold launches; otherwise reference admission fails.

Article-crop pixel agreement is ONLY agreement with the operator's references,
not independent semantic proof of article identity. References must be actual
prepared Animal states. PSS includes shared mappings and possibly graphics
mtrack; these are observational cold-launch comparisons, not retained-heap or
causal allocation measurements. No emulator/smaps results are pooled.

Optional --measure-startup also polls actual screenshots for up to 15 seconds
after each explicit launch. Requires an unlocked, verified Home foreground and
a nonmatching prelaunch crop. Reports conservative host-monotonic pixel-ready
intervals, including screenshot polling overhead, NOT Android frame timing or
semantic article proof. am start -W logs remain separate, not user-ready times.
The 30-second PSS settle begins AFTER pixel readiness in this mode. Startup has
only two launches per app (four with --blocks 8), not the PSS sample count.
"""

import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import signal
import statistics
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageStat

CROP = (0.08, 0.08, 0.92, 0.80)
MAX_MAD = 5.0  # Mean absolute RGB channel difference, in 0..255 units.
DEADLINE_SECONDS = 900
CLEANUP_RESERVE = 45
SAMPLES = 4
SETTLE = 30
STARTUP_SECONDS = 15
STARTUP_POLL_SECONDS = 0.05
METRICS = {
    'pssKiB': 'Total PSS', 'mainPssKiB': 'Main PSS',
    'rendererPssKiB': 'WebView renderer PSS', 'otherPssKiB': 'Other owned PSS',
    'nativeHeapPssKiB': 'Native heap PSS', 'apkPssKiB': '.apk mmap PSS',
    'soPssKiB': '.so mmap PSS', 'mappedCodePssKiB': '.apk + .so PSS',
    'unknownPssKiB': 'Unknown row PSS', 'graphicsPssKiB': 'Graphics summary PSS',
    'nativeHeapAllocKiB': 'Native heap Alloc', 'nativeHeapSizeKiB': 'Native heap Size',
}


def stamp():
    return {'unix': time.time(), 'monotonic': time.monotonic()}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def package(value):
    if not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+', value):
        raise argparse.ArgumentTypeError('Expected a dotted Android package name')
    return value


def parse_meminfo(raw, pid):
    if not re.search(r'\*\* MEMINFO in pid ' + re.escape(pid) + r' \[', raw):
        raise RuntimeError('meminfo PID does not match requested process')
    values = {}
    for key, label in [('pssKiB', 'TOTAL PSS'), ('rssKiB', 'TOTAL RSS'),
                       ('swapPssKiB', 'TOTAL SWAP PSS')]:
        matches = re.findall(re.escape(label) + r':\s*(\d+)', raw)
        if len(matches) != 1:
            raise RuntimeError('Missing/ambiguous metric: ' + label)
        values[key] = int(matches[0])
    # Detailed table rows, not the differently defined App Summary Native Heap.
    table = raw.split('App Summary', 1)[0]
    for key, label in [('nativeHeapPssKiB', 'Native Heap'), ('apkPssKiB', '.apk mmap'),
                       ('soPssKiB', '.so mmap'), ('unknownPssKiB', 'Unknown')]:
        matches = re.findall(r'^\s*' + re.escape(label) + r'\s+([\d \t]+)$', table, re.M)
        if len(matches) != 1:
            raise RuntimeError('Missing/ambiguous detailed meminfo row: ' + label)
        row = [int(n) for n in matches[0].split()]
        if len(row) < 5:
            raise RuntimeError('Unsupported meminfo table layout: ' + label)
        values[key] = row[0]
        if label == 'Native Heap':
            values['nativeHeapSizeKiB'] = row[5] if len(row) == 8 else None
            values['nativeHeapAllocKiB'] = row[6] if len(row) == 8 else None
    graphics = re.findall(r'^\s*Graphics:\s*(\d+)', raw, re.M)
    if len(graphics) != 1:
        raise RuntimeError('Missing/ambiguous graphics summary')
    values['graphicsPssKiB'] = int(graphics[0])
    values['mappedCodePssKiB'] = values['apkPssKiB'] + values['soPssKiB']
    return values


class Profiler:
    def __init__(self, args, references):
        self.args = args
        self.packages = {'ours': args.ours, 'official': args.official}
        self.output = args.output
        self.started = time.monotonic()
        self.deadline = self.started + DEADLINE_SECONDS
        self.cleanup = False
        self.admitted = False
        self.commands = self.output.joinpath('commands.jsonl').open('x')
        self.references = references
        self.report = {
            'schema': 1, 'status': 'running', 'started': stamp(),
            'toolSha256': digest(Path(__file__).read_bytes()),
            'serial': args.serial, 'model': args.model, 'adb': args.adb,
            'packages': self.packages, 'order': list('ABBA' * (args.blocks // 4)),
            'samplesPerBlock': SAMPLES, 'settleSeconds': SETTLE,
            'intersampleSeconds': 2, 'hardDeadlineSeconds': DEADLINE_SECONDS,
            'metric': 'Sum dumpsys TOTAL PSS of ALL exclusively owned ActivityManager processes; requires a WebView renderer',
            'limitations': [
                'Reference-crop agreement is not independent semantic proof of article identity.',
                'PSS shared mapping attribution depends on other processes; cold means process-cold, not cold page cache.',
                'Separate category medians are not additive; graphics summary and detailed PSS categories can overlap.',
                'Adjacent AB/BA block-median differences are descriptive, not causal or significance estimates.',
            ],
            'references': {}, 'blocks': [], 'cleanup': [],
        }
        if args.measure_startup:
            self.report['startupMeasurement'] = {
                'maximumSecondsPerLaunch': STARTUP_SECONDS,
                'sleepBetweenAcquisitionsSeconds': STARTUP_POLL_SECONDS,
                'clock': 'Host time.monotonic; t0 immediately before Popen(am start -W)',
                'bounds': '[last mismatch acquisition start - t0 (or 0), first match acquisition end - t0]',
                'settleStarts': 'After admitted pixel readiness and successful launch-process completion',
                'limitations': [
                    'Screenshot polling and adb acquisition overhead are included; no FPS or Android frame accuracy.',
                    'Crop threshold agreement with trusted references is not semantic article identity proof.',
                    'A match is an observed crop, not a guarantee of sustained readiness or interactivity.',
                    'am start -W timings are launch logs only, not user-ready timings.',
                ],
            }
        for alias, ref in references.items():
            data = ref['bytes']
            target = self.output / ('reference-' + alias + '.png')
            target.write_bytes(data)
            self.report['references'][alias] = {
                'source': str(ref['path']), 'copy': target.name, 'sha256': digest(data),
                'dimensions': list(ref['image'].size), 'normalizedCrop': CROP,
                'maximumMeanAbsoluteRGBDifference': MAX_MAD,
            }
        self.persist()

    def persist(self):
        path = self.output / 'report.json'
        # The directory is exclusively reserved by this run. Keep partials readable.
        temporary = self.output / 'report.partial.json'
        temporary.write_text(json.dumps(self.report, indent=2) + '\n')
        temporary.replace(path)

    def remaining(self):
        limit = self.deadline if self.cleanup else self.deadline - CLEANUP_RESERVE
        remaining = limit - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('Overall profiling deadline reached')
        return remaining

    def adb(self, *args, binary=False, timeout=25, deadline=None):
        command = [self.args.adb, '-s', self.args.serial, *args]
        evidence = {'argv': command, 'started': stamp()}
        result = None
        try:
            allowed = min(timeout, self.remaining())
            if deadline is not None:
                allowed = min(allowed, deadline - time.monotonic())
            if allowed <= 0:
                raise TimeoutError('Command deadline reached')
            result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                                    timeout=allowed, check=False)
            evidence.update({'returnCode': result.returncode, 'stdoutBytes': len(result.stdout),
                             'stdoutSha256': digest(result.stdout),
                             'stderr': result.stderr.decode(errors='replace')})
            if not binary:
                evidence['stdout'] = result.stdout.decode(errors='replace')
            if result.returncode:
                raise RuntimeError('adb command failed: ' + repr(command) + ': ' + evidence['stderr'])
            return result.stdout if binary else evidence['stdout']
        except BaseException as error:
            evidence['failure'] = {'type': type(error).__name__, 'message': str(error)}
            if result is None:
                for name in ('stdout', 'stderr'):
                    partial = getattr(error, name, None)
                    if partial:
                        evidence[name] = partial.decode(errors='replace') if isinstance(partial, bytes) else partial
            raise
        finally:
            evidence['ended'] = stamp()
            self.commands.write(json.dumps(evidence) + '\n')
            self.commands.flush()

    def sleep(self, seconds):
        if self.remaining() < seconds:
            raise TimeoutError('Insufficient time before overall deadline')
        time.sleep(seconds)

    def guard(self):
        # Once per block, never enumerate or probe any other attached device.
        if self.adb('get-serialno').strip() != self.args.serial:
            raise RuntimeError('Physical serial mismatch')
        if self.adb('shell', 'getprop', 'ro.product.model').strip() != self.args.model:
            raise RuntimeError('Physical model mismatch')
        if self.adb('shell', 'getprop', 'ro.kernel.qemu').strip() == '1':
            raise RuntimeError('Emulators are not admitted')
        self.admitted = True

    def unlocked(self, deadline=None):
        user = self.adb('shell', 'am', 'get-current-user', deadline=deadline).strip()
        if not user.isdigit():
            raise RuntimeError('Cannot identify current Android user')
        trust = self.adb('shell', 'dumpsys', 'trust', deadline=deadline)
        users = re.split(r'(?m)(?=^[ \t]*User ")', trust)
        selected = [block for block in users
                    if re.match(r'[ \t]*User .*\(id=' + user + r'[,)]', block)]
        if len(selected) != 1 or not re.search(r'\bdeviceLocked=(?:0|false)\b', selected[0]):
            raise RuntimeError('Current-user unlocked/keyguard state could not be admitted')
        return {'currentUser': user, 'unlocked': True}

    def foreground(self, alias, deadline=None):
        pkg = self.packages[alias]
        raw = self.adb('shell', 'dumpsys', 'activity', 'activities', deadline=deadline)
        resumed = [line for line in raw.splitlines()
                   if 'topResumedActivity=' in line or 'mResumedActivity:' in line]
        if not resumed or not all(pkg + '/' in line for line in resumed):
            raise RuntimeError('Expected benchmark package foreground: ' + pkg)
        state = self.unlocked(deadline)
        return {'checked': stamp(), 'resumed': resumed, **state}

    def launcher(self):
        # Resolve the actual Home handler; never assume a vendor launcher package.
        resolved = self.adb('shell', 'cmd', 'package', 'resolve-activity', '--brief',
                            '-a', 'android.intent.action.MAIN',
                            '-c', 'android.intent.category.HOME').strip().splitlines()
        component = resolved[-1] if resolved else ''
        if not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+/[A-Za-z0-9_.$]+', component):
            raise RuntimeError('Cannot resolve a unique Home activity')
        if component.split('/', 1)[0] in self.packages.values():
            raise RuntimeError('Home activity belongs to a benchmark package')
        raw = self.adb('shell', 'dumpsys', 'activity', 'activities')
        resumed = [line for line in raw.splitlines()
                   if 'topResumedActivity=' in line or 'mResumedActivity:' in line]
        if not resumed or not all(component in line.split() for line in resumed):
            raise RuntimeError('Resolved Home activity is not exclusively foreground')
        state = self.unlocked()
        return {'checked': stamp(), 'component': component, 'resumed': resumed, **state}

    def startup_frame(self, alias, label, frames, deadline=None):
        target = self.output / (label + '.png')
        frame = {'path': target.name, 'acquisitionStart': stamp()}
        frames.append(frame)
        self.persist()
        try:
            data = self.adb('exec-out', 'screencap', '-p', binary=True, deadline=deadline)
            frame['acquisitionEnd'] = stamp()
            target.write_bytes(data)
            frame['sha256'] = digest(data)
            image = Image.open(io.BytesIO(data))
            if image.format != 'PNG':
                raise RuntimeError('Device screenshot is not PNG')
            image = image.convert('RGB')
            reference = self.references[alias]['image']
            if image.size != reference.size:
                raise RuntimeError('Screenshot/reference dimensions differ')
            width, height = image.size
            box = tuple(round(n * (width if i % 2 == 0 else height)) for i, n in enumerate(CROP))
            mad = statistics.mean(ImageStat.Stat(
                ImageChops.difference(image.crop(box), reference.crop(box))).mean)
            frame.update({
                'pixelCrop': box, 'normalizedCrop': CROP,
                'meanAbsoluteRGBDifference': mad, 'threshold': MAX_MAD,
                'referenceSha256': digest(self.references[alias]['bytes']),
                'matchesReference': mad <= MAX_MAD,
            })
            return frame
        except BaseException as error:
            frame['failure'] = {'type': type(error).__name__, 'message': str(error)}
            raise
        finally:
            self.persist()

    def startup(self, alias, component, block):
        prefix = f'block-{block["index"] + 1}-{alias}-startup'
        observation = {'status': 'running', 'prelaunchFrames': [], 'frames': []}
        block['startup'] = observation
        self.persist()
        self.adb('shell', 'input', 'keyevent', 'KEYCODE_HOME')
        observation['launcherBefore'] = self.launcher()
        before = self.startup_frame(alias, prefix + '-prelaunch', observation['prelaunchFrames'])
        observation['launcherAfter'] = self.launcher()
        self.persist()
        if before['matchesReference']:
            raise RuntimeError('Prelaunch Home crop already matches prepared article; timing rejected')
        command = [self.args.adb, '-s', self.args.serial, 'shell', 'am', 'start', '-W', '-n', component]
        stdout_path = self.output / (prefix + '-am-start-stdout.txt')
        stderr_path = self.output / (prefix + '-am-start-stderr.txt')
        launch = {'argv': command, 'stdoutPath': stdout_path.name, 'stderrPath': stderr_path.name}
        observation['launchProcess'] = launch
        process = None
        try:
            with stdout_path.open('xb') as stdout, stderr_path.open('xb') as stderr:
                allowed = min(STARTUP_SECONDS, self.remaining())
                t0 = stamp()
                observation['t0'] = t0
                deadline = t0['monotonic'] + allowed
                launch.update({'started': t0, 'deadlineMonotonic': deadline})
                process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
                launch['hostPid'] = process.pid
                self.persist()
                while True:
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Prepared article did not become pixel-ready within startup deadline')
                    frame = self.startup_frame(alias, prefix + f'-{len(observation["frames"]) + 1:03d}',
                                               observation['frames'], deadline)
                    if frame['matchesReference']:
                        observation['firstMatch'] = frame
                        self.persist()
                        frame['foreground'] = self.foreground(alias, deadline)
                        if time.monotonic() >= deadline:
                            raise TimeoutError('Pixel-ready admission exceeded startup deadline')
                        last = observation.get('lastMismatch')
                        observation['boundsSeconds'] = {
                            'lower': last['acquisitionStart']['monotonic'] - t0['monotonic'] if last else 0.0,
                            'upper': frame['acquisitionEnd']['monotonic'] - t0['monotonic'],
                        }
                        observation['pixelReadyAdmitted'] = stamp()
                        break
                    observation['lastMismatch'] = frame
                    self.persist()
                    delay = min(STARTUP_POLL_SECONDS, deadline - time.monotonic())
                    if delay <= 0:
                        raise TimeoutError('Prepared article did not become pixel-ready within startup deadline')
                    self.sleep(delay)
                timeout = min(self.remaining(), deadline - time.monotonic())
                if timeout <= 0:
                    raise TimeoutError('Launch process did not complete within startup deadline')
                process.wait(timeout=timeout)
        except BaseException as error:
            observation.update({'status': 'rejected', 'failure': {'type': type(error).__name__, 'message': str(error)}})
            launch['failure'] = observation['failure']
            raise
        finally:
            cleanup = {'killAttempted': False, 'terminationConfirmed': process is None, 'errors': []}
            launch['cleanup'] = cleanup
            try:
                if process is not None:
                    try:
                        if process.poll() is None:
                            # This exact Popen is owned by the run; never kill the adb server.
                            cleanup['killAttempted'] = True
                            process.kill()
                    except BaseException as error:
                        cleanup['errors'].append({'operation': 'kill', 'type': type(error).__name__,
                                                  'message': str(error)})
                    try:
                        process.wait(timeout=max(0, min(5, self.deadline - time.monotonic())))
                        cleanup['terminationConfirmed'] = True
                    except BaseException as error:
                        cleanup['errors'].append({'operation': 'reap', 'type': type(error).__name__,
                                                  'message': str(error)})
                    launch['returnCode'] = process.returncode
            finally:
                # Preserve the launch receipt even when kill/reap fails; outer
                # stop-both/Home cleanup must still get its reserved deadline.
                for name, path in [('stdout', stdout_path), ('stderr', stderr_path)]:
                    if path.exists():
                        data = path.read_bytes()
                        launch[name + 'Bytes'] = len(data)
                        launch[name + 'Sha256'] = digest(data)
                        launch[name] = data.decode(errors='replace')
                launch['ended'] = stamp()
                self.commands.write(json.dumps(launch) + '\n')
                self.commands.flush()
                self.persist()
            if cleanup['errors'] or not cleanup['terminationConfirmed']:
                failure = {'type': 'LaunchCleanupError', 'message': 'Owned launch process cleanup could not be admitted'}
                observation.update({'status': 'rejected', 'failure': failure})
                launch['failure'] = failure
                self.persist()
                raise RuntimeError(failure['message'])
        block['launch'] = launch['stdout']
        if launch['returnCode'] != 0 or not re.search(r'^Status:\s*ok\s*$', block['launch'], re.M) or re.search(
                r'Error:|Exception', block['launch'] + launch['stderr']):
            observation['status'] = 'rejected'
            self.persist()
            raise RuntimeError('Activity launch was not successful')
        observation['status'] = 'complete'
        observation['settleStarted'] = stamp()
        self.persist()

    def screenshot(self, alias, label):
        state = self.foreground(alias)
        data = self.adb('exec-out', 'screencap', '-p', binary=True)
        target = self.output / (label + '.png')
        target.write_bytes(data)
        image = Image.open(io.BytesIO(data))
        if image.format != 'PNG':
            raise RuntimeError('Device screenshot is not PNG')
        image = image.convert('RGB')
        reference = self.references[alias]['image']
        if image.size != reference.size:
            raise RuntimeError('Screenshot/reference dimensions differ')
        width, height = image.size
        box = tuple(round(n * (width if i % 2 == 0 else height)) for i, n in enumerate(CROP))
        difference = ImageChops.difference(image.crop(box), reference.crop(box))
        mad = statistics.mean(ImageStat.Stat(difference).mean)
        result = {'path': target.name, 'sha256': digest(data), 'foreground': state,
                  'pixelCrop': box, 'normalizedCrop': CROP, 'meanAbsoluteRGBDifference': mad,
                  'threshold': MAX_MAD, 'referenceSha256': digest(self.references[alias]['bytes']),
                  'matchesReference': mad <= MAX_MAD}
        # Persist rejection evidence before throwing, not just a message.
        self.report['blocks'][-1][label.rsplit('-', 1)[-1] + 'Screenshot'] = result
        self.persist()
        if mad > MAX_MAD:
            raise RuntimeError(f'{alias} article crop differs from prepared reference: {mad:.3f} > {MAX_MAD}')
        return result

    def ownership(self, alias):
        pkg = self.packages[alias]
        observation = {'started': stamp(), 'selected': {}, 'shared': [], 'procStats': {}}
        raw = self.adb('shell', 'dumpsys', 'activity', 'processes')
        # Full raw output is in the durable commands journal, referenced by time.
        observation['activityProcessesSha256'] = digest(raw.encode())
        for block in re.split(r'(?m)(?=^  \*APP\* )', raw):
            header = re.match(r'  \*APP\* .*?ProcessRecord\{[^ ]+ (\d+):([^/ ]+)', block)
            owner = re.search(r'(?m)^\s+packageList=\{([^}]+)\}', block)
            if not header or not owner:
                continue
            packages = {item.strip() for item in owner[1].split(',')}
            if pkg not in packages:
                continue
            if packages != {pkg}:
                observation['shared'].append(block)
                continue
            pid = header[1]
            stat = self.adb('shell', 'cat', '/proc/' + pid + '/stat')
            observation['procStats'][pid] = stat
            fields = stat[stat.rfind(')') + 2:].split()
            if ')' not in stat or len(fields) <= 19 or not fields[19].isdigit() or stat.split(' ', 1)[0] != pid:
                raise RuntimeError('Unparseable process generation identity')
            if pid in observation['selected']:
                raise RuntimeError('Duplicate ActivityManager process record')
            observation['selected'][pid] = {'name': header[2], 'startTicks': fields[19]}
        main = self.adb('shell', 'pidof', pkg).strip()
        observation['mainPid'] = main
        selected = observation['selected']
        if observation['shared']:
            raise RuntimeError('Shared ActivityManager package ownership; cannot attribute all processes')
        if not re.fullmatch(r'\d+', main) or main not in selected or selected[main]['name'] != pkg:
            raise RuntimeError('No unique exclusively attributed main process')
        renderers = [pid for pid, item in selected.items() if pid != main
                     and ':sandboxed_process' in item['name']
                     and 'org.chromium.content.app.SandboxedProcessService' in item['name']]
        if not renderers:
            raise RuntimeError('No exclusively owned WebView renderer; main-only samples rejected')
        observation.update({'rendererPids': renderers, 'ended': stamp()})
        return observation

    @staticmethod
    def same_generation(left, right):
        if left['selected'] != right['selected'] or left['mainPid'] != right['mainPid']:
            raise RuntimeError('Owned process set/generation changed; sample/block rejected')

    def identity(self, alias):
        pkg = self.packages[alias]
        raw = self.adb('shell', 'pm', 'path', pkg)
        paths = [line.removeprefix('package:') for line in raw.splitlines() if line.startswith('package:')]
        if not paths or len(paths) != len(set(paths)) or any(not re.fullmatch(r'/[A-Za-z0-9_./+=@~-]+\.apk', p) for p in paths):
            raise RuntimeError('No valid unique installed base/split APK paths for ' + pkg)
        apks = []
        for path in paths:
            sha = self.adb('shell', 'sha256sum', path).split()
            size = self.adb('shell', 'stat', '-c', '%s', path).strip()
            if not sha or not re.fullmatch(r'[0-9a-fA-F]{64}', sha[0]) or not size.isdigit() or int(size) <= 0:
                raise RuntimeError('Invalid installed APK hash/size')
            apks.append({'devicePath': path, 'sha256': sha[0].lower(), 'bytes': int(size)})
        raw_package = self.adb('shell', 'dumpsys', 'package', pkg)
        versions = [line.strip() for line in raw_package.splitlines()
                    if re.match(r'\s*(?:versionCode|versionName)=', line)]
        if not any(line.startswith('versionCode=') for line in versions):
            raise RuntimeError('No installed package versionCode')
        return {'apks': apks, 'versions': versions, 'packageDumpSha256': digest(raw_package.encode())}

    def inventory(self):
        return {
            'packages': {alias: self.identity(alias) for alias in self.packages},
            'api': self.adb('shell', 'getprop', 'ro.build.version.sdk').strip(),
            'ram': self.adb('shell', 'cat', '/proc/meminfo'),
            'webview': self.adb('shell', 'dumpsys', 'webviewupdate'),
            'thermal': self.adb('shell', 'dumpsys', 'thermalservice'),
            'battery': self.adb('shell', 'dumpsys', 'battery'),
        }

    def block(self, index, alias):
        block = {'index': index, 'alias': alias, 'status': 'running', 'started': stamp(), 'samples': []}
        self.report['blocks'].append(block)
        self.persist()
        print(f'Block {index + 1}/{self.args.blocks}: {alias}: identity guard, process-cold launch, {SETTLE}s settle', flush=True)
        self.guard()
        if index == 0:
            self.report['inventoryBefore'] = self.inventory()
            self.persist()
        # Only the two explicitly selected benchmark apps may be force-stopped.
        for pkg in self.packages.values():
            self.adb('shell', 'am', 'force-stop', pkg)
        pkg = self.packages[alias]
        resolved = self.adb('shell', 'cmd', 'package', 'resolve-activity', '--brief', pkg).strip().splitlines()
        component = resolved[-1] if resolved else ''
        if not re.fullmatch(re.escape(pkg) + r'/[A-Za-z0-9_.$]+', component):
            raise RuntimeError('Unexpected resolved launch component')
        block['component'] = component
        if self.args.measure_startup:
            self.startup(alias, component, block)
        else:
            launch = self.adb('shell', 'am', 'start', '-W', '-n', component)
            block['launch'] = launch
            if not re.search(r'^Status:\s*ok\s*$', launch, re.M) or re.search(r'Error:|Exception', launch):
                raise RuntimeError('Activity launch was not successful')
        self.sleep(SETTLE)
        self.screenshot(alias, f'block-{index + 1}-{alias}-before')
        identity = self.ownership(alias)
        block['initialOwnership'] = identity
        self.persist()
        for sample_index in range(SAMPLES):
            sample = {'index': sample_index, 'status': 'running', 'started': stamp(), 'processes': {}}
            block['samples'].append(sample)
            self.persist()
            sample['foregroundBefore'] = self.foreground(alias)
            before = self.ownership(alias)
            sample['before'] = before
            self.same_generation(identity, before)
            for pid, generation in identity['selected'].items():
                raw = self.adb('shell', 'dumpsys', 'meminfo', pid)
                path = self.output / f'block-{index + 1}-sample-{sample_index + 1}-pid-{pid}-meminfo.txt'
                path.write_text(raw)
                process = {'identity': generation, 'rawPath': path.name, 'rawSha256': digest(raw.encode())}
                sample['processes'][pid] = process
                self.persist()
                process['metrics'] = parse_meminfo(raw, pid)
            after = self.ownership(alias)
            sample['after'] = after
            self.same_generation(identity, after)
            sample['foregroundAfter'] = self.foreground(alias)
            totals = {}
            for key in ('pssKiB', 'rssKiB', 'swapPssKiB', *list(METRICS)[4:]):
                values = [p['metrics'][key] for p in sample['processes'].values()]
                totals[key] = sum(values) if all(v is not None for v in values) else None
            totals['mainPssKiB'] = sample['processes'][identity['mainPid']]['metrics']['pssKiB']
            totals['rendererPssKiB'] = sum(sample['processes'][pid]['metrics']['pssKiB'] for pid in identity['rendererPids'])
            totals['otherPssKiB'] = totals['pssKiB'] - totals['mainPssKiB'] - totals['rendererPssKiB']
            sample.update({'metrics': totals, 'status': 'complete', 'ended': stamp()})
            self.persist()
            print(f'  sample {sample_index + 1}/{SAMPLES}: {totals["pssKiB"] / 1024:.2f} MiB total PSS', flush=True)
            if sample_index + 1 < SAMPLES:
                self.sleep(2)
        self.screenshot(alias, f'block-{index + 1}-{alias}-after')
        block.update({'status': 'complete', 'ended': stamp()})
        self.persist()

    def summarize(self):
        cohorts = {}
        for alias in self.packages:
            samples = [s['metrics'] for b in self.report['blocks'] if b['alias'] == alias for s in b['samples']]
            cohorts[alias] = {}
            for key in METRICS:
                values = [s[key] for s in samples]
                cohorts[alias][key] = ({'medianKiB': statistics.median(values), 'minimumKiB': min(values),
                                       'maximumKiB': max(values), 'sampleCount': len(values)}
                                      if all(v is not None for v in values) else None)
        pairs = []
        for i in range(0, len(self.report['blocks']), 2):
            blocks = self.report['blocks'][i:i + 2]
            pair = {b['alias']: b for b in blocks}
            differences = {}
            for key in METRICS:
                values = {alias: [s['metrics'][key] for s in pair[alias]['samples']] for alias in self.packages}
                differences[key] = (statistics.median(values['ours']) - statistics.median(values['official'])
                                    if all(v is not None for vs in values.values() for v in vs) else None)
            pairs.append({'blockIndices': [b['index'] for b in blocks], 'oursMinusOfficialKiB': differences})
        self.report['summary'] = {'cohorts': cohorts, 'adjacentBlockPairs': pairs}
        lines = ['# Physical Android process-cold memory', '',
                 '| Metric (MiB) | Ours median [min–max] | Official/native median [min–max] | Median paired Δ (ours − official) |',
                 '|---|---:|---:|---:|']
        for key, title in METRICS.items():
            cells = []
            for alias in self.packages:
                value = cohorts[alias][key]
                cells.append('unavailable' if value is None else
                             f'{value["medianKiB"] / 1024:.2f} [{value["minimumKiB"] / 1024:.2f}–{value["maximumKiB"] / 1024:.2f}]')
            deltas = [p['oursMinusOfficialKiB'][key] for p in pairs]
            delta = 'unavailable' if any(v is None for v in deltas) else f'{statistics.median(deltas) / 1024:+.2f}'
            lines.append(f'| {title} | {cells[0]} | {cells[1]} | {delta} |')
        lines += ['', 'Paired differences (MiB), median of each four-sample block, ours − official:']
        for pair in pairs:
            value = pair['oursMinusOfficialKiB']['pssKiB'] / 1024
            indices = ', '.join(str(i + 1) for i in pair['blockIndices'])
            lines.append(f'- Blocks {indices}: total PSS {value:+.2f}')
        lines += ['', '*Separate medians are not additive. Graphics summary and detailed categories may overlap. '
                  'PSS shared-mapping attribution is context-dependent; this is not a retained-heap measurement.*',
                  '*Screenshots establish crop agreement with supplied prepared references only, not semantic article proof.*']
        if self.args.measure_startup:
            startup = {}
            lines += ['', '## Prepared-article pixel-ready startup bounds', '',
                      '| App | Launch count | Median lower bound (s) | Median upper bound (s) |',
                      '|---|---:|---:|---:|']
            for alias in self.packages:
                observations = [b['startup'] for b in self.report['blocks'] if b['alias'] == alias]
                bounds = [item['boundsSeconds'] for item in observations]
                startup[alias] = {
                    'launchCount': len(bounds),
                    'medianLowerSeconds': statistics.median(item['lower'] for item in bounds),
                    'medianUpperSeconds': statistics.median(item['upper'] for item in bounds),
                }
                value = startup[alias]
                lines.append(f'| {alias} | {value["launchCount"]} | '
                             f'{value["medianLowerSeconds"]:.3f} | {value["medianUpperSeconds"]:.3f} |')
            self.report['summary']['startup'] = startup
            lines += ['', 'Bounds are [last mismatch acquisition start (or t0), first match acquisition end] '
                      'relative to host-monotonic t0 immediately before launch. Median lower and upper '
                      'endpoints bound the median of these launches; no midpoint estimate is used.',
                      'Actual PNG acquisitions include adb/polling overhead. These are not FPS, Android '
                      'frame-accurate timings, semantic article proof, or guaranteed interactivity. '
                      'am start -W logs are separate and are not user-ready timings.',
                      'Startup uses one observation per launch, not four PSS samples. '
                      'PSS settles for 30 seconds after admitted readiness and launch completion.']
        text = '\n'.join(lines) + '\n'
        # A generated results artifact, not repository documentation.
        (self.output / 'summary.md').write_text(text)
        return text

    def run(self):
        summary = None
        try:
            for index, code in enumerate(self.report['order']):
                self.block(index, 'ours' if code == 'A' else 'official')
            after = {alias: self.identity(alias) for alias in self.packages}
            self.report['installedIdentityAfter'] = after
            before = self.report['inventoryBefore']['packages']
            # A package dump can contain changing usage metadata. Compare only
            # installed APK bytes/paths and version identity, retaining dump hashes.
            if any(after[alias][key] != before[alias][key]
                   for alias in self.packages for key in ('apks', 'versions')):
                raise RuntimeError('Installed package identity changed during profiling')
            summary = self.summarize()
            self.report['status'] = 'complete'
        except BaseException as error:
            self.report.update({'status': 'rejected', 'failure': {'type': type(error).__name__, 'message': str(error)}})
            for block in self.report['blocks']:
                if block['status'] == 'running':
                    block['status'] = 'rejected'
                    block['ended'] = stamp()
                    if block.get('startup', {}).get('status') == 'running':
                        block['startup'].update({
                            'status': 'rejected', 'failure': self.report['failure'], 'ended': stamp(),
                        })
                    for sample in block['samples']:
                        if sample['status'] == 'running':
                            sample.update({'status': 'rejected', 'ended': stamp()})
        finally:
            self.cleanup = True
            if self.admitted:
                for args in [('shell', 'am', 'force-stop', pkg) for pkg in self.packages.values()] + [('shell', 'input', 'keyevent', 'KEYCODE_HOME')]:
                    entry = {'argv': list(args)}
                    try:
                        self.adb(*args, timeout=10)
                        entry['status'] = 'complete'
                    except BaseException as error:
                        entry.update({'status': 'failed', 'error': str(error)})
                    self.report['cleanup'].append(entry)
            else:
                self.report['cleanup'].append({'status': 'skipped', 'reason': 'Physical target never admitted; no device mutations performed'})
            if any(item['status'] == 'failed' for item in self.report['cleanup']):
                self.report['status'] = 'rejected'
                self.report.setdefault('failure', {'type': 'CleanupError', 'message': 'Could not stop apps and return Home'})
            self.report.update({'ended': stamp(), 'actualWallSeconds': time.monotonic() - self.started})
            self.persist()
            self.commands.close()
        if self.report['status'] == 'complete':
            print('\n' + summary, flush=True)
        else:
            print('REJECTED: ' + self.report['failure']['message'], file=sys.stderr, flush=True)
        print(f'Actual wall time: {self.report["actualWallSeconds"]:.1f}s\nJSON report: {self.output / "report.json"}', flush=True)
        return 0 if self.report['status'] == 'complete' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--adb', required=True, help='Explicit adb executable path/name')
    parser.add_argument('--serial', required=True, help='Exact physical serial; no device enumeration')
    parser.add_argument('--model', required=True, help='Exact ro.product.model')
    parser.add_argument('--output', required=True, type=Path, help='New exclusive directory; must not exist')
    parser.add_argument('--reference-ours', required=True, type=Path, help='Trusted prepared Animal article PNG')
    parser.add_argument('--reference-official', required=True, type=Path, help='Trusted prepared Animal article PNG')
    parser.add_argument('--ours', type=package, default='org.kiwix.workbench')
    parser.add_argument('--official', type=package, default='org.kiwix.kiwixmobile.standalone')
    parser.add_argument('--blocks', type=int, choices=(4, 8), default=4)
    parser.add_argument('--measure-startup', action='store_true',
                        help='Also measure bounded prepared-article pixel readiness before the 30s PSS settle')
    args = parser.parse_args()
    if args.ours == args.official:
        parser.error('Benchmark packages must be distinct')
    if any(not value.strip() or '\n' in value or '\r' in value for value in (args.adb, args.serial, args.model)):
        parser.error('adb, serial, and model must be explicit nonempty single-line values')
    references = {}
    for alias in ('ours', 'official'):
        path = getattr(args, 'reference_' + alias).resolve()
        data = path.read_bytes()
        image = Image.open(io.BytesIO(data))
        if image.format != 'PNG' or min(image.size) < 100:
            parser.error('References must be actual full-size PNG screenshots')
        references[alias] = {'path': path, 'bytes': data, 'image': image.convert('RGB')}
    args.output = args.output.absolute()
    args.output.mkdir(mode=0o700, parents=False, exist_ok=False)
    profiler = Profiler(args, references)
    def interrupted(signum, _frame):
        raise KeyboardInterrupt('Interrupted by signal ' + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    return profiler.run()


if __name__ == '__main__':
    sys.exit(main())
