#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Single-cold-session Android activity/PSS comparison (Python 3 + Pillow).

Prepare a verified reference and explicit UI plan for each app first. Run one
session with --adb PATH --serial SERIAL --model MODEL --alias ours|official
--plan PLAN.json --reference START.png --output NEW_DIRECTORY. Pair ours/official
sessions, then reverse their order. This does not install apps or grant access.

Plan JSON contract (no selectors or article identities are guessed):
{
  "schema": 1, "alias": "ours", "workload": "wiki",
  "archive": {"name": "observed archive", "sha256": "64 lowercase hex"},
  "environment": {"theme": "observed", "font": "observed"},
  "startIdentity": "operator-verified starting article and scroll",
  "prepare": [ACTION, ...],
  "return": [ACTION, ...],
  "cycles": [{"query": "observed query", "article": "semantic identity",
              "actions": [ACTION, ...]}, ... exactly 20]
}
Actions (exact keys only):
  {"op":"tap", "x":123, "y":456}
  {"op":"text", "text":"ASCII search words"}
  {"op":"key", "key":"BACK|ENTER|SEARCH|DEL|MOVE_END|MOVE_HOME"}
  {"op":"swipe", "x1":1, "y1":2, "x2":3, "y2":4, "ms":300}
  {"op":"dwell", "seconds":0.5}  # at most 2 seconds
  {"op":"ready", "reference":"relative-or-absolute.png",
   "identity":"operator-verified semantic article/query identity"}
Each cycle must end with ready, proving agreement with its operator-trusted
app-specific full screenshot article crop, not independent semantic identity.
prepare may be empty; return must be nonempty. All PNGs/plan bytes are frozen
before device access. Full standard profiler crop is used, including at all
checkpoints. No UI hierarchy, arbitrary shell, GC, cache clearing or restarts.
No pooling of the four adjacent samples as session replication. Pair separate
cold sessions externally to calculate ours-minus-official excess growth.
"""
import argparse
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import re
import signal
import statistics
import sys
import time

SOURCE = Path(__file__).resolve().with_name('profile-android-memory.py')
SOURCE_SHA = '8217a32ad87aaaf20c9ced8643d8033fa895302887b219e6316251ffa57235d5'
if hashlib.sha256(SOURCE.read_bytes()).hexdigest() != SOURCE_SHA:
    raise RuntimeError('Permanent profiler SHA mismatch; refusing import')
spec = importlib.util.spec_from_file_location('unchanged_memory_profiler', SOURCE)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def exact(obj, keys):
    if not isinstance(obj, dict) or set(obj) != set(keys):
        raise ValueError('Expected exact fields: ' + ', '.join(keys))


def integer(value, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ValueError('Integer outside admitted range')


def nonempty(value):
    if not isinstance(value, str) or not value.strip() or len(value) > 2048:
        raise ValueError('Expected nonempty bounded identity/text')


def png(path):
    data = path.read_bytes()
    image = base.Image.open(io.BytesIO(data))
    if image.format != 'PNG' or min(image.size) < 100:
        raise ValueError('Expected full-size trusted PNG')
    return {'path': path, 'bytes': data, 'image': image.convert('RGB')}


def freeze_plan(path, alias):
    raw = path.read_bytes()
    plan = json.loads(raw)
    exact(plan, ('schema', 'alias', 'workload', 'archive', 'environment',
                 'startIdentity', 'prepare', 'return', 'cycles'))
    if plan['schema'] != 1 or plan['alias'] != alias or plan['workload'] not in ('wiki', 'street'):
        raise ValueError('Plan schema/alias/workload mismatch')
    exact(plan['archive'], ('name', 'sha256'))
    nonempty(plan['archive']['name'])
    if not re.fullmatch('[0-9a-f]{64}', plan['archive']['sha256']):
        raise ValueError('Archive SHA required')
    exact(plan['environment'], ('theme', 'font'))
    for value in (*plan['environment'].values(), plan['startIdentity']):
        nonempty(value)
    refs = {}

    def actions(items, required=True):
        if not isinstance(items, list) or not (int(required) <= len(items) <= 40):
            raise ValueError('Action list must be bounded, nonempty when required')
        for a in items:
            if not isinstance(a, dict):
                raise ValueError('Action must be an object')
            op = a.get('op')
            fields = {'tap': ('x', 'y'), 'text': ('text',), 'key': ('key',),
                      'swipe': ('x1', 'y1', 'x2', 'y2', 'ms'),
                      'dwell': ('seconds',), 'ready': ('reference', 'identity')}
            if op not in fields:
                raise ValueError('Action not allowed')
            exact(a, ('op', *fields[op]))
            if op in ('tap', 'swipe'):
                for key in fields[op]:
                    integer(a[key], 1 if key == 'ms' else 0, 1500 if key == 'ms' else 10000)
            elif op == 'text':
                # Android input text travels through adb's remote shell. Reject
                # shell metacharacters and literal % escapes, not merely quote.
                if not isinstance(a['text'], str) or not re.fullmatch(r'[A-Za-z0-9 .,_-]{1,200}', a['text']):
                    raise ValueError('Text supports safe ASCII letters/numbers/space/.,_- only')
            elif op == 'key':
                if a['key'] not in ('BACK', 'ENTER', 'SEARCH', 'DEL', 'MOVE_END', 'MOVE_HOME'):
                    raise ValueError('Key not allowed')
            elif op == 'dwell':
                if type(a['seconds']) not in (int, float) or not 0 < a['seconds'] <= 2:
                    raise ValueError('Dwell must be >0 and <=2 seconds')
            else:
                nonempty(a['identity'])
                nonempty(a['reference'])
                refpath = (path.parent / a['reference']).resolve()
                refs[a['reference']] = png(refpath)

    actions(plan['prepare'], False)
    actions(plan['return'])
    if not isinstance(plan['cycles'], list) or len(plan['cycles']) != 20:
        raise ValueError('Exactly 20 ordered cycles required')
    for cycle in plan['cycles']:
        exact(cycle, ('query', 'article', 'actions'))
        nonempty(cycle['query'])
        nonempty(cycle['article'])
        actions(cycle['actions'])
        if cycle['actions'][-1]['op'] != 'ready' or cycle['actions'][-1]['identity'] != cycle['article']:
            raise ValueError('Every cycle must end with its semantic article ready reference')
    return raw, plan, refs


class ActivityProfiler(base.Profiler):
    def __init__(self, args, reference, raw, plan, action_refs):
        super().__init__(args, {args.alias: reference})
        self.plan = plan
        self.action_refs = action_refs
        self.sequence = 0
        (self.output / 'plan.json').write_bytes(raw)
        self.report.update({'schema': 2, 'experiment': 'single-cold-session-activity',
                            'helperSha256': base.digest(Path(__file__).read_bytes()),
                            'sourceProfilerSha256': SOURCE_SHA, 'order': [args.alias],
                            'planSha256': base.digest(raw), 'plan': plan,
                            'actions': [], 'completedCycles': 0,
                            'checkpoints': [0, 5, 10, 20],
                            'limitations': [
                                'One cold session, not four independent samples per checkpoint.',
                                'PNG agreement is not independent semantic identity proof; operator must verify references.',
                                'Archive/theme/font are operator attestations; installed APK identity is checked.',
                                'PSS is context-dependent, not retained heap; no forced GC.',
                                'Compare separate ours/official sessions externally for excess growth.']})
        self.report['actionReferences'] = {}
        for index, (name, ref) in enumerate(action_refs.items()):
            copy = f'action-reference-{index:03d}.png'
            (self.output / copy).write_bytes(ref['bytes'])
            self.report['actionReferences'][name] = {'copy': copy, 'sha256': base.digest(ref['bytes'])}
        self.persist()

    def ready(self, reference, identity, label):
        alias = self.args.alias
        previous = self.references[alias]
        frames = []
        receipt = {'identity': identity, 'frames': frames, 'started': base.stamp()}
        self.report.setdefault('readiness', []).append(receipt)
        deadline = min(time.monotonic() + 15, time.monotonic() + self.remaining())
        self.references[alias] = reference
        try:
            while True:
                frame = self.startup_frame(alias, f'{label}-ready-{len(frames):03d}', frames, deadline)
                if frame['matchesReference']:
                    receipt['foreground'] = self.foreground(alias, deadline)
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Readiness deadline exceeded')
                    receipt['ready'] = base.stamp()
                    return
                if time.monotonic() >= deadline:
                    raise TimeoutError('Reference not matched within 15 seconds')
                self.sleep(min(0.1, deadline - time.monotonic()))
        finally:
            self.references[alias] = previous
            self.persist()

    def actions(self, items, context):
        for action in items:
            self.sequence += 1
            entry = {'sequence': self.sequence, 'context': context, 'action': action, 'started': base.stamp()}
            self.report['actions'].append(entry)
            self.persist()
            self.foreground(self.args.alias)
            op = action['op']
            if op == 'tap':
                self.adb('shell', 'input', 'tap', str(action['x']), str(action['y']))
            elif op == 'text':
                self.adb('shell', 'input', 'text', action['text'].replace(' ', '%s'))
            elif op == 'key':
                self.adb('shell', 'input', 'keyevent', 'KEYCODE_' + action['key'])
            elif op == 'swipe':
                self.adb('shell', 'input', 'swipe', *(str(action[k]) for k in ('x1', 'y1', 'x2', 'y2', 'ms')))
            elif op == 'dwell':
                self.sleep(action['seconds'])
            else:
                self.ready(self.action_refs[action['reference']], action['identity'], f'action-{self.sequence:04d}')
            entry['ended'] = base.stamp()
            self.persist()

    def checkpoint(self, count):
        alias = self.args.alias
        b = {'index': len(self.report['blocks']), 'alias': alias, 'cycles': count,
             'status': 'running', 'samples': [], 'started': base.stamp()}
        previous = self.report['blocks'][-1] if self.report['blocks'] else None
        self.report['blocks'].append(b)
        self.ready(self.references[alias], self.plan['startIdentity'], f'checkpoint-{count}')
        b['settleStarted'] = base.stamp()
        self.sleep(30)
        self.screenshot(alias, f'checkpoint-{count}-before')
        identity = self.ownership(alias)
        b['initialOwnership'] = identity
        if previous:
            try:
                self.same_generation(previous['initialOwnership'], identity)
                b['generationChangedSincePrevious'] = False
            except RuntimeError:
                b['generationChangedSincePrevious'] = True
        else:
            b['generationChangedSincePrevious'] = None
        self.persist()
        for index in range(4):
            sample = {'index': index, 'status': 'running', 'started': base.stamp(), 'processes': {}}
            b['samples'].append(sample)
            self.persist()
            sample['foregroundBefore'] = self.foreground(alias)
            sample['before'] = self.ownership(alias)
            self.same_generation(identity, sample['before'])
            for pid, generation in identity['selected'].items():
                raw = self.adb('shell', 'dumpsys', 'meminfo', pid)
                path = self.output / f'checkpoint-{count}-sample-{index + 1}-pid-{pid}-meminfo.txt'
                path.write_text(raw)
                process = {'identity': generation, 'rawPath': path.name, 'rawSha256': base.digest(raw.encode())}
                sample['processes'][pid] = process
                self.persist()
                process['metrics'] = base.parse_meminfo(raw, pid)
            sample['after'] = self.ownership(alias)
            self.same_generation(identity, sample['after'])
            sample['foregroundAfter'] = self.foreground(alias)
            totals = {}
            for key in ('pssKiB', 'rssKiB', 'swapPssKiB', *list(base.METRICS)[4:]):
                values = [p['metrics'][key] for p in sample['processes'].values()]
                totals[key] = sum(values) if all(v is not None for v in values) else None
            totals['mainPssKiB'] = sample['processes'][identity['mainPid']]['metrics']['pssKiB']
            totals['rendererPssKiB'] = sum(sample['processes'][pid]['metrics']['pssKiB'] for pid in identity['rendererPids'])
            totals['otherPssKiB'] = totals['pssKiB'] - totals['mainPssKiB'] - totals['rendererPssKiB']
            sample.update({'metrics': totals, 'status': 'complete', 'ended': base.stamp()})
            self.persist()
            if index < 3:
                self.sleep(2)
        self.screenshot(alias, f'checkpoint-{count}-after')
        self.same_generation(identity, self.ownership(alias))
        b.update({'status': 'complete', 'ended': base.stamp()})
        self.persist()

    def session(self):
        self.guard()
        self.report['inventoryBefore'] = self.inventory()
        self.persist()
        for pkg in self.packages.values():
            self.adb('shell', 'am', 'force-stop', pkg)
        pkg = self.packages[self.args.alias]
        lines = self.adb('shell', 'cmd', 'package', 'resolve-activity', '--brief', pkg).strip().splitlines()
        component = lines[-1] if lines else ''
        if not re.fullmatch(re.escape(pkg) + r'/[A-Za-z0-9_.$]+', component):
            raise RuntimeError('Unexpected launch component')
        launch = self.adb('shell', 'am', 'start', '-W', '-n', component)
        self.report['launch'] = launch
        if not re.search(r'^Status:\s*ok\s*$', launch, re.M) or re.search(r'Error:|Exception', launch):
            raise RuntimeError('Launch failed')
        self.actions(self.plan['prepare'], 'prepare')
        self.checkpoint(0)
        for index, cycle in enumerate(self.plan['cycles'], 1):
            self.actions(cycle['actions'], {'cycle': index, 'query': cycle['query'], 'article': cycle['article']})
            self.report['completedCycles'] = index
            self.persist()
            if index in (5, 10, 20):
                self.actions(self.plan['return'], {'returnForCheckpoint': index})
                self.checkpoint(index)
        after = {alias: self.identity(alias) for alias in self.packages}
        self.report['installedIdentityAfter'] = after
        if any(after[a][k] != self.report['inventoryBefore']['packages'][a][k]
               for a in self.packages for k in ('apks', 'versions')):
            raise RuntimeError('Installed identity changed')
        blocks = self.report['blocks']
        if self.report['completedCycles'] != 20 or [b['cycles'] for b in blocks] != [0, 5, 10, 20] or any(b['status'] != 'complete' for b in blocks):
            raise RuntimeError('Incomplete 20-cycle session')
        medians = [{k: statistics.median(s['metrics'][k] for s in b['samples'])
                    if all(s['metrics'][k] is not None for s in b['samples']) else None
                    for k in base.METRICS} for b in blocks]
        self.report['summary'] = [
            {'cycles': b['cycles'], 'medianKiB': m,
             'growthFromOwnZeroKiB': {k: m[k] - medians[0][k] if m[k] is not None and medians[0][k] is not None else None for k in m}}
            for b, m in zip(blocks, medians)]

    def run(self):
        try:
            self.session()
            self.report['status'] = 'complete'
        except BaseException as error:
            self.report.update({'status': 'rejected', 'failure': {'type': type(error).__name__, 'message': str(error)}})
            for block in self.report['blocks']:
                if block['status'] == 'running':
                    block['status'] = 'rejected'
                for sample in block['samples']:
                    if sample['status'] == 'running':
                        sample['status'] = 'rejected'
        finally:
            # Ignore repeated terminal interrupts only during reserved cleanup.
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            self.cleanup = True
            if self.admitted:
                commands = [('shell', 'am', 'force-stop', p) for p in self.packages.values()]
                commands.append(('shell', 'input', 'keyevent', 'KEYCODE_HOME'))
                for command in commands:
                    receipt = {'argv': list(command)}
                    try:
                        self.adb(*command, timeout=10)
                        receipt['status'] = 'complete'
                    except BaseException as error:
                        receipt.update({'status': 'failed', 'error': str(error)})
                        self.report['status'] = 'rejected'
                        self.report.setdefault('failure', {'type': 'CleanupError', 'message': str(error)})
                    self.report['cleanup'].append(receipt)
            else:
                self.report['cleanup'].append({'status': 'skipped', 'reason': 'Target never admitted; no mutations'})
            self.report.update({'ended': base.stamp(), 'actualWallSeconds': time.monotonic() - self.started})
            self.persist()
            self.commands.close()
        return 0 if self.report['status'] == 'complete' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--alias', required=True, choices=('ours', 'official'))
    parser.add_argument('--plan', required=True, type=Path)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--adb', required=True)
    parser.add_argument('--serial', required=True, help='Exact physical serial; no enumeration')
    parser.add_argument('--model', required=True, help='Exact ro.product.model')
    args = parser.parse_args()
    if any(not value.strip() or '\n' in value or '\r' in value
           for value in (args.adb, args.serial, args.model)):
        parser.error('adb, serial and model must be explicit nonempty single-line values')
    args.ours = 'org.kiwix.workbench'
    args.official = 'org.kiwix.kiwixmobile.standalone'
    args.blocks = 4
    args.measure_startup = False
    raw, plan, refs = freeze_plan(args.plan.resolve(), args.alias)
    reference = png(args.reference.resolve())
    if any(r['image'].size != reference['image'].size for r in refs.values()):
        parser.error('All trusted references must have identical screen dimensions')
    args.output = args.output.absolute()
    args.output.mkdir(mode=0o700, parents=False, exist_ok=False)
    profiler = ActivityProfiler(args, reference, raw, plan, refs)
    def interrupted(signum, _frame):
        raise KeyboardInterrupt('Signal ' + str(signum))
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    return profiler.run()


if __name__ == '__main__':
    sys.exit(main())