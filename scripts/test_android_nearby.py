# SPDX-License-Identifier: GPL-3.0-or-later
"""Host-only topology/crash identity regressions; never contacts a device."""
import importlib.util
import argparse
import base64
import hashlib
import json
from pathlib import Path
import tempfile
import subprocess
import time
import signal
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("android_nearby", Path(__file__).with_name("android-nearby.py"))
nearby = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nearby)


class InspectorMotionTests(unittest.TestCase):
    # Execute the real inspector against a minimal read-only DOM fixture. This
    # covers admission semantics, not Android coordinates or physical UI proof.
    def inspect(self, *, ancestor=None, control=None, covered=False, missing_api=False):
        fixture = r"""
const fs = require("node:fs"), vm = require("node:vm");
const scenario = JSON.parse(process.argv[1]);
const bounds = {left: 16, top: 600, right: 160, bottom: 640, width: 144, height: 40};
const ancestor = {parentElement: null, getBoundingClientRect: () => ({left:0,top:0,right:384,bottom:751,width:384,height:751})};
const control = {parentElement: ancestor, textContent: "Nearby", disabled: false, value: "",
  getAttribute: () => null, getBoundingClientRect: () => bounds, contains: (other) => other === control};
if (!scenario.missing_api) {
  ancestor.getAnimations = () => scenario.ancestor ? [scenario.ancestor] : [];
  control.getAnimations = () => scenario.control ? [scenario.control] : [];
}
const document = {
  querySelectorAll: (selector) => selector === "nav.main-nav button" ? [control] : [],
  elementFromPoint: () => scenario.covered ? {} : control,
};
const inspect = vm.runInNewContext(fs.readFileSync(process.argv[2], "utf8"), {
  document, innerWidth: 384, innerHeight: 751,
  getComputedStyle: () => ({visibility:"visible",display:"block",overflowY:"visible",overflowX:"visible"}),
});
process.stdout.write(JSON.stringify(inspect({op:"nearby"})));
"""
        scenario = dict(ancestor=ancestor, control=control, covered=covered, missing_api=missing_api)
        result = subprocess.run(["node", "-e", fixture, json.dumps(scenario), str(nearby.SOURCE / "inspect.js")],
                                capture_output=True, text=True, check=True, timeout=10)
        return json.loads(result.stdout)

    def test_running_sheet_ancestor_is_not_tappable_even_when_current_hit_test_matches(self):
        row = self.inspect(ancestor=dict(pending=False, playState="running"))
        self.assertEqual(row, {"pending": "Waiting target motion to settle"})

    def test_pending_control_animation_does_not_authorize_a_tap(self):
        row = self.inspect(control=dict(pending=True, playState="idle"))
        self.assertNotIn("ready", row)
        self.assertEqual(row["pending"], "Waiting target motion to settle")

    def test_finished_animation_admits_current_visible_hit_target(self):
        row = self.inspect(ancestor=dict(pending=False, playState="finished"))
        self.assertTrue(row["ready"])
        self.assertTrue(row["tappable"])
        self.assertEqual(row["name"], "Nearby")

    def test_covered_stable_control_is_not_tappable(self):
        self.assertFalse(self.inspect(covered=True)["tappable"])

    def test_missing_animation_inspection_has_no_unsafe_fallback(self):
        row = self.inspect(missing_api=True)
        self.assertIn("error", row)
        self.assertNotIn("ready", row)


class SenderHotspotRadioTests(unittest.TestCase):
    LEGACY = """Wi-Fi is enabled
Dump of WifiStateMachinePrime
Current wifi mode: ClientModeActiveState
NumActiveModeManagers: 1
--Dump of ClientModeManager--
current StateMachine mode: StartedState
mClientInterfaceName: wlan0
mIfaceIsUp: true

WifiStateMachine:
rec[0]: time=11-09 processed=ConnectedState org=ConnectedState
curState=DisconnectedState
mWifiInfo SSID: <unknown ssid>, BSSID: <none>, Supplicant state: INACTIVE, Frequency: -1MHz
mNetworkInfo [type: WIFI[], state: DISCONNECTED/IDLE, reason: (unspecified)]
"""

    def test_legacy_location_requires_current_framework_boolean_not_secure_setting_or_log(self):
        evidence = dict(schema=1, android_api=28, source="LocationManager.isLocationEnabled",
                        location_services_enabled=True)
        nearby.validate_legacy_location(evidence)
        for field, value in (("location_services_enabled", False), ("location_services_enabled", None),
                             ("location_services_enabled", "true"), ("location_services_enabled", 1),
                             ("source", "settings secure location_mode"), ("source", "historical WiFiLog"),
                             ("schema", True), ("android_api", 29)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                nearby.validate_legacy_location(dict(evidence, **{field: value}))
        for unknown in (None, {}, {"location_mode": None}, {"enabledProviders": ["fused", "passive"]}):
            with self.subTest(unknown=unknown), self.assertRaises(ValueError):
                nearby.validate_legacy_location(unknown)

    def test_legacy_enabled_disconnected_baseline_not_disabled_or_current_ap(self):
        row = nearby.parse_legacy_radio(self.LEGACY + "Historical SoftApManager frequency=2462\n", "baseline")
        self.assertTrue(nearby.sender_hotspot_initial(row))
        self.assertFalse(row["sta_disabled"])
        self.assertFalse(nearby.sender_hotspot_route(row, "publish"))

    def test_only_current_started_softap_manager_admits_legacy_ap(self):
        dump = self.LEGACY.replace("NumActiveModeManagers: 1", "NumActiveModeManagers: 2").replace(
            "\nWifiStateMachine:", "\n--Dump of SoftApManager--\ncurrent StateMachine mode: StartedState\n"
            "mApInterfaceName: ap0\nmIfaceIsUp: true\n\nWifiStateMachine:")
        row = nearby.parse_legacy_radio(dump, "publishing")
        self.assertTrue(nearby.sender_hotspot_route(row, "publish"))
        self.assertFalse(nearby.sender_hotspot_initial(row))
        for broken in (dump.replace("mIfaceIsUp: true", "mIfaceIsUp: false"),
                       dump.replace("StartedState", "StartingState"),
                       dump.replace("NumActiveModeManagers: 2", "NumActiveModeManagers: 1"),
                       dump.replace("SoftApManager", "UnknownManager")):
            with self.subTest(broken=broken), self.assertRaises(ValueError):
                nearby.parse_legacy_radio(broken, "publishing")

    def test_historical_manager_or_unknown_station_never_admits(self):
        for dump in ("rec[0]: " + self.LEGACY.replace("\n", "\nrec[0]: "),
                     self.LEGACY.replace("mNetworkInfo ", "previous mNetworkInfo "),
                     self.LEGACY.replace("INACTIVE", "COMPLETED"), "DUMP TIMEOUT\n"):
            with self.subTest(dump=dump), self.assertRaises(ValueError):
                nearby.parse_legacy_radio(dump, "baseline")

    def test_joined_receiver_is_station_not_reverse_ap(self):
        station = nearby.parse_station("Wi-fi is enabled\nWifiInfo: BSSID: a0:b1:c2:d3:e4:f5, "
                                       "IP: /192.168.49.2, Supplicant state: COMPLETED")
        row = nearby.parse_radio(station, "mCurrentSoftApInfoMap {}\n", "catalog")
        self.assertTrue(nearby.sender_hotspot_route(row, "receive"))
        self.assertFalse(nearby.sender_hotspot_initial(row))
        self.assertFalse(nearby.sender_hotspot_route(dict(row, current_ap_verified=True, no_ap_verified=False), "receive"))
        self.assertFalse(nearby.sender_hotspot_route(dict(row, sta_disabled=True), "receive"))


class PairGateAndCancellationTests(unittest.TestCase):
    def test_exact_gate_binds_device_role_nonce_deadline_and_sender_network(self):
        args = argparse.Namespace(mode="receive", nonce="a" * 32, topology="sender-hotspot",
                                  device="receiver", deadline=12345)
        gate = dict(schema=1, **vars(args), sender_network_sha256="b" * 64)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "gate.json"
            path.write_text(json.dumps(gate))
            self.assertEqual(nearby.load_route_gate(path, args), gate)
            for key, value in (("schema", True), ("nonce", "c" * 32), ("device", "sender"),
                               ("mode", "publish"), ("deadline", 12346), ("topology", "routerless"),
                               ("sender_network_sha256", "unknown"), ("extra", True)):
                with self.subTest(key=key):
                    path.write_text(json.dumps(dict(gate, **{key: value})))
                    with self.assertRaises(ValueError):
                        nearby.load_route_gate(path, args)
            del gate["sender_network_sha256"]
            path.write_text(json.dumps(gate))
            with self.assertRaises(ValueError):
                nearby.load_route_gate(path, args)

    def test_only_post_ready_sigterm_publisher_can_deliberately_stop(self):
        valid = dict(graceful=True, mode="publish", ready=True, signum=signal.SIGTERM,
                     host_error=None, cancel_sent=False)
        self.assertTrue(nearby.graceful_publish_stop(**valid))
        for key, value in (("graceful", False), ("mode", "receive"), ("ready", False),
                           ("signum", signal.SIGINT), ("host_error", "real failure"), ("cancel_sent", True)):
            with self.subTest(key=key):
                self.assertFalse(nearby.graceful_publish_stop(**dict(valid, **{key: value})))


class CurrentRadioTests(unittest.TestCase):
    def parse(self, dump):
        return nearby.parse_radio({"sta_disabled": True, "sta_connected": False}, dump, "test")

    def test_samsung_empty_current_maps_admit_fresh_routerless_start(self):
        sample = self.parse("NumActiveModeManagers: 1\nmCurrentSoftApInfoMap {}\nSoftApManager:\nmCurrentSoftApInfoMap {}\n")
        self.assertTrue(sample["no_ap_verified"])
        self.assertFalse(sample["current_ap_verified"])

    def test_current_ap_frequency_blocks_no_ap_regardless_of_field_separator(self):
        for separator in (" ", ": ", "="):
            with self.subTest(separator=separator):
                sample = self.parse("mCurrentSoftApInfoMap" + separator + "{swlan0=SoftApInfo{frequency=2462, bandwidth=2}}\n")
                self.assertTrue(sample["current_ap_verified"])
                self.assertFalse(sample["no_ap_verified"])
                self.assertEqual(sample["ap_frequency_mhz"], [2462])

    def test_historical_frequency_cannot_admit_current_ap(self):
        sample = self.parse("11-07 15:00:00 mCurrentSoftApInfoMap {frequency=2462}\n")
        self.assertFalse(sample["current_ap_verified"])
        self.assertFalse(sample["no_ap_verified"])

    def test_empty_or_unrecognized_dump_does_not_prove_no_ap(self):
        for dump in ("", "DUMP TIMEOUT\n", "mCurrentSoftApInfoMap UNKNOWN\n"):
            with self.subTest(dump=dump):
                self.assertFalse(self.parse(dump)["no_ap_verified"])

    def test_active_manager_or_starting_state_overrules_empty_current_map(self):
        for active in ("mActiveModeManagers: [SoftApManager{id=1}]\n", "mSoftApLocalOnlyState 12\n", "mSoftApLocalOnlyState=13\n"):
            with self.subTest(active=active):
                self.assertFalse(self.parse("mCurrentSoftApInfoMap {}\n" + active)["no_ap_verified"])

    def test_radio_evidence_excludes_network_credentials(self):
        sample = self.parse("mCurrentSoftApInfoMap {SoftApInfo{frequency=2462, SSID=private-network, password=secret}}\n")
        self.assertNotIn("private-network", str(sample))
        self.assertNotIn("secret", str(sample))



class LanAdmissionTests(unittest.TestCase):
    def sample(self, *, bssid="a0:b1:c2:d3:e4:f5", ip="192.168.4.20", connected=True):
        status = ("Wi-fi is enabled\nWi-fi is connected\n" if connected else "Wi-fi is disabled\n")
        status += f"WifiInfo: SSID: private-network, BSSID: {bssid}, IP: /{ip}, Supplicant state: " + ("COMPLETED" if connected else "DISCONNECTED")
        return nearby.parse_radio(nearby.parse_station(status), "mCurrentSoftApInfoMap {}\n", "test")

    def test_actual_connected_station_ipv4_and_no_ap_are_all_required(self):
        self.assertTrue(nearby.lan_radio_admitted(self.sample()))
        for change in (dict(ip="0.0.0.0"), dict(ip="127.0.0.1"), dict(ip="169.254.1.2"),
                       dict(ip="999.1.2.3"), dict(bssid="02:00:00:00:00:00"), dict(connected=False)):
            with self.subTest(change=change):
                self.assertFalse(nearby.lan_radio_admitted(self.sample(**change)))

    def test_lan_cannot_admit_routerless_or_station_roaming_or_new_ip(self):
        baseline = self.sample()
        self.assertFalse(nearby.lan_radio_admitted(self.sample(bssid="a0:b1:c2:d3:e4:f6"), baseline))
        self.assertFalse(nearby.lan_radio_admitted(self.sample(ip="192.168.4.21"), baseline))
        self.assertFalse(nearby.lan_radio_admitted(dict(baseline, sta_disabled=True)))
        self.assertFalse(nearby.lan_radio_admitted(dict(baseline, no_ap_verified=False, current_ap_verified=True)))

    def test_network_identity_is_not_retained_in_radio_receipt(self):
        sample = str(self.sample())
        self.assertNotIn("private-network", sample)
        self.assertNotIn("a0:b1:c2:d3:e4:f5", sample)


class LanPersistedJobTests(unittest.TestCase):
    def proof(self):
        args = argparse.Namespace(sender_device="11111111-1111-4111-8111-111111111111",
                                  archive_id="22222222-2222-4222-8222-222222222222",
                                  file_name="fixture.zim", size=1024, sha256="a" * 64)
        job = dict(journal_key="job", id="job", fileName=args.file_name, status="complete",
                   url=f"kiwix-library://public/33333333333343338333333333333333/{args.sender_device}/{args.archive_id}",
                   downloaded=args.size, total=args.size, destination="/managed/fixture.zim", transport="quic-library",
                   publication=dict(size=args.size, sha256=args.sha256), matches_independent_received_bytes=True,
                   consumed_acquisition=True, installation_id="installed")
        result = dict(managed_path=job["destination"], installation_id="installed",
                      received_journal=dict(status="matched", match_count=1, matched_job=job))
        return result, args, job

    def test_real_persisted_session_peer_and_consumed_installation_are_required(self):
        result, args, _ = self.proof()
        nearby.validate_received_job(result, args)
        for field, value in (("consumed_acquisition", False), ("installation_id", "other"),
                             ("destination", "/other/fixture.zim"), ("transport", "http"),
                             ("error", "failed"), ("status", "paused"), ("downloaded", 1023)):
            with self.subTest(field=field):
                result, args, job = self.proof()
                job[field] = value
                with self.assertRaises(ValueError):
                    nearby.validate_received_job(result, args)

    def test_matching_bytes_and_uuid_do_not_admit_another_sender_or_missing_session(self):
        for url in ("kiwix-library://public/session/{peer}/{book}",
                    "kiwix-library://public/33333333333343338333333333333333/44444444-4444-4444-8444-444444444444/{book}",
                    "kiwix-library://public/33333333-3333-4333-8333-333333333333/{peer}/{book}",
                    "kiwix-library://public/{peer}/{book}"):
            with self.subTest(url=url):
                result, args, job = self.proof()
                job["url"] = url.format(peer=args.sender_device, book=args.archive_id)
                with self.assertRaises(ValueError):
                    nearby.validate_received_job(result, args)

    def test_ambiguous_jobs_or_nonpositive_publication_generation_do_not_admit(self):
        result, args, _ = self.proof()
        result["received_journal"]["match_count"] = 2
        with self.assertRaises(ValueError):
            nearby.validate_received_job(result, args)
        for generation in (0, -1, True, "1"):
            self.assertFalse(nearby.valid_publication(dict(publisher_id=args.sender_device, library_generation=generation)))


class InspectorTopologyTests(unittest.TestCase):
    def route(self, op, hotspot):
        fixture = r"""
const fs = require("node:fs"), vm = require("node:vm");
const scenario = JSON.parse(process.argv[1]);
const rect = () => ({left:0,top:0,right:300,bottom:100,width:300,height:100});
const node = (textContent) => ({textContent, getBoundingClientRect:rect});
const heading = node("Owned Mac publisher"), connected = node("Connected · 1 archives"), hotspotHeading = node("Temporary Wi-Fi");
const library = {...node("library"), querySelectorAll:(s) => s === ":scope > p[role='status']" ? [connected] : []};
const dialog = {...node("dialog"), querySelector:(s) => s === ".modal-heading h2" ? heading : null, contains:(e) => e === library};
const hotspot = {...node("hotspot"), querySelector:(s) => s === "h2" ? hotspotHeading : null};
const document = {querySelectorAll:(s) => s === "dialog[open]" ? [dialog] :
  s === ".localswarm-library" ? [library] : s === ".localswarm-hotspot" && scenario.hotspot ? [hotspot] : []};
const inspect = vm.runInNewContext(fs.readFileSync(process.argv[2],"utf8"), {document, TextEncoder,
  innerWidth:384,innerHeight:751,getComputedStyle:() => ({visibility:"visible",display:"block"})});
process.stdout.write(JSON.stringify(inspect({op:scenario.op,peer:"Owned Mac publisher",topology:"lan"})));
"""
        output = subprocess.run(["node", "-e", fixture, json.dumps(dict(op=op, hotspot=hotspot)),
                                 str(nearby.SOURCE / "inspect.js")],
                                capture_output=True, text=True, check=True, timeout=10)
        return json.loads(output.stdout)

    def test_lan_catalog_without_hotspot_cannot_satisfy_reverse_hotspot(self):
        self.assertTrue(self.route("lan-route", False)["ready"])
        self.assertNotIn("ready", self.route("reverse-route", False))

    def test_routerless_catalog_hotspot_is_fatal_in_explicit_lan(self):
        self.assertIn("error", self.route("lan-route", True))
        self.assertTrue(self.route("reverse-route", True)["product_owned_ap_at_catalog"])

    def test_sender_hotspot_catalog_rejects_reverse_receiver_ap(self):
        self.assertTrue(self.route("sender-hotspot-route", False)["ready"])
        self.assertIn("error", self.route("sender-hotspot-route", True))

    def test_catalog_cleanup_uses_header_close_not_duplicate_footer_close(self):
        fixture = r"""
const fs = require("node:fs"), vm = require("node:vm");
const rect = () => ({left:10,top:10,right:90,bottom:50,width:80,height:40});
const node = (textContent) => ({textContent, getBoundingClientRect:rect, getAnimations:() => [],
  getAttribute:() => null, contains:(other) => false});
const heading = node(process.argv[1]), close = {...node("Close"), contains:(other) => other === close};
const footerClose = node("Close");
const header = {...node("heading"), querySelectorAll:(s) => s === "button" ? [close] : []};
const dialog = {...node("dialog"), parentElement:null,
  querySelector:(s) => s === ".modal-heading h2" ? heading : null,
  querySelectorAll:(s) => s === ".modal-heading" ? [header] : s === "button" ? [close,footerClose] : []};
close.parentElement = header; header.parentElement = dialog;
const document = {querySelectorAll:(s) => s === "dialog[open]" ? [dialog] : [], elementFromPoint:() => close};
const inspect = vm.runInNewContext(fs.readFileSync(process.argv[2],"utf8"), {document,TextEncoder,
  innerWidth:384,innerHeight:751,getComputedStyle:() => ({visibility:"visible",display:"block",overflowY:"visible",overflowX:"visible"})});
process.stdout.write(JSON.stringify(inspect({op:"receive-close",peer:"Owned publisher",cleanup_owned:true})));
"""
        def inspect(peer):
            completed = subprocess.run(["node", "-e", fixture, peer, str(nearby.SOURCE / "inspect.js")],
                                       capture_output=True, text=True, check=True, timeout=10)
            return json.loads(completed.stdout)
        row = inspect("Owned publisher")
        self.assertTrue(row["ready"])
        self.assertTrue(row["tappable"])
        self.assertEqual(row["name"], "Close")
        self.assertIn("error", inspect("Unrelated dialog"))

class PublicationCleanupInspectionTests(unittest.TestCase):
    """Execute the real read-only inspector at failed-start and live-share boundaries."""
    def inspect(self, **scenario):
        fixture = r"""
const fs = require("node:fs"), vm = require("node:vm"), scenario = JSON.parse(process.argv[1]);
const rect = () => ({left:10,top:10,right:310,bottom:110,width:300,height:100});
const node = (textContent) => ({textContent,getBoundingClientRect:rect,getAttribute:() => null,closest:() => null});
const failure = node("Creating the Wi-Fi hotspot failed: HOTSPOT_UNAVAILABLE");
const heading = node(scenario.heading || (scenario.live ? "You’re sharing your library" : "Share your library"));
const start = {...node("Share all"), disabled:!!scenario.disabled};
const stop = node("Stop sharing"), hotspot = node("Temporary Wi-Fi");
const share = {...node("share"), getAttribute:(name) => name === "aria-busy" && !scenario.unknown_busy ? String(!!scenario.starting) : null,
  querySelector:(s) => s === "h2" ? heading : null,
  querySelectorAll:(s) => s === "button" ? (scenario.live ? [stop] : scenario.no_start ? [] : [start]) : []};
const dialog = {...node("unexpected dialog"),querySelector:() => null,contains:() => true};
const document = {querySelectorAll:(s) =>
  s === "dialog[open]" && scenario.dialog ? [dialog] :
  s === ".inline-error,[role='alert']" && scenario.failure ? [failure] :
  s === ".localswarm-share-card" && !scenario.no_share ? [share] :
  s === ".localswarm-hotspot" && scenario.hotspot ? [hotspot] : []};
const inspect = vm.runInNewContext(fs.readFileSync(process.argv[2],"utf8"), {document,
  innerWidth:384,innerHeight:751,getComputedStyle:() => ({visibility:"visible",display:"block"})});
process.stdout.write(JSON.stringify(inspect({op:scenario.op || "publication-stopped",cleanup_owned:scenario.owned !== false})));
"""
        completed = subprocess.run(["node", "-e", fixture, json.dumps(scenario), str(nearby.SOURCE / "inspect.js")],
                                   capture_output=True, text=True, check=True, timeout=10)
        return json.loads(completed.stdout)

    def test_failed_start_is_observed_already_stopped_without_suppressing_publication_error(self):
        stopped = self.inspect(failure=True)
        self.assertTrue(stopped["ready"])
        self.assertFalse(stopped["sharing_active"])
        self.assertTrue(stopped["publication_nonstarting"])
        self.assertTrue(stopped["publication_nonsharing"])
        self.assertFalse(stopped["hotspot_active"])
        self.assertIn("HOTSPOT_UNAVAILABLE", stopped["publication_errors"][0])
        self.assertNotIn("point", stopped)
        for request in (dict(owned=False), dict(op="hotspot"), dict(op="publish")):
            with self.subTest(request=request):
                rejected = self.inspect(failure=True, **request)
                self.assertIn("error", rejected)
                self.assertNotIn("ready", rejected)

    def test_live_owned_share_requires_stop_before_stopped_observation(self):
        live = self.inspect(live=True, hotspot=True, failure=True)
        self.assertNotIn("ready", live)
        self.assertTrue(live["sharing_active"])
        self.assertTrue(live["hotspot_active"])
        self.assertIn("HOTSPOT_UNAVAILABLE", live["publication_errors"][0])
        stopped = self.inspect()
        self.assertTrue(stopped["ready"])
        self.assertEqual(stopped["publication_errors"], [])

    def test_missing_transitional_or_unrelated_ui_never_clears_publication_ownership(self):
        for state in (dict(no_share=True), dict(no_start=True), dict(starting=True), dict(disabled=True),
                      dict(hotspot=True), dict(dialog=True), dict(unknown_busy=True), dict(heading="Unknown sharing state")):
            with self.subTest(state=state):
                self.assertNotIn("ready", self.inspect(failure=True, **state))


class NativeCleanupNormalizationTests(unittest.TestCase):
    def proof(self, mode="publish"):
        bound = dict(mode=mode, topology="sender-hotspot", nonce="a" * 32, deadline=12345,
                     device="actual-device", sender_device="actual-source", archive_id="actual-archive",
                     file_name="fixture.zim", title="Fixture", size=1024, sha256="b" * 64,
                     peer="Actual publisher", reviewed_crash_sha256="")
        identity = dict(package=nearby.TARGET, apk_sha256="c" * 64, certificate_sha256="d" * 64,
                        process_id=123, process_uid=10001, debuggable=False,
                        android_os=dict(api=28, model="actual-model"))
        owner = dict(schema=1, source="fixed retained KiwixMobilePlugin/LocalHotspot fields on main thread",
                     plugin_identity_bound=True, owner_present=True, generation=1, plugin_invoke_pending=False,
                     system_ui_pending=False, start_pending=False, reservation_active=False,
                     quic_route_retained=False, cancelled_start_pending=False, closing_callback_marker=False,
                     retired=True)
        retirement = dict(schema=1, publication_nonstarting=True, publication_nonsharing=True,
                          hotspot_retired=True, native_hotspot=owner)
        native = dict(bound, schema=1, installed_identity=identity, android_api=28, status="failed",
                      error="HOTSPOT_UNAVAILABLE", cleanup_complete=True, crash_report_preserved=True,
                      sender_route_started=mode == "publish", receiver_route_started=False,
                      no_ap_at_instrumentation_ready=True, preserved_before_jobs=True,
                      preserved_before_books_settings_tabs=True)
        if mode == "publish":
            native.update(primary_error="HOTSPOT_UNAVAILABLE", owned_sharing_stopped=True, publication_stopped=True, publication_retirement=retirement,
                          publication_stop_ui=dict(ready=True, publication_nonstarting=True, publication_nonsharing=True,
                              sharing_active=False, hotspot_active=False, publication_errors=["HOTSPOT_UNAVAILABLE"]))
        return native, bound, identity

    def test_failed_start_normal_and_fallback_keep_same_validated_retirement_and_first_failure(self):
        native, bound, identity = self.proof()
        normal = nearby.normalize_native_result(native, bound, identity, 28)
        fallback = nearby.normalize_native_result(native, bound, identity, 28,
                                                  host_failure=dict(status="failed", error="Host decoder failed"))
        for key, value in normal.items():
            self.assertEqual(fallback[key], value)
        self.assertTrue(fallback["cleanup_complete"])
        self.assertEqual(fallback["status"], "failed")
        self.assertEqual(fallback["error"], "HOTSPOT_UNAVAILABLE")
        self.assertEqual(fallback["device_cleanup_result"], native)
        self.assertEqual(fallback["host_failure"]["error"], "Host decoder failed")

    def test_wrong_source_identity_api_or_ownership_cannot_be_normalized_in_either_path(self):
        for key, value in (("nonce", "e" * 32), ("device", "another-device"), ("size", "1024"),
                           ("schema", True), ("android_api", 29), ("sender_route_started", None),
                           ("receiver_route_started", True), ("installed_identity", {})):
            for host_failure in (None, dict(status="failed", error="Host decoder failed")):
                with self.subTest(key=key, host_failure=host_failure):
                    native, bound, identity = self.proof()
                    native[key] = value
                    with self.assertRaises(ValueError):
                        nearby.normalize_native_result(native, bound, identity, 28, host_failure)

    def test_idle_ui_or_no_ap_cannot_override_native_pending_or_retained_resources(self):
        for key in ("plugin_invoke_pending", "system_ui_pending", "start_pending", "reservation_active",
                    "quic_route_retained", "cancelled_start_pending"):
            with self.subTest(key=key):
                native, bound, identity = self.proof()
                native["publication_retirement"]["native_hotspot"][key] = True
                with self.assertRaises(ValueError):
                    nearby.normalize_native_result(native, bound, identity, 28, dict(error="Host failed"))
        for key in ("publication_nonstarting", "publication_nonsharing", "hotspot_retired"):
            with self.subTest(key=key):
                native, bound, identity = self.proof()
                native["publication_retirement"][key] = False
                with self.assertRaises(ValueError):
                    nearby.normalize_native_result(native, bound, identity, 28)

    def test_later_cancellation_cannot_replace_original_product_failure_with_host_failure(self):
        native, bound, identity = self.proof()
        native["cancellation_requested"] = True
        observed = nearby.normalize_native_result(native, bound, identity, 28,
                                                  dict(error="Controller cancellation requested"))
        self.assertEqual(observed["error"], "HOTSPOT_UNAVAILABLE")
        self.assertEqual(observed["primary_error"], "HOTSPOT_UNAVAILABLE")
        self.assertTrue(observed["cancellation_requested"])
        self.assertEqual(observed["host_failure"]["error"], "Controller cancellation requested")
        native["primary_error"] = "another failure"
        with self.assertRaises(ValueError):
            nearby.normalize_native_result(native, bound, identity, 28)

    def test_pre_route_receiver_cancellation_preserves_native_identity_and_host_failure(self):
        native, bound, identity = self.proof(mode="receive")
        native.update(error="Host cancellation requested", cancellation_requested=True)
        normalized = nearby.normalize_native_result(native, bound, identity, 28,
                                                    dict(error="Cancelled before controller route gate"))
        self.assertEqual(normalized["installed_identity"], identity)
        self.assertFalse(normalized["receiver_route_started"])
        self.assertTrue(normalized["cleanup_complete"])
        self.assertEqual(normalized["error"], "Cancelled before controller route gate")
        self.assertEqual(normalized["device_cleanup_result"]["error"], "Host cancellation requested")


class InterruptedUiTimingTests(unittest.TestCase):
    def test_interrupted_ui_wait_ends_before_delayed_owned_cleanup(self):
        TimingLog, _ = nearby.load_timing_dependency(preparing=True)
        with tempfile.TemporaryDirectory() as directory, TimingLog(Path(directory), "interrupted-wait", "android_test") as timing:
            with patch.object(nearby, "TIMING", timing):
                wait = nearby.timed("ui_wait", "host_observed_native_ui_wait")
                wait.__enter__()
                try:
                    raise RuntimeError("Native event decoder interrupted")
                except RuntimeError:
                    cleanup = nearby.begin_cleanup_timing(wait)
                before = timing.summary()["category_wall_seconds_inclusive"]["ui_wait"]
                time.sleep(.03)  # A genuinely delayed cleanup, not a fabricated clock/sample.
                nearby.finish_timing_span(cleanup)
                after = timing.summary()
                self.assertEqual(after["category_wall_seconds_inclusive"]["ui_wait"], before)
                self.assertGreaterEqual(after["category_wall_seconds_inclusive"]["cleanup"], .02)
                self.assertGreater(after["total_wall_seconds"], before)


class FrozenTimingDependencyTests(unittest.TestCase):
    def test_missing_or_changed_frozen_timing_identity_rejects_before_module_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "timing.py"
            marker = Path(directory) / "unreviewed-code-executed"
            reviewed = b"class TimingLog:\n    reviewed = True\n"
            frozen = hashlib.sha256(reviewed).hexdigest()
            source.write_text("__import__('pathlib').Path(" + repr(str(marker)) + ").touch()\nclass TimingLog: pass\n")
            for expected in (None, "", True, "A" * 64, "a" * 63, frozen):
                with self.subTest(expected=expected), self.assertRaises(ValueError):
                    nearby.load_timing_dependency(expected, path=source)
                self.assertFalse(marker.exists())

    def test_only_verified_timing_bytes_execute_even_if_path_is_replaced_before_compile(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "timing.py"
            marker = Path(directory) / "replacement-executed"
            reviewed = b"class TimingLog:\n    reviewed = True\n"
            source.write_bytes(reviewed)
            original_module_type = nearby.types.ModuleType

            def replace_after_hash(name):
                source.write_text("__import__('pathlib').Path(" + repr(str(marker)) + ").touch()\nclass TimingLog: pass\n")
                return original_module_type(name)

            with patch.object(nearby.types, "ModuleType", side_effect=replace_after_hash):
                observed, digest = nearby.load_timing_dependency(hashlib.sha256(reviewed).hexdigest(), path=source)
            self.assertTrue(observed.reviewed)
            self.assertEqual(digest, hashlib.sha256(reviewed).hexdigest())
            self.assertFalse(marker.exists())


class FrozenPrepareReceiptTests(unittest.TestCase):
    def test_replacement_rejected_before_parsing_or_tool_device_access(self):
        original = b'{"reviewed_crash_sha256":"' + b"a" * 64 + b'"}'
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "prepare.json"
            receipt.write_bytes(original)
            frozen = hashlib.sha256(original).hexdigest()
            receipt.write_bytes(b"replacement is not even valid JSON")
            args = argparse.Namespace(device="serial", mode="capture-crash", nonce="a" * 32,
                                      deadline=int(time.time()) + 60, output=str(Path(directory) / "out"),
                                      prepared=str(receipt), prepared_sha256=frozen)
            with patch.object(nearby, "toolset") as tools, patch.object(nearby, "run") as commands:
                with self.assertRaisesRegex(ValueError, "differs from frozen SHA256"):
                    nearby.execute(args)
                tools.assert_not_called()
                commands.assert_not_called()
            self.assertFalse(Path(args.output).exists())

    def test_parses_the_verified_bytes_even_when_name_changes_before_parse(self):
        original = b'{"reviewed_crash_sha256":"' + b"a" * 64 + b'"}'
        replacement = b'{"reviewed_crash_sha256":"' + b"b" * 64 + b'"}'
        parse = json.loads
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "prepare.json"
            receipt.write_bytes(original)

            def replace_before_parse(data):
                receipt.write_bytes(replacement)
                return parse(data)

            with patch.object(nearby.json, "loads", side_effect=replace_before_parse):
                observed = nearby.load_prepare_receipt(receipt, hashlib.sha256(original).hexdigest())
            self.assertEqual(observed["reviewed_crash_sha256"], "a" * 64)
            self.assertEqual(receipt.read_bytes(), replacement)

    def test_receipt_reads_have_a_hard_size_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "prepare.json"
            receipt.write_bytes(b" " * (1024 * 1024 + 1))
            with self.assertRaisesRegex(ValueError, "exceeds size bound"):
                nearby.load_prepare_receipt(receipt)


class ReviewedCrashIdentityTests(unittest.TestCase):
    def test_receipt_is_the_authority_not_a_runtime_override(self):
        reviewed = "a" * 64
        args = argparse.Namespace(reviewed_crash_sha256=None)
        self.assertEqual(nearby.reviewed_crash_identity(args, {"reviewed_crash_sha256": reviewed}), reviewed)
        args.reviewed_crash_sha256 = "b" * 64
        with self.assertRaisesRegex(ValueError, "differs from prepare receipt"):
            nearby.reviewed_crash_identity(args, {"reviewed_crash_sha256": reviewed})
        with self.assertRaisesRegex(ValueError, "differs from prepare receipt"):
            nearby.reviewed_crash_identity(args, {"reviewed_crash_sha256": ""})

    def test_missing_malformed_or_multiple_identities_cannot_authorize_close(self):
        for frozen in (None, False, [], "A" * 64, "a" * 63, "a" * 64 + "," + "b" * 64):
            with self.subTest(frozen=frozen), self.assertRaises(ValueError):
                nearby.reviewed_crash_identity(argparse.Namespace(reviewed_crash_sha256=None),
                                               {"reviewed_crash_sha256": frozen})
        for supplied in ("", "A" * 64, "a" * 63, "a" * 65, "../pending.json"):
            with self.subTest(supplied=supplied), self.assertRaises(ValueError):
                nearby.reviewed_crash_identity(argparse.Namespace(reviewed_crash_sha256=supplied))


class RetainedCrashTests(unittest.TestCase):
    def evidence(self, data=b'{ "schema": 1, "message": "redacted native failure", "stack": [] }\n'):
        return dict(present=True, native_redacted=True, size=len(data),
                    sha256=hashlib.sha256(data).hexdigest(), base64=base64.b64encode(data).decode())

    def test_preserves_original_file_bytes_not_pretty_printed_equivalent(self):
        data = b'{ "stack": [], "message": "native failure", "schema": 1 }\n'
        evidence = self.evidence(data)
        identity = {"package": nearby.TARGET, "process_id": 6743}
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            nearby.retain_crash_report(out, evidence, identity)
            self.assertEqual((out / "crash-report.json").read_bytes(), data)
            receipt = json.loads((out / "crash-report-receipt.json").read_text())
            self.assertEqual(receipt["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(receipt["installed_identity"], identity)
            self.assertNotIn("base64", receipt)

    def test_changed_bytes_hash_size_or_redaction_assertion_never_retained(self):
        original = self.evidence()
        mutations = (
            {"sha256": "b" * 64},
            {"size": original["size"] - 1},
            {"native_redacted": False},
            {"base64": original["base64"] + "invalid!"},
            {"size": 32769},
            {"size": True},
        )
        for change in mutations:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                out = Path(directory)
                with self.assertRaises(ValueError):
                    nearby.retain_crash_report(out, dict(original, **change), {})
                self.assertFalse((out / "crash-report.json").exists())

    def test_nonrecord_or_unknown_schema_never_retained_even_with_matching_hash(self):
        for data in (b"[]", b"null", b'{"schema":2}', b'{"schema":true}', b"not JSON"):
            with self.subTest(data=data), tempfile.TemporaryDirectory() as directory:
                out = Path(directory)
                with self.assertRaises(ValueError):
                    nearby.retain_crash_report(out, self.evidence(data), {})
                self.assertFalse((out / "crash-report.json").exists())

    def test_absence_has_receipt_but_cannot_smuggle_report_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            nearby.retain_crash_report(out, {"present": False}, {})
            self.assertFalse((out / "crash-report.json").exists())
            self.assertFalse(json.loads((out / "crash-report-receipt.json").read_text())["present"])
            with self.assertRaises(ValueError):
                nearby.retain_crash_report(out, {"present": False, "base64": "anything"}, {})


if __name__ == "__main__":
    unittest.main()
