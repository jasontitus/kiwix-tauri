# SPDX-License-Identifier: GPL-3.0-or-later
"""Off-device authority, frozen-byte and real owned-process cancellation tests.

Hardware owner runs: python3 -m unittest discover -s scripts -p test_android_pair.py
No test opens adb, changes product data or certifies physical topology/content.
"""
import copy
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SPEC = importlib.util.spec_from_file_location('android_pair', Path(__file__).with_name('android-pair.py'))
pair = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pair)
ARCHIVE = '02b6d195-2785-f9cc-9c03-29d44dfbaa36'
PUBLISHER = '342cb0c8-2b7c-41d0-aea4-d8b479f9622a'
INSTALLATION = '07a60905-bf91-42ce-a8c8-60ae069288d9'
JOB = '0de6d420-fdcb-4dd2-9779-10fe50c9379c'
# The pairing tool runs on a macOS/Linux host. Process groups, POSIX signals
# and no-follow opens do not exist on Windows, so those tests are skipped there.
posix_host = unittest.skipUnless(os.name == 'posix', 'requires a POSIX host (process groups, signals, O_NOFOLLOW)')


class PairAuthorityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        for name in ('source.zim', 'sender.json', 'receiver.json'):
            (self.root / name).write_bytes(b'fixture')
        self.config = dict(schema=1, topology='sender-hotspot', sender_device='S1TESTSER02',
                           receiver_device='R5TESTSER01', sender_prepared=str(self.root / 'sender.json'),
                           receiver_prepared=str(self.root / 'receiver.json'), source=str(self.root / 'source.zim'),
                           archive_id=ARCHIVE, file_name=ARCHIVE + '.zim', title='Reader fixture',
                           expected_text='Image article', peer='Fire', window_seconds=600)
        self.source = dict(archive_id=ARCHIVE, file_name=ARCHIVE + '.zim', title='Reader fixture',
                           peer='Fire', size=266619, sha256='a' * 64)

    def test_physical_serials_topology_and_independent_authority(self):
        pair.validate_config(self.config)
        for changes in ({'receiver_device': self.config['sender_device']},
                        {'topology': 'routerless'}, {'topology': 'lan'},
                        {'receiver_prepared': self.config['sender_prepared']},
                        {'window_seconds': 601}, {'window_seconds': True},
                        {'sender_device': 'Fire; adb shell'}, {'schema': True}):
            with self.subTest(changes=changes), self.assertRaises(RuntimeError):
                pair.validate_config(dict(self.config, **changes))

    def test_managed_source_full_hash_not_metadata_or_provider(self):
        value = dict(source_kind='managed', managed=True, source_bytes_verified=True,
                     receivedhash=self.source['sha256'], source_sha256=self.source['sha256'],
                     receivedsize=self.source['size'], installation_id=INSTALLATION)
        pair.validate_source(value, self.source)
        for changes in ({'source_kind': 'provider'}, {'source_bytes_verified': False},
                        {'source_sha256': 'b' * 64}, {'receivedsize': 266618},
                        {'installation_id': ''}, {'managed': False}):
            with self.subTest(changes=changes), self.assertRaises(RuntimeError):
                pair.validate_source(dict(value, **changes), self.source)

    def receiver(self):
        job = dict(id=JOB, journal_key=JOB, url='kiwix-library://public/' + '3' * 32 + '/' + PUBLISHER + '/' + ARCHIVE,
                   transport='quic-library', error=None, importError=None, status='complete',
                   fileName=self.source['file_name'], downloaded=self.source['size'], total=self.source['size'],
                   publication={'size': self.source['size'], 'sha256': self.source['sha256']},
                   destination='/private/managed/book.zim', consumed_acquisition=True,
                   installation_id=INSTALLATION, matches_independent_received_bytes=True)
        return dict(installation_id=INSTALLATION, managed_path=job['destination'],
                    received_journal=dict(status='matched', match_count=1, matched_job=job, new_rows=[job]))

    def test_consumed_receive_job_cannot_rebind_source_or_old_journal(self):
        publication = dict(publisher_id=PUBLISHER, library_generation=7)
        value = self.receiver()
        pair.validate_received_job(value, self.source, publication)
        for mutate in (
                lambda v: v['received_journal'].update(new_rows=[]),
                lambda v: v['received_journal'].update(match_count=2),
                lambda v: v['received_journal']['matched_job'].update(consumed_acquisition=False),
                lambda v: v['received_journal']['matched_job'].update(installation_id=JOB),
                lambda v: v['received_journal']['matched_job'].update(destination='/another/book.zim'),
                lambda v: v['received_journal']['matched_job'].update(url=v['received_journal']['matched_job']['url'].replace(PUBLISHER, JOB)),
                lambda v: v['received_journal']['matched_job'].update(publication={'size': self.source['size'], 'sha256': 'b' * 64})):
            changed = copy.deepcopy(value)
            mutate(changed)
            with self.assertRaises(RuntimeError):
                pair.validate_received_job(changed, self.source, publication)
        # The source session is intentionally unavailable. The receiver's actual
        # session is retained, never replaced by a controller-invented nonce.
        pair.validate_received_job(value, self.source, publication)
        self.assertIn('/' + '3' * 32 + '/', value['received_journal']['matched_job']['url'])

    def test_no_ap_is_not_joined_receiver_station_off(self):
        pair.validate_baseline(dict(sta_connected=False, sta_disabled=False,
                                    no_ap_verified=True, current_ap_verified=False))
        with self.assertRaises(RuntimeError):
            pair.validate_baseline(dict(sta_connected=True, no_ap_verified=True, current_ap_verified=False))
        with self.assertRaises(RuntimeError):
            pair.validate_baseline(dict(sta_connected=False, no_ap_verified=False, current_ap_verified=True))

    def test_real_legacy_parser_output_admits_only_api28_current_ap(self):
        spec = importlib.util.spec_from_file_location('pair_nearby', Path(__file__).with_name('android-nearby.py'))
        nearby = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(nearby)
        dump = ('Wi-Fi is enabled\nDump of WifiStateMachinePrime\nNumActiveModeManagers: 1\n'
                '--Dump of SoftApManager--\ncurrent StateMachine mode: StartedState\n'
                'mApInterfaceName: ap0\nmIfaceIsUp: true\nWifiStateMachine:\n'
                'mWifiInfo SSID: <unknown ssid>, BSSID: <none>, Supplicant state: INACTIVE\n'
                'mNetworkInfo [type: WIFI[], state: DISCONNECTED/IDLE, reason: (unspecified)]\n')
        peer = pair.AndroidPeer.__new__(pair.AndroidPeer)
        peer.role, peer.source = 'sender', self.source
        peer.identity = dict(target_apk_sha256='a' * 64, certificate_sha256='b' * 64)
        peer.instrumentation = {'publisher_id': PUBLISHER}
        value = dict(status='ready', publisher_ready=True, product_owned_ap_at_publish=True,
                     source_kind='managed', managed=True, source_bytes_verified=True,
                     receivedhash=self.source['sha256'], source_sha256=self.source['sha256'],
                     receivedsize=self.source['size'], installation_id=INSTALLATION,
                     publication=dict(publisher_id=PUBLISHER, library_generation=7),
                     hotspot_network_sha256='c' * 64,
                     installed_identity=dict(package=pair.TARGET, debuggable=False, apk_sha256='a' * 64,
                                             certificate_sha256='b' * 64, android_os={'api': 28}),
                     route_radio=nearby.parse_legacy_radio(dump, 'publishing'))
        self.assertEqual(peer.admit_publisher(value)['publisher_id'], PUBLISHER)
        value['installed_identity']['android_os']['api'] = 36
        with self.assertRaisesRegex(RuntimeError, 'frequency evidence'):
            peer.admit_publisher(value)
        value['installed_identity']['android_os']['api'] = 28
        baseline = dump.replace('SoftApManager', 'ClientModeManager').replace('mApInterfaceName', 'mClientInterfaceName')
        value['route_radio'] = nearby.parse_legacy_radio(baseline + 'Historical SoftApManager frequency=2462\n', 'publishing')
        with self.assertRaisesRegex(RuntimeError, 'sender-AP proof'):
            peer.admit_publisher(value)
        with self.assertRaises(ValueError):
            nearby.parse_legacy_radio(dump.replace('StartedState', 'StartingState'), 'publishing')
        with self.assertRaises(ValueError):
            nearby.parse_legacy_radio('history: ' + dump.replace('\n', '\nhistory: '), 'publishing')

    @posix_host
    def test_sdk_parent_link_retarget_is_rejected_before_tool_execution(self):
        sdk = self.root / 'sdk'
        build = sdk / 'build-tools/36.0.0'
        platform = sdk / 'platforms/android-36'
        adb = sdk / 'platform-tools'
        for directory in (build, platform, adb):
            directory.mkdir(parents=True)
        tools = {name: str(build / name) for name in ('aapt2', 'd8', 'zipalign', 'apksigner')}
        tools.update(adb=str(adb / 'adb'), android_jar=str(platform / 'android.jar'))
        for path in tools.values():
            Path(path).write_bytes(b'reviewed runtime')
        pair.validate_tool_paths(tools)
        saved = self.root / 'reviewed-build-tools'
        build.rename(saved)
        replacement = self.root / 'replacement-build-tools'
        replacement.mkdir()
        for name in ('aapt2', 'd8', 'zipalign', 'apksigner'):
            (replacement / name).write_bytes(b'unreviewed executable')
        build.symlink_to(replacement, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, 'Canonical input required'):
            pair.validate_tool_paths(tools)
        # The old resolved target still exists: checking it alone is insufficient.
        self.assertEqual(pair.pin(saved / 'apksigner')['size'], len(b'reviewed runtime'))

    def test_route_gate_refuses_pre_instrumentation_and_unknown_sender_network(self):
        peer = pair.AndroidPeer.__new__(pair.AndroidPeer)
        peer.role, peer.gate, peer.instrumentation = 'receiver', self.root / 'gate.json', None
        peer.binding = dict(schema=1, nonce='1' * 32, topology='sender-hotspot',
                            device='R5TESTSER01', mode='receive', deadline=12345)
        with self.assertRaisesRegex(RuntimeError, 'before instrumentation'):
            peer.release_route('a' * 64)
        self.assertFalse(peer.gate.exists())
        peer.instrumentation = {'status': 'ready'}
        with self.assertRaisesRegex(RuntimeError, 'actual source hotspot'):
            peer.release_route()
        self.assertFalse(peer.gate.exists())
        peer.release_route('a' * 64)
        with self.assertRaises(FileExistsError):
            peer.release_route('b' * 64)
        self.assertEqual(json.loads(peer.gate.read_text())['sender_network_sha256'], 'a' * 64)

    @posix_host
    def test_source_uuid_comes_from_actual_header_not_config_label(self):
        import uuid
        path = Path(self.config['source'])
        path.write_bytes(b'ZIM\x04' + b'\0' * 4 + uuid.UUID(ARCHIVE).bytes + b'actual bytes')
        actual = pair.pin(path)
        source = dict(self.source, size=actual['size'], sha256=actual['sha256'])
        pair.validate_fixture(self.config, source)
        with self.assertRaisesRegex(RuntimeError, 'header/archive identity'):
            pair.validate_fixture(dict(self.config, archive_id=JOB), source)
        path.write_bytes(path.read_bytes() + b'changed')
        with self.assertRaisesRegex(RuntimeError, 'source bytes changed'):
            pair.validate_fixture(self.config, source)

    @posix_host
    def test_runtime_tree_detects_added_import_and_changed_link_target(self):
        runtime = self.root / 'runtime'
        runtime.mkdir()
        (runtime / 'module.py').write_text('VALUE = 1\n')
        frozen = pair.tree_pin(runtime, python=True)
        (runtime / '__pycache__').mkdir()
        (runtime / '__pycache__/module.pyc').write_bytes(b'cached imports are executable even under -B')
        self.assertNotEqual(frozen, pair.tree_pin(runtime, python=True))
        (runtime / 'unexpected.py').write_text('raise RuntimeError("drift")\n')
        self.assertNotEqual(frozen, pair.tree_pin(runtime, python=True))
        (runtime / 'unexpected.py').unlink()
        target = self.root / 'outside.py'
        target.write_text('VALUE = 2\n')
        (runtime / 'linked.py').symlink_to(target)
        frozen = pair.tree_pin(runtime, python=True)
        target.write_text('VALUE = 3\n')
        self.assertNotEqual(frozen, pair.tree_pin(runtime, python=True))

    @posix_host
    def test_frozen_drift_is_rejected_before_any_helper_import_or_spawn(self):
        dependency = self.root / 'dependency.py'
        dependency.write_text('VALUE = "reviewed"\n')
        config_path = self.root / 'config.json'
        config_path.write_text(json.dumps(self.config))
        value = dict(schema=1, status='ready', config=self.config,
                     python={'executable': str(Path(sys.executable).resolve()), 'version': sys.version},
                     inputs=[pair.pin(dependency)], runtime_trees=[],
                     config_file={'path': str(config_path), 'sha256': pair.pin(config_path)['sha256']})
        prepared = self.root / 'prepared.json'
        prepared.write_text(json.dumps(value))
        digest = pair.pin(prepared)['sha256']
        dependency.write_text('raise RuntimeError("must not be imported")\n')
        with self.assertRaisesRegex(RuntimeError, 'Frozen input changed'):
            pair.load_frozen(prepared, digest, self.config['sender_device'], self.config['receiver_device'])
        prepared.write_text(prepared.read_text() + '\n')
        with self.assertRaisesRegex(RuntimeError, 'preparation bytes changed'):
            pair.load_frozen(prepared, digest, self.config['sender_device'], self.config['receiver_device'])

    @posix_host
    def test_missing_timing_dependency_is_rejected_before_runtime_imports(self):
        value = dict(schema=1, status='ready', config=self.config,
                     python={'executable': str(Path(sys.executable).resolve()), 'version': sys.version},
                     inputs=[], runtime_trees=[])
        prepared = self.root / 'prepared.json'
        prepared.write_text(json.dumps(value))
        with self.assertRaisesRegex(RuntimeError, 'timing dependency missing'):
            pair.load_frozen(prepared, pair.pin(prepared)['sha256'],
                             self.config['sender_device'], self.config['receiver_device'])

    def test_receipt_cannot_inherit_other_device_crash_or_nonce(self):
        peer = pair.AndroidPeer.__new__(pair.AndroidPeer)
        peer.role, peer.output = 'receiver', self.root
        peer.binding = dict(device='R5TESTSER01', nonce='1' * 32, topology='sender-hotspot',
                            reviewed_crash_sha256='6' * 64)
        receipt = dict(peer.binding, cleanup_complete=True)
        path = self.root / 'result.json'
        path.write_text(json.dumps(receipt))
        self.assertEqual(peer.receipt('result.json'), receipt)
        for changes in ({'device': 'S1TESTSER02'}, {'nonce': '2' * 32},
                        {'reviewed_crash_sha256': ''}, {'topology': 'routerless'}):
            path.write_text(json.dumps(dict(receipt, **changes)))
            with self.assertRaises(RuntimeError):
                peer.receipt('result.json')


@posix_host
class OwnedCancellationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def child(self, role='receiver'):
        # An actual child with kernel signal disposition; no mocked Popen/wait.
        ready = self.root / ('ready-' + role)
        script = ('import pathlib,signal,time,sys\n'
                  'signal.signal(signal.SIGINT, lambda *_: sys.exit(23))\n'
                  'signal.signal(signal.SIGTERM, lambda *_: sys.exit(24))\n'
                  'pathlib.Path(sys.argv[1]).write_text("ready")\n'
                  'while True: time.sleep(.05)\n')
        process = subprocess.Popen([sys.executable, '-I', '-S', '-B', '-c', script, str(ready)],
                                   start_new_session=True)
        def finish():
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)
        self.addCleanup(finish)
        end = time.monotonic() + 5
        while not ready.exists():
            self.assertIsNone(process.poll())
            self.assertLess(time.monotonic(), end)
            time.sleep(.01)
        peer = pair.AndroidPeer.__new__(pair.AndroidPeer)
        peer.process, peer.role, peer.output = process, role, self.root / role
        peer.output.mkdir()
        peer.binding = dict(device=role, nonce='1' * 32, topology='sender-hotspot')
        peer.window = pair.Window(self.root, 120)
        peer.log = (self.root / (role + '.log')).open('wb')
        self.addCleanup(peer.log.close)
        return peer

    def test_failure_cancels_exact_owned_children_and_reaps_without_faking_release(self):
        sender, receiver = self.child('sender'), self.child('receiver')
        sender.request_stop()
        receiver.request_stop()
        for peer in (sender, receiver):
            with self.assertRaisesRegex(RuntimeError, 'cleanup unconfirmed'):
                peer.reap()
            self.assertEqual(peer.process.returncode, 23)
            self.assertTrue(peer.log.closed)

    def test_deliberate_stop_and_failure_cancel_have_distinct_kernel_signals(self):
        sender = self.child('sender')
        sender.request_stop(successful=True)
        sender.process.wait(timeout=5)
        self.assertEqual(sender.process.returncode, 24)
        # Exit alone never certifies AP or admitted-route cleanup.
        with self.assertRaisesRegex(RuntimeError, 'cleanup unconfirmed'):
            sender.reap()

    def test_causal_child_failure_survives_generic_readiness_boundary(self):
        peer = self.child()
        peer.request_stop()
        peer.process.wait(timeout=5)
        error = dict(peer.binding, error='native retained socket route rejected')
        (peer.output / 'result.json').write_text(json.dumps(error))
        with self.assertRaisesRegex(RuntimeError, 'native retained socket route rejected'):
            pair.await_receipt(peer, 'ready.json', [peer], peer.window)

    def test_cleanup_does_not_equate_no_ap_or_exit_with_consumer_retirement(self):
        peer = self.child()
        peer.request_stop()
        peer.process.wait(timeout=5)
        peer.identity = dict(target_apk_sha256='a' * 64, certificate_sha256='b' * 64)
        value = dict(cleanup_complete=True, final_radio={'no_ap_verified': True},
                     installed_identity=dict(package=pair.TARGET, debuggable=False,
                                             apk_sha256='a' * 64, certificate_sha256='b' * 64))
        with self.assertRaisesRegex(RuntimeError, 'retirement unconfirmed'):
            peer.cleanup_proof(value)
        value.update(receiver_route_started=True, receiver_join_retired=True, receiver_consumer_retired=True)
        self.assertEqual(peer.cleanup_proof(value), value)
        value.update(receiver_route_started=False, receiver_join_retired=False, receiver_consumer_retired=False)
        with self.assertRaisesRegex(RuntimeError, 'disconnected station proof'):
            peer.cleanup_proof(value)
        value['final_radio']['sta_connected'] = False
        self.assertEqual(peer.cleanup_proof(value), value)
        value['final_radio']['no_ap_verified'] = False
        with self.assertRaisesRegex(RuntimeError, 'no-AP cleanup proof'):
            peer.cleanup_proof(value)


if __name__ == '__main__':
    unittest.main()
