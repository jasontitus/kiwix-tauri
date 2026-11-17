// SPDX-License-Identifier: GPL-3.0-or-later
package org.kiwix.nearby;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.graphics.Bitmap;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.wifi.WifiInfo;
import android.graphics.Rect;
import android.os.Bundle;
import android.os.Build;
import android.os.SystemClock;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructStat;
import android.util.Base64;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.accessibility.AccessibilityNodeInfo;
import android.webkit.WebView;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileDescriptor;
import java.io.FileInputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** Test-APK-only runner: no application, AndroidX, Kotlin, or JNI dependencies. */
public final class NearbyInstrumentation extends Instrumentation {
    private static final String TARGET = "org.kiwix.workbench";
    private static final String PREFIX = "KIWIX_ANDROID_NEARBY ";
    private Bundle args;
    private volatile Activity activity;
    private JSONObject result;
    private JSONObject before;
    private Context target;
    private File root;
    private String inspector;
    private long end;
    private long started;
    private volatile boolean sharingOwned;
    private final AtomicBoolean publisherReady = new AtomicBoolean();
    private final AtomicBoolean stopPublish = new AtomicBoolean();
    private final AtomicReference<JSONObject> publishAck = new AtomicReference<>();
    private JSONObject downloadsBefore;
    private final AtomicBoolean finished = new AtomicBoolean();
    private final AtomicBoolean cancellation = new AtomicBoolean();
    private final AtomicReference<JSONObject> routeAck = new AtomicReference<>();
    private final AtomicReference<JSONObject> initialAck = new AtomicReference<>();
    private android.content.BroadcastReceiver control;
    private boolean cleaning;
    private JSONObject crashSnapshot;
    private JSONObject crashRecord;
    private boolean receiveOwned;
    private Network receivedNetwork;
    private final List<Long> initialNetworks = new ArrayList<>();
    private Object publisherPlugin;

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        args = arguments;
        start();
    }

    @Override public void callActivityOnResume(Activity value) {
        super.callActivityOnResume(value);
        if (TARGET.equals(value.getPackageName())) activity = value;
    }

    private String arg(String key) {
        String value = args.getString(key);
        if (value == null || value.isEmpty()) throw new IllegalArgumentException("Missing " + key);
        return value;
    }

    private boolean lan() { return "lan".equals(arg("topology")); }
    private boolean senderHotspot() { return "sender-hotspot".equals(arg("topology")); }
    private boolean strictReceive() { return lan() || senderHotspot(); }

    private ConnectivityManager connectivity() {
        return (ConnectivityManager)target.getSystemService(Context.CONNECTIVITY_SERVICE);
    }

    private void captureJoinedNetwork() throws Exception {
        if (Build.VERSION.SDK_INT < 30) throw new IllegalStateException("Sender-hotspot receiving requires API30+");
        Network found = null;
        for (Network network : connectivity().getAllNetworks()) {
            NetworkCapabilities caps = connectivity().getNetworkCapabilities(network);
            if (caps == null || !caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
                    || initialNetworks.contains(network.getNetworkHandle())) continue;
            if (found != null) throw new IllegalStateException("Ambiguous new receiver Wi-Fi Network");
            WifiInfo wifi = ((android.net.wifi.WifiManager)target.getApplicationContext()
                .getSystemService(Context.WIFI_SERVICE)).getConnectionInfo();
            String ssid = wifi == null ? null : wifi.getSSID();
            if (ssid == null || ssid.equals("<unknown ssid>")) throw new IllegalStateException("Joined network identity unavailable");
            if (ssid.startsWith("\"") && ssid.endsWith("\"")) ssid = ssid.substring(1, ssid.length() - 1);
            result.put("joined_network_sha256", hash(ssid.getBytes(StandardCharsets.UTF_8)));
            if (!result.getString("joined_network_sha256").equals(initialAck.get().getString("sender_network_sha256")))
                throw new IllegalStateException("Actual receiver station differs from admitted sender AP");
            found = network;
        }
        if (found == null) throw new IllegalStateException("No actual newly joined receiver Wi-Fi Network");
        receivedNetwork = found;
        result.put("joined_network_handle", found.getNetworkHandle());
    }

    private void retireReceive() throws Exception {
        // Stop all is admitted only with an isolated no-sharing/no-active-nearby-job
        // baseline. Unlike Stop finding, its awaited product path revokes consumers.
        AccessibilityNodeInfo foreground = automation().getRootInActiveWindow();
        if (foreground != null && "com.android.settings".contentEquals(foreground.getPackageName())) {
            joinConsent(foreground, true);
            SystemClock.sleep(150);
        }
        if (!inspect("receive-close").optBoolean("no_dialog")) tap("receive-close");
        tap("nearby");
        tap("stop-receive");
        JSONObject stopped = waitReady("receive-stopped");
        if (receivedNetwork != null) {
            while (connectivity().getNetworkCapabilities(receivedNetwork) != null) pause();
        } else {
            for (Network network : connectivity().getAllNetworks())
                if (!initialNetworks.contains(network.getNetworkHandle()))
                    throw new IllegalStateException("Uncaptured receiver route remains; retirement not proven");
        }
        preserve(state());
        receiveOwned = false;
        result.put("receiver_join_retired", true).put("receiver_consumer_retired", true)
            .put("receiver_retirement", stopped.put("source", "awaited ordinary Stop all plus captured Network disappearance")
                .put("joined_network_handle", receivedNetwork == null ? JSONObject.NULL : receivedNetwork.getNetworkHandle()));
    }

    private boolean lanEvidence(JSONObject evidence, JSONObject baseline) throws Exception {
        return evidence.getBoolean("sta_connected") && !evidence.getBoolean("sta_disabled")
            && evidence.getBoolean("no_ap_verified") && !evidence.getBoolean("current_ap_verified")
            && evidence.getString("station_identity_sha256").matches("[0-9a-f]{64}")
            && evidence.getString("station_ipv4").matches("[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+")
            && (baseline == null || (evidence.getString("station_identity_sha256").equals(baseline.getString("station_identity_sha256"))
                && evidence.getString("station_ipv4").equals(baseline.getString("station_ipv4"))));
    }

    private static final class HostCancellation extends IllegalStateException {
        HostCancellation() { super("Host cancellation requested"); }
    }

    private void check() {
        if (cancellation.get() && !cleaning) throw new HostCancellation();
        if (SystemClock.elapsedRealtime() >= end) throw new IllegalStateException("Explicit deadline reached");
    }

    private void pause() {
        check();
        SystemClock.sleep(Math.min(250, Math.max(1, end - SystemClock.elapsedRealtime())));
    }

    private void emit(String event, JSONObject fields) {
        try {
            JSONObject row = new JSONObject().put("event", event).put("nonce", arg("nonce")).put("topology", arg("topology"))
                .put("elapsed_ms", SystemClock.elapsedRealtime() - started).put("data", fields);
            Bundle status = new Bundle();
            status.putString("stream", PREFIX + row.toString() + "\n");
            sendStatus(0, status);
        } catch (Exception error) { throw new IllegalStateException(error); }
    }

    @Override public void onStart() {
        started = SystemClock.elapsedRealtime();
        try {
            long deadline = Long.parseLong(arg("deadline")) * 1000;
            long remaining = deadline - System.currentTimeMillis();
            if (remaining < 10000 || remaining > 3600000) throw new IllegalArgumentException("Deadline must be 10..3600 seconds away");
            end = started + remaining - 5000; // reserve bounded owned-sharing cleanup
            final long hardEnd = started + remaining;
            Thread watchdog = new Thread(() -> {
                SystemClock.sleep(Math.max(1, hardEnd - SystemClock.elapsedRealtime() - 500));
                if (finished.compareAndSet(false, true)) {
                    try { emit("watchdog", new JSONObject().put("status", "failed").put("error", "Hard instrumentation deadline reached")); }
                    catch (Throwable ignored) { }
                    // Kill only this instrumentation-owned target process if a
                    // blocked framework/main-thread call prevents normal finish.
                    android.os.Process.killProcess(android.os.Process.myPid());
                }
            }, "nearby-finite-deadline");
            watchdog.setDaemon(true);
            watchdog.start();
            result = new JSONObject();
            String mode = arg("mode");
            result.put("sender_route_started", false).put("receiver_route_started", false);
            result.put("android_api", Build.VERSION.SDK_INT);
            if (!"routerless".equals(arg("topology")) && !lan() && !senderHotspot()) throw new IllegalArgumentException("Unknown topology");
            for (String key : new String[]{"mode", "topology", "nonce", "deadline", "device"}) result.put(key, arg(key));
            if (!"capture-crash".equals(mode)) {
                for (String key : new String[]{"sender_device", "archive_id", "file_name", "title", "size", "sha256", "peer"}) result.put(key, arg(key));
                result.put("size", Long.parseLong(arg("size")));
            }
            String reviewed = args.getString("reviewed_crash_sha256");
            if (reviewed == null || (!reviewed.isEmpty() && !reviewed.matches("[0-9a-f]{64}")))
                throw new IllegalArgumentException("Explicit frozen reviewed crash identity required");
            result.put("reviewed_crash_sha256", reviewed).put("schema", 1).put("status", "running")
                .put("deadline", Long.parseLong(arg("deadline"))).put("ui_launched", false);
            target = getTargetContext();
            if (!TARGET.equals(target.getPackageName())) throw new IllegalStateException("Wrong target package");
            root = new File(target.getFilesDir().getCanonicalFile(), "kiwix");
            registerControl();
            JSONObject identity = installedIdentity();
            result.put("installed_identity", identity);
            emit("identity", identity);
            crashSnapshot = readCrashReport();
            emit("crash-report", crashSnapshot);
            if ("capture-crash".equals(mode)) {
                if (!crashSnapshot.getBoolean("present")) throw new IllegalStateException("No pending native crash report");
                result.put("profile_read_only", true);
            } else {
                if (crashSnapshot.getBoolean("present")) {
                    if (!reviewed.equals(crashSnapshot.getString("sha256")))
                        throw new IllegalStateException("Unreviewed native crash report; capture and review exact full-file SHA256 before retrying");
                    crashRecord = new JSONObject(new String(Base64.decode(crashSnapshot.getString("base64"), Base64.NO_WRAP), StandardCharsets.UTF_8));
                } else if (!reviewed.isEmpty()) throw new IllegalStateException("Reviewed native crash report is no longer present");
                if (Build.VERSION.SDK_INT == 28 && senderHotspot() && "publish".equals(mode)) {
                    android.location.LocationManager location =
                        (android.location.LocationManager)target.getSystemService(Context.LOCATION_SERVICE);
                    if (location == null) throw new IllegalStateException("Current LocationManager unavailable; no settings input admitted");
                    boolean enabled = location.isLocationEnabled();
                    JSONObject evidence = new JSONObject().put("schema", 1).put("android_api", Build.VERSION.SDK_INT)
                        .put("source", "LocationManager.isLocationEnabled").put("location_services_enabled", enabled);
                    result.put("location_preflight", evidence);
                    emit("location-preflight", evidence);
                    if (!enabled) throw new IllegalStateException("NEARBY_LOCATION_SERVICES_REQUIRED: actual current LocationManager is disabled; no settings toggled");
                }
                before = state();
                downloadsBefore = readJobs();
                JSONObject book = findBook(before, arg("archive_id"));
                JSONObject settings = before.getJSONObject("settings");
                result.put("bandwidth_limits", new JSONObject().put("downloadLimit", settings.getLong("downloadLimit"))
                    .put("uploadLimit", settings.getLong("uploadLimit")));
                if ("receive".equals(mode) && book != null) throw new IllegalStateException("Expected archive is already in library; refusing duplicate/reselection");
                if (!"receive".equals(mode) && book == null) throw new IllegalStateException("Expected archive missing for " + mode);
                emit("before", new JSONObject().put("books", before.getJSONArray("books")).put("windows", before.optJSONObject("windows"))
                    .put("settings_sha256", hash(canonical(before.getJSONObject("settings")).getBytes(StandardCharsets.UTF_8))));
                if ("verify".equals(mode)) {
                    verifyFile(before, false);
                } else if ("receive".equals(mode) || "publish".equals(mode)) {
                    inspector = new String(readAsset("inspect.js"), StandardCharsets.UTF_8);
                    launch();
                    result.put("ui_launched", true);
                    if (crashRecord != null) closeReviewedCrash();
                    assertCrashUnchanged();
                    tap("nearby");
                    JSONObject ready = waitReady("receiving-ready");
                    if (!senderHotspot() && !String.valueOf(!lan()).equals(arg("sta_disabled")))
                        throw new IllegalStateException("Host station admission differs from explicit topology");
                    if (ready.getBoolean("hotspot_active")) throw new IllegalStateException("Pre-existing product AP at instrumentation ready");
                    if (("publish".equals(mode) || strictReceive()) && ready.getBoolean("sharing_active"))
                        throw new IllegalStateException("Pre-existing library sharing is not owned by this run");
                    if (strictReceive() && ready.getBoolean("other_shares_active"))
                        throw new IllegalStateException("Pre-existing archive share is not owned by this run");
                    result.put("sta_disabled", Boolean.parseBoolean(arg("sta_disabled"))).put("no_ap_at_instrumentation_ready", true);
                    if (senderHotspot() && "publish".equals(mode)) {
                        String peer = ownedPublicationText("peer-id", 64);
                        if (!UUID.fromString(peer).toString().equals(peer)) throw new IllegalStateException("Noncanonical publisher ID");
                        result.put("publisher_id", peer);
                    }
                    if ("receive".equals(mode) && !strictReceive()) result.put("reverse_hotspot_required", true).put("reverse_hotspot_verified", false);
                    if (senderHotspot() && "publish".equals(mode)) {
                        JSONObject baseline = nativeHotspotState();
                        if (!baseline.getBoolean("retired")) throw new IllegalStateException("Pre-existing native hotspot ownership is not owned by this run");
                        result.put("publication_native_baseline", baseline);
                    }
                    emit("instrumentation-ready", new JSONObject(result.toString()).put("status", "ready"));
                    // Both directions keep route-creating native input untouched
                    // until the host acknowledges the fresh initial OS baseline.
                    while (initialAck.get() == null) pause();
                    if ("receive".equals(mode)) receive();
                    else publish();
                } else throw new IllegalArgumentException("Unsupported mode");
                preserve(state());
            }
            assertCrashUnchanged();
            check();
            result.put("status", "complete");
        } catch (Throwable error) {
            try {
                if (result == null) result = new JSONObject().put("schema", 1);
                result.put("status", "failed").put("error", error.toString());
                if (!(error instanceof HostCancellation)) result.put("primary_error", error.toString());
                emit("failure", new JSONObject().put("error", error.toString()));
                if (activity != null) screenshot("failure.png");
            } catch (Throwable ignored) { /* the final receipt still reports the original failure */ }
        } finally {
            cleaning = true;
            try {
                if (sharingOwned) {
                    end = Math.min(SystemClock.elapsedRealtime() + 4500,
                        SystemClock.elapsedRealtime() + Long.parseLong(arg("deadline")) * 1000 - System.currentTimeMillis() - 250);
                    // A failed start may already have returned to the explicit
                    // unshared/no-hotspot UI. Intent alone is not a live share.
                    JSONObject stopped;
                    JSONObject nativeStopped = null;
                    boolean stopTapped = false;
                    while (true) {
                        stopped = inspect("publication-stopped");
                        if (stopped.optBoolean("ready")) {
                            if (!senderHotspot()) break;
                            nativeStopped = nativeHotspotState();
                            if (nativeStopped.getBoolean("retired")) break;
                        }
                        if (stopped.optBoolean("sharing_active") && !stopTapped) {
                            tap("stop-share");
                            stopTapped = true;
                        }
                        pause();
                    }
                    sharingOwned = false;
                    result.put("owned_sharing_stopped", true).put("publication_stopped", true)
                        .put("publication_stop_ui", stopped);
                    if (senderHotspot()) result.put("publication_retirement", new JSONObject().put("schema", 1)
                        .put("publication_nonstarting", stopped.getBoolean("publication_nonstarting"))
                        .put("publication_nonsharing", stopped.getBoolean("publication_nonsharing"))
                        .put("hotspot_retired", nativeStopped.getBoolean("retired")).put("native_hotspot", nativeStopped));
                    JSONArray cleanupErrors = stopped.getJSONArray("publication_errors");
                    if (cleanupErrors.length() > 0) {
                        result.put("status", "failed");
                        if (!result.has("error")) result.put("error", "Publication cleanup observed page errors: " + cleanupErrors);
                    }
                    preserve(state());
                    emit("cleanup", new JSONObject().put("owned_sharing_stopped", true).put("publication_stopped", true));
                }
                if (receiveOwned) {
                    end = Math.min(SystemClock.elapsedRealtime() + 4500,
                        SystemClock.elapsedRealtime() + Long.parseLong(arg("deadline")) * 1000 - System.currentTimeMillis() - 250);
                    retireReceive();
                }
            } catch (Throwable cleanup) {
                try { result.put("status", "failed").put("cleanup_error", cleanup.toString()); } catch (Exception ignored) { }
            }
            if (control != null) {
                try { target.unregisterReceiver(control); }
                catch (Throwable error) { try { result.put("status", "failed").put("cleanup_error", error.toString()); } catch (Exception ignored) { } }
            }
            if (cancellation.get()) {
                try {
                    if (!result.has("error")) result.put("error", "Host cancellation requested");
                    result.put("status", "failed").put("cancellation_requested", true);
                }
                catch (Exception ignored) { }
            }
            try {
                if (crashSnapshot != null) {
                    assertCrashUnchanged();
                    result.put("crash_report_preserved", true);
                }
            } catch (Throwable error) {
                try { result.put("status", "failed").put("crash_report_preserved", false)
                    .put("crash_preservation_error", error.toString()); }
                catch (Exception ignored) { }
            }
            try { result.put("elapsed_ms", SystemClock.elapsedRealtime() - started).put("cleanup_complete", !sharingOwned && !receiveOwned); emit("result", result); }
            catch (Throwable ignored) { }
            if (finished.compareAndSet(false, true)) {
                Bundle finalBundle = new Bundle();
                finalBundle.putString("stream", "Nearby instrumentation finished\n");
                int code = "complete".equals(result.optString("status")) ? Activity.RESULT_OK : Activity.RESULT_CANCELED;
                try {
                    finish(code, finalBundle);
                } catch (RuntimeException unretired) {
                    // Instrumentation.finish() disconnects UiAutomation before
                    // finishing the instrumentation thread, so a dead binder
                    // leaves this run owned forever with the watchdog disabled.
                    // Retire the exact owned target process ourselves.
                    String detail = unretired.getClass().getName() + ": " + unretired.getMessage();
                    android.util.Log.w("KiwixNearby", "finish failed; retiring owned target", unretired);
                    try {
                        emit("finish-retirement", new JSONObject()
                            .put("status", "failed").put("action", "kill-owned-target")
                            .put("error", detail));
                    } catch (Throwable ignored) { /* the receipt already reported the run */ }
                    android.os.Process.killProcess(android.os.Process.myPid());
                }
            }
        }
    }

    /** Test-only host coordination, not an application command or UI bypass. */
    private void registerControl() throws Exception {
        final String action = "org.kiwix.nearby.CONTROL." + arg("nonce");
        control = new android.content.BroadcastReceiver() {
            @Override public void onReceive(Context context, Intent intent) {
                if (!action.equals(intent.getAction()) || !arg("nonce").equals(intent.getStringExtra("nonce"))) return;
                String command = intent.getStringExtra("command");
                if ("cancel".equals(command)) cancellation.set(true);
                if ("STOP_PUBLISH".equals(command) && "publish".equals(arg("mode"))) {
                    if (publisherReady.get() && sharingOwned) stopPublish.set(true);
                    else cancellation.set(true);
                }
                if (("route_ack".equals(command) || "ready_ack".equals(command) || "publish_ack".equals(command)) && !cancellation.get()) {
                    try {
                        byte[] bytes = Base64.decode(intent.getStringExtra("evidence"), Base64.URL_SAFE | Base64.NO_WRAP);
                        JSONObject evidence = new JSONObject(new String(bytes, StandardCharsets.UTF_8));
                        if (arg("nonce").equals(evidence.getString("nonce")) && arg("deadline").equals(evidence.getString("deadline"))
                                && arg("topology").equals(evidence.getString("topology"))) {
                            if (lan()) {
                                if ("ready_ack".equals(command) && lanEvidence(evidence, null)) initialAck.compareAndSet(null, evidence);
                                if (initialAck.get() != null && lanEvidence(evidence, initialAck.get())) {
                                    if ("route_ack".equals(command)) routeAck.compareAndSet(null, evidence);
                                    if ("publish_ack".equals(command) && "publish".equals(arg("mode"))) publishAck.compareAndSet(null, evidence);
                                }
                            } else if (senderHotspot()) {
                                if ("ready_ack".equals(command) && evidence.getBoolean("sta_disconnected")
                                        && !evidence.getBoolean("sta_connected") && evidence.getBoolean("no_ap_verified")
                                        && !evidence.getBoolean("current_ap_verified")
                                        && (!"receive".equals(arg("mode")) || evidence.getString("sender_network_sha256").matches("[0-9a-f]{64}")))
                                    initialAck.compareAndSet(null, evidence);
                                if (initialAck.get() != null) {
                                    if ("route_ack".equals(command) && "receive".equals(arg("mode"))
                                            && lanEvidence(evidence, null)) routeAck.compareAndSet(null, evidence);
                                    if ("publish_ack".equals(command) && "publish".equals(arg("mode"))
                                            && !evidence.getBoolean("sta_connected") && evidence.getBoolean("current_ap_verified")
                                            && !evidence.getBoolean("no_ap_verified")) publishAck.compareAndSet(null, evidence);
                                }
                            } else if (evidence.getBoolean("sta_disabled")) {
                                if ("route_ack".equals(command) && evidence.getBoolean("current_ap_verified")) routeAck.compareAndSet(null, evidence);
                                if ("ready_ack".equals(command) && evidence.getBoolean("no_ap_verified")) initialAck.compareAndSet(null, evidence);
                                if ("publish_ack".equals(command) && "publish".equals(arg("mode"))
                                        && evidence.getBoolean("current_ap_verified") && evidence.getJSONArray("ap_frequency_mhz").length() > 0)
                                    publishAck.compareAndSet(null, evidence);
                            }
                        }
                    } catch (Exception ignored) { /* malformed acknowledgements cannot admit a route */ }
                }
            }
        };
        android.content.IntentFilter filter = new android.content.IntentFilter(action);
        if (android.os.Build.VERSION.SDK_INT >= 33) target.registerReceiver(control, filter, Context.RECEIVER_EXPORTED);
        else target.registerReceiver(control, filter);
        emit("control-ready", new JSONObject().put("action", action));
    }

    private JSONObject installedIdentity() throws Exception {
        PackageManager pm = target.getPackageManager();
        PackageInfo info = pm.getPackageInfo(TARGET, PackageManager.GET_SIGNATURES);
        ApplicationInfo app = info.applicationInfo;
        if (app.uid != android.os.Process.myUid()) throw new IllegalStateException("Instrumentation is not running as the installed target UID");
        if ((app.flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0) throw new IllegalStateException("Target is debuggable, expected optimized release");
        if (!TARGET.equals(info.packageName)) throw new IllegalStateException("Package mismatch");
        Signature[] signatures = info.signatures;
        if (signatures == null || signatures.length != 1) throw new IllegalStateException("Expected one installed signer");
        String certificate = hash(signatures[0].toByteArray());
        if (!certificate.equals(arg("certificate_sha256"))) throw new IllegalStateException("Installed certificate changed");
        JSONObject measured = digest(new File(app.sourceDir), false);
        if (!measured.getString("sha256").equals(arg("target_apk_sha256"))) throw new IllegalStateException("Installed APK changed after host probe");
        JSONObject identity = new JSONObject().put("package", info.packageName).put("version_name", info.versionName)
            .put("version_code", android.os.Build.VERSION.SDK_INT >= 28 ? info.getLongVersionCode() : info.versionCode)
            .put("apk_sha256", measured.getString("sha256")).put("certificate_sha256", certificate)
            .put("debuggable", false).put("process_id", android.os.Process.myPid()).put("process_uid", android.os.Process.myUid());
        identity.put("android_os", new JSONObject().put("api", Build.VERSION.SDK_INT).put("release", Build.VERSION.RELEASE)
            .put("manufacturer", Build.MANUFACTURER).put("model", Build.MODEL)
            .put("fingerprint_sha256", hash(Build.FINGERPRINT.getBytes(StandardCharsets.UTF_8))));
        return identity;
    }

    private void launch() throws Exception {
        Intent launch = target.getPackageManager().getLaunchIntentForPackage(TARGET);
        if (launch == null || launch.getComponent() == null || !TARGET.equals(launch.getComponent().getPackageName())) throw new IllegalStateException("No exact target launch activity");
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivitySync(launch);
        while (activity == null || webView() == null) { allowScopedPermission(); pause(); }
    }

    private WebView webView() throws Exception {
        AtomicReference<WebView> found = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        runOnMainSync(() -> {
            try {
                List<WebView> views = new ArrayList<>();
                if (activity != null) collect(activity.getWindow().getDecorView(), views);
                if (views.size() > 1) throw new IllegalStateException("Ambiguous visible WebViews");
                if (views.size() == 1) found.set(views.get(0));
            } catch (Throwable error) { failure.set(error); }
        });
        if (failure.get() != null) throw new IllegalStateException(failure.get());
        return found.get();
    }

    private void collect(View view, List<WebView> found) {
        if (!view.isShown()) return;
        if (view instanceof WebView) { found.add((WebView)view); return; }
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup)view;
            for (int i = 0; i < group.getChildCount(); i++) collect(group.getChildAt(i), found);
        }
    }

    private JSONObject inspect(String op) throws Exception {
        check();
        WebView web = webView();
        if (web == null) {
            AccessibilityNodeInfo foreground = automation().getRootInActiveWindow();
            String pkg = foreground == null || foreground.getPackageName() == null ? "" : foreground.getPackageName().toString();
            if (!pkg.isEmpty() && !TARGET.equals(pkg) && !"com.android.permissioncontroller".equals(pkg)
                    && !"com.google.android.permissioncontroller".equals(pkg))
                throw new IllegalStateException("Target WebView lost to unexpected foreground package: " + pkg);
            return new JSONObject().put("pending", "No foreground WebView");
        }
        JSONObject request = new JSONObject().put("op", op).put("peer", arg("peer")).put("topology", arg("topology"))
            .put("file_name", arg("file_name")).put("title", arg("title"));
        request.put("cleanup_owned", cleaning && (sharingOwned || receiveOwned));
        if (crashRecord != null) request.put("crash_record", crashRecord);
        CountDownLatch latch = new CountDownLatch(1);
        AtomicReference<String> response = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        runOnMainSync(() -> {
            try {
                web.evaluateJavascript("(function(){try{return (" + inspector + ")(" + request + ");}catch(e){return {error:String(e)};}})()", value -> {
                    response.set(value); latch.countDown();
                });
            } catch (Throwable error) { failure.set(error); latch.countDown(); }
        });
        if (!latch.await(Math.max(1, Math.min(3000, end - SystemClock.elapsedRealtime())), TimeUnit.MILLISECONDS)) throw new IllegalStateException("WebView read-only inspection timed out");
        if (failure.get() != null) throw new IllegalStateException(failure.get());
        String raw = response.get();
        if (raw == null || "null".equals(raw)) return new JSONObject().put("pending", "Document not ready");
        JSONObject row = new JSONObject(raw);
        if (row.has("error")) throw new IllegalStateException(row.toString());
        return row;
    }

    private JSONObject waitReady(String op) throws Exception {
        String last = "";
        while (true) {
            check();
            allowScopedPermission();
            JSONObject row = inspect(op);
            if (row.optBoolean("ready")) {
                emit("ui-ready", new JSONObject().put("operation", op));
                return row;
            }
            String current = row.toString();
            if (!current.equals(last)) { emit("waiting", new JSONObject().put("operation", op).put("inspection", row)); last = current; }
            pause();
        }
    }

    private float[] screen(JSONObject viewport, double x, double y) throws Exception {
        WebView web = webView();
        if (web == null) throw new IllegalStateException("WebView disappeared");
        AtomicReference<float[]> point = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        runOnMainSync(() -> {
            try {
                if (activity == null || !activity.hasWindowFocus()) throw new IllegalStateException("Target Activity is not foreground/focused");
                int[] origin = new int[2]; web.getLocationOnScreen(origin);
                double scale = web.getWidth() / viewport.optDouble("width", 0);
                if (!Double.isFinite(scale) || scale <= 0) throw new IllegalStateException("Invalid DOM/native scale");
                point.set(new float[]{(float)(origin[0] + x * scale), (float)(origin[1] + y * scale)});
            } catch (Throwable error) { failure.set(error); }
        });
        if (failure.get() != null) throw new IllegalStateException(failure.get());
        return point.get();
    }

    private void motion(float x, float y, int action, long down) {
        MotionEvent event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0);
        event.setSource(InputDevice.SOURCE_TOUCHSCREEN);
        try {
            if (!automation().injectInputEvent(event, true)) throw new IllegalStateException("Native touch injection rejected");
        } finally { event.recycle(); }
    }

    private void admitInput() throws Exception {
        // Only stopping publication already started by this run may cross a
        // newly discovered crash barrier; no permission/transfer input may.
        if (cleaning) {
            if (!sharingOwned && !receiveOwned) throw new IllegalStateException("No owned cleanup input");
        } else assertCrashUnchanged();
    }

    private void touch(float x, float y, boolean beginPublication) throws Exception {
        admitInput();
        if (beginPublication) {
            sharingOwned = true;
            result.put("sender_route_started", true);
        }
        long down = SystemClock.uptimeMillis();
        motion(x, y, MotionEvent.ACTION_DOWN, down);
        SystemClock.sleep(60);
        motion(x, y, MotionEvent.ACTION_UP, down);
    }

    private boolean sameRect(JSONObject first, JSONObject second) throws Exception {
        return first.getDouble("left") == second.getDouble("left") && first.getDouble("top") == second.getDouble("top")
            && first.getDouble("right") == second.getDouble("right") && first.getDouble("bottom") == second.getDouble("bottom");
    }

    private boolean sameTapTarget(JSONObject first, JSONObject second) throws Exception {
        return second.optBoolean("ready") && second.optBoolean("tappable")
            && first.getString("name").equals(second.getString("name"))
            && first.optString("value").equals(second.optString("value"))
            && sameRect(first.getJSONObject("rect"), second.getJSONObject("rect"))
            && sameRect(first.getJSONObject("clip"), second.getJSONObject("clip"))
            && first.getJSONObject("viewport").getDouble("width") == second.getJSONObject("viewport").getDouble("width")
            && first.getJSONObject("viewport").getDouble("height") == second.getJSONObject("viewport").getDouble("height");
    }

    /** Independent native accessible target at the actual injected point; no DOM-only authority for crash actions. */
    private JSONObject nativeCrashHit(String op, JSONObject inspection, float[] point) throws Exception {
        if (!"crash-view".equals(op) && !"crash-close".equals(op)) return null;
        String expected = "crash-view".equals(op) ? "View report" : "Close";
        if (!expected.equals(inspection.getString("name"))) throw new IllegalStateException("Unexpected scoped crash input label");
        AccessibilityNodeInfo foreground = automation().getRootInActiveWindow();
        if (foreground == null || !TARGET.contentEquals(foreground.getPackageName() == null ? "" : foreground.getPackageName()))
            throw new IllegalStateException("Scoped crash input requires the exact foreground target package");
        List<AccessibilityNodeInfo> nodes = new ArrayList<>();
        collectNodes(foreground, nodes);
        Rect admitted = null;
        for (AccessibilityNodeInfo node : nodes) {
            if (!node.isVisibleToUser() || !node.isEnabled() || !node.isClickable()) continue;
            String label = node.getText() == null ? "" : node.getText().toString();
            String description = node.getContentDescription() == null ? "" : node.getContentDescription().toString();
            if (!expected.equals(label) && !expected.equals(description)) continue;
            Rect bounds = new Rect();
            node.getBoundsInScreen(bounds);
            if (!bounds.contains(Math.round(point[0]), Math.round(point[1]))) continue;
            if (admitted != null) throw new IllegalStateException("Ambiguous native crash action at injected point");
            admitted = bounds;
        }
        if (admitted == null) throw new IllegalStateException("Native crash action does not match the injected point");
        return new JSONObject().put("package", TARGET).put("name", expected)
            .put("left", admitted.left).put("top", admitted.top).put("right", admitted.right).put("bottom", admitted.bottom);
    }

    private void tap(String op) throws Exception {
        if (cleaning && !(sharingOwned && "stop-share".equals(op)) &&
                !(receiveOwned && ("nearby".equals(op) || "stop-receive".equals(op) || "receive-close".equals(op))))
            throw new IllegalStateException("Cleanup input is limited to owned sharing/receive retirement");
        for (int attempt = 0; attempt < 40; attempt++) {
            JSONObject row = waitReady(op);
            if (row.getBoolean("tappable")) {
                // Require independent, motion-free layout samples before mapping
                // to native coordinates; a one-shot hit test races sheet entry.
                SystemClock.sleep(100);
                JSONObject fresh = inspect(op);
                if (!sameTapTarget(row, fresh)) continue;
                row = fresh;
                JSONObject p = row.getJSONObject("point");
                float[] xy = screen(row.getJSONObject("viewport"), p.getDouble("x"), p.getDouble("y"));
                JSONObject nativeHit = nativeCrashHit(op, row, xy);
                if (nativeHit != null) row.put("native_hit", nativeHit);
                touch(xy[0], xy[1], "publish".equals(op));
                emit("input", new JSONObject().put("operation", op).put("inspection", row));
                SystemClock.sleep(150);
                return;
            }
            JSONObject viewport = row.getJSONObject("viewport");
            JSONObject r = row.getJSONObject("rect"), clip = row.getJSONObject("clip"), scroll = row.getJSONObject("scroll");
            double left = Math.max(0, scroll.getDouble("left")), right = Math.min(viewport.getDouble("width"), scroll.getDouble("right"));
            double top = Math.max(0, scroll.getDouble("top")), bottom = Math.min(viewport.getDouble("height"), scroll.getDouble("bottom"));
            if (right <= left || bottom - top < 40) throw new IllegalStateException("No visible native scroll area: " + row);
            boolean up = r.getDouble("bottom") > clip.getDouble("bottom");
            boolean down = r.getDouble("top") < clip.getDouble("top");
            if (!up && !down) throw new IllegalStateException("Control obscured by another view: " + row);
            double x = (left + right) / 2, y1 = top + (bottom - top) * (up ? .78 : .22), y2 = top + (bottom - top) * (up ? .22 : .78);
            float[] a = screen(viewport, x, y1), b = screen(viewport, x, y2);
            admitInput();
            long t = SystemClock.uptimeMillis(); motion(a[0], a[1], MotionEvent.ACTION_DOWN, t);
            for (int i = 1; i <= 12; i++) { SystemClock.sleep(20); motion(a[0] + (b[0] - a[0]) * i / 12, a[1] + (b[1] - a[1]) * i / 12, MotionEvent.ACTION_MOVE, t); }
            motion(b[0], b[1], MotionEvent.ACTION_UP, t);
            pause();
        }
        throw new IllegalStateException("Native scrolling did not reveal " + op);
    }
    private void dismissKeyboard() throws Exception {
        admitInput();
        if (Build.VERSION.SDK_INT < 30) throw new IllegalStateException("Receive keyboard proof requires Android 11 or later");
        WebView web = webView();
        if (web == null) throw new IllegalStateException("No foreground WebView for keyboard dismissal");
        AtomicReference<Throwable> failure = new AtomicReference<>();
        runOnMainSync(() -> {
            try {
                WindowInsetsController controller = web.getWindowInsetsController();
                if (controller == null) throw new IllegalStateException("No attached native insets controller");
                // Native system UI only: no DOM focus/value writes or app commands.
                controller.hide(WindowInsets.Type.ime());
            } catch (Throwable error) { failure.set(error); }
        });
        if (failure.get() != null) throw new IllegalStateException(failure.get());
        long limit = Math.min(end, SystemClock.elapsedRealtime() + 5000), hiddenSince = 0;
        while (SystemClock.elapsedRealtime() < limit) {
            check();
            AtomicReference<Boolean> visible = new AtomicReference<>();
            runOnMainSync(() -> {
                WindowInsets insets = web.getRootWindowInsets();
                visible.set(insets == null || insets.isVisible(WindowInsets.Type.ime())
                    || insets.getInsets(WindowInsets.Type.ime()).bottom > 0);
            });
            long now = SystemClock.elapsedRealtime();
            if (Boolean.FALSE.equals(visible.get())) {
                if (hiddenSince == 0) hiddenSince = now;
                if (now - hiddenSince >= 300) {
                    result.put("keyboard_hidden_before_download", true);
                    emit("keyboard-hidden", new JSONObject().put("source", "WindowInsets.Type.ime()").put("visible", false));
                    return;
                }
            } else hiddenSince = 0;
            pause();
        }
        throw new IllegalStateException("Native keyboard did not finish hiding before Download");
    }


    private void receive() throws Exception {
        if (strictReceive()) {
            if (downloadsBefore == null) throw new IllegalStateException("Receive requires readable initial job journal");
            for (Iterator<String> keys = downloadsBefore.keys(); keys.hasNext();) {
                JSONObject job = downloadsBefore.getJSONObject(keys.next());
                if (arg("file_name").equals(job.optString("fileName")) || job.optString("url").endsWith("/" + arg("archive_id")))
                    throw new IllegalStateException("Pre-existing conflicting receive job; no cancellation or reset admitted");
                if (senderHotspot() && ("quic-library".equals(job.optString("transport")) || "localswarm".equals(job.optString("transport"))) &&
                        !("complete".equals(job.optString("status")) || "error".equals(job.optString("status")) || "cancelled".equals(job.optString("status"))))
                    throw new IllegalStateException("Existing active nearby job prevents isolated Stop all cleanup");
            }
        }
        if (senderHotspot()) {
            for (Network network : connectivity().getAllNetworks()) initialNetworks.add(network.getNetworkHandle());
            receiveOwned = true;
            result.put("receiver_route_started", true);
        }
        emit("receiving", new JSONObject().put("message", "Waiting exact peer; sender may now establish route"));
        tap("peer");
        JSONObject route = waitReady(lan() ? "lan-route" : senderHotspot() ? "sender-hotspot-route" : "reverse-route");
        if (senderHotspot()) captureJoinedNetwork();
        emit("catalog-route", route);
        // The host captures current OS AP radio state while this real catalog
        // remains open. No Download is admitted until its nonce-bound ACK.
        while (routeAck.get() == null) pause();
        result.put("route_radio", routeAck.get());
        if (lan()) result.put("lan_route_verified", true).put("lan_route", route);
        else if (senderHotspot()) result.put("sender_hotspot_route_verified", true).put("sender_hotspot_route", route);
        else {
            result.put("product_owned_ap_at_catalog", true).put("reverse_hotspot_verified", true).put("reverse_hotspot", route);
            emit("reverse-hotspot-verified", new JSONObject().put("product_ui", route).put("radio", routeAck.get()));
        }
        tap("filter");
        long keys = SystemClock.uptimeMillis();
        admitInput();
        sendKeySync(new KeyEvent(keys, keys, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0, KeyEvent.META_CTRL_ON));
        sendKeySync(new KeyEvent(keys, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, KeyEvent.KEYCODE_A, 0, KeyEvent.META_CTRL_ON));
        admitInput();
        sendKeyDownUpSync(KeyEvent.KEYCODE_DEL);
        admitInput();
        sendStringSync(arg("file_name"));
        dismissKeyboard();
        tap("download");
        // Queue admission navigates to Downloads. Do not queue the archive again.
        waitReady("open");
        emit("imported_open", new JSONObject().put("file_name", arg("file_name")));
        tap("open");
        JSONObject reader = waitReady("reader");
        JSONObject after;
        while (true) {
            after = state();
            if (arg("archive_id").equals(activeBook(after))) break;
            pause();
        }
        verifyFile(after, true);
        captureReceivedJournal();
        if (strictReceive() && !"matched".equals(result.getJSONObject("received_journal").getString("status")))
            throw new IllegalStateException("Receive lacks exactly one persisted publication job linked to actual managed bytes");
        result.put("readable_ui", true).put("reader", reader).put("reader_visual_review_required", true);
        screenshot("reader.png");
        emit("reader", reader);
        if (senderHotspot()) retireReceive();
    }

    private Object nativeField(Object owner, String name) throws Exception {
        java.lang.reflect.Field field = owner.getClass().getDeclaredField(name);
        field.setAccessible(true);
        return field.get(owner);
    }

    /** Read-only fixed-schema observation of the retained owner, not a plugin command. */
    private JSONObject nativeHotspotState() throws Exception {
        AtomicReference<JSONObject> snapshot = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        runOnMainSync(() -> {
            try {
                Class<?> managerClass = Class.forName("app.tauri.plugin.PluginManager", false, target.getClassLoader());
                // Tauri obfuscates the manager/handle members, including the
                // singleton's field name. Resolve by retained value identity.
                Object manager = null;
                for (java.lang.reflect.Field field : managerClass.getDeclaredFields()) {
                    if (!java.lang.reflect.Modifier.isStatic(field.getModifiers())) continue;
                    field.setAccessible(true);
                    Object value = field.get(null);
                    if (value != null && managerClass.isInstance(value)) { manager = value; break; }
                }
                if (manager == null) throw new IllegalStateException("No retained PluginManager singleton");
                Object registry = null;
                Object plugin = null;
                for (Class<?> type = manager.getClass(); type != null && plugin == null; type = type.getSuperclass()) {
                    for (java.lang.reflect.Field field : type.getDeclaredFields()) {
                        field.setAccessible(true);
                        Object value;
                        try {
                            value = java.lang.reflect.Modifier.isStatic(field.getModifiers()) ? field.get(null) : field.get(manager);
                        } catch (RuntimeException ignored) { continue; }
                        if (!(value instanceof java.util.Map)) continue;
                        for (Object entry : ((java.util.Map<?, ?>)value).values()) {
                            if (entry == null) continue;
                            if ("org.kiwix.mobile.KiwixMobilePlugin".equals(entry.getClass().getName())) {
                                plugin = entry; registry = value; break;
                            }
                            for (Class<?> holder = entry.getClass(); holder != null && plugin == null; holder = holder.getSuperclass()) {
                                for (java.lang.reflect.Field inner : holder.getDeclaredFields()) {
                                    inner.setAccessible(true);
                                    Object candidate;
                                    try {
                                        candidate = java.lang.reflect.Modifier.isStatic(inner.getModifiers()) ? inner.get(null) : inner.get(entry);
                                    } catch (RuntimeException ignored) { continue; }
                                    if (candidate != null && "org.kiwix.mobile.KiwixMobilePlugin".equals(candidate.getClass().getName())) {
                                        plugin = candidate; registry = value; break;
                                    }
                                }
                            }
                            if (plugin != null) break;
                        }
                        if (plugin != null) break;
                    }
                }
                if (plugin == null) throw new IllegalStateException("No retained KiwixMobilePlugin in any manager map");
                if (plugin == null || (publisherPlugin != null && publisherPlugin != plugin)) {
                    StringBuilder seen = new StringBuilder();
                    for (Object handle : ((java.util.Map<?, ?>)registry).values()) {
                        if (seen.length() > 400) break;
                        seen.append(handle == null ? "null" : handle.getClass().getName());
                        if (handle != null) {
                            for (Class<?> type = handle.getClass(); type != null && type != Object.class; type = type.getSuperclass()) {
                                for (java.lang.reflect.Field field : type.getDeclaredFields()) {
                                    field.setAccessible(true);
                                    Object value;
                                    try { value = field.get(handle); } catch (RuntimeException ignored) { continue; }
                                    seen.append('/').append(value == null ? "null" : value.getClass().getName());
                                }
                            }
                        }
                        seen.append(';');
                    }
                    throw new IllegalStateException("Retained native publisher plugin missing or replaced: size="
                        + ((java.util.Map<?, ?>)registry).size() + " entries=" + seen);
                }
                publisherPlugin = plugin;
                Object owner = nativeField(plugin, "hotspotOwner");
                boolean invoke = nativeField(plugin, "hotspotInvoke") != null;
                int systemUi = ((Integer)nativeField(plugin, "hotspotSystemUi")).intValue();
                boolean pending = false, reservation = false, route = false, cancelledStart = false, closing = false;
                long generation = 0;
                if (owner != null) {
                    if (!"org.kiwix.mobile.LocalHotspot".equals(owner.getClass().getName()))
                        throw new IllegalStateException("Unknown retained native hotspot owner schema");
                    pending = nativeField(owner, "pending") != null;
                    reservation = nativeField(owner, "reservation") != null;
                    route = nativeField(owner, "quicInterface") != null || nativeField(owner, "quicLease") != null;
                    cancelledStart = ((Boolean)nativeField(owner, "stopRequested")).booleanValue();
                    closing = ((Boolean)nativeField(owner, "closing")).booleanValue();
                    generation = ((Number)nativeField(owner, "generation")).longValue();
                    if (generation < 0) throw new IllegalStateException("Invalid native hotspot generation");
                }
                // closing only suppresses a stale-loss callback; stop increments
                // generation before close. It is not retained reservation ownership.
                snapshot.set(new JSONObject().put("schema", 1).put("source", "fixed retained KiwixMobilePlugin/LocalHotspot fields on main thread")
                    .put("plugin_identity_bound", true).put("owner_present", owner != null).put("generation", generation)
                    .put("plugin_invoke_pending", invoke).put("system_ui_pending", systemUi != 0)
                    .put("start_pending", pending).put("reservation_active", reservation)
                    .put("quic_route_retained", route).put("cancelled_start_pending", cancelledStart)
                    .put("closing_callback_marker", closing)
                    .put("retired", !invoke && systemUi == 0 && !pending && !reservation && !route && !cancelledStart));
            } catch (Throwable error) { failure.set(error); }
        });
        if (failure.get() != null) throw new IllegalStateException("Native hotspot ownership schema unavailable: " + failure.get(), failure.get());
        return snapshot.get();
    }

    private void publish() throws Exception {
        JSONObject book = findBook(before, arg("archive_id"));
        if (book == null || !book.getBoolean("available") || !arg("title").equals(book.getString("title"))
                || book.getLong("size") != Long.parseLong(arg("size")) || book.getString("installationId").isEmpty())
            throw new IllegalStateException("Expected exact available publication installation missing");
        result.put("installation_id", book.getString("installationId"));
        if (book.getBoolean("managed")) {
            verifyFile(before, false);
            result.put("source_kind", "managed").put("source_bytes_verified", true)
                .put("source_sha256", result.getString("receivedhash"));
        } else {
            if (senderHotspot()) throw new IllegalStateException("Sender-hotspot acceptance requires independently hashed managed source");
            JSONObject provider = book.getJSONObject("sourceReference");
            if (provider.getString("reference").isEmpty()) throw new IllegalStateException("Unmanaged publication requires retained provider authority");
            // Metadata is not a file digest. The ordinary production publisher
            // alone retains/revalidates provider read authority for every read.
            result.put("source_kind", "provider").put("source_bytes_verified", false).put("managed", false)
                .put("installation_id", book.getString("installationId"))
                .put("source_metadata", new JSONObject().put("archive_id", book.getString("id"))
                    .put("title", book.getString("title")).put("size", book.getLong("size"))
                    .put("installation_id", book.getString("installationId")));
        }
        // Ordinary native UI only: LAN Share all never invokes temporary Wi-Fi.
        tap("publish");
        JSONObject publicationUI = waitReady(lan() ? "lan-publication" : "hotspot");
        if (senderHotspot()) {
            String ssid = publicationUI.getString("network_name");
            if (ssid.isEmpty()) throw new IllegalStateException("Actual owned hotspot identity missing");
            result.put("hotspot_network_sha256", hash(ssid.getBytes(StandardCharsets.UTF_8)));
        }
        publicationUI.remove("network_name");
        result.put(lan() ? "lan_publication" : "hotspot", publicationUI);
        result.put("publication", installedPublication());
        emit("publishing", new JSONObject(result.toString()));
        while (publishAck.get() == null) pause();
        result.put("publisher_ready", true).put("route_radio", publishAck.get());
        if (!lan()) result.put("product_owned_ap_at_publish", true);
        publisherReady.set(true);
        screenshot("publishing.png");
        emit("publisher-ready", new JSONObject(result.toString()).put("status", "ready"));
        while (!stopPublish.get() && SystemClock.elapsedRealtime() + 500 < end) {
            JSONObject current = inspect(lan() ? "lan-publication" : "hotspot");
            if (!current.optBoolean("ready") || !canonical(result.getJSONObject("publication")).equals(canonical(installedPublication())))
                throw new IllegalStateException("Owned sharing stopped or changed before requested stop/deadline");
            pause();
        }
        result.put("publication_stop_reason", stopPublish.get() ? "STOP_PUBLISH" : "finite_deadline");
    }

    /** Actual installed private publication identity, independent of the UI label. */
    private JSONObject installedPublication() throws Exception {
        String peer = ownedPublicationText("peer-id", 64);
        String generation = ownedPublicationText("library-generation", 32);
        if (!UUID.fromString(peer).toString().equals(peer) || !generation.matches("[1-9][0-9]*"))
            throw new IllegalStateException("Invalid installed publication peer/generation");
        long value = Long.parseLong(generation);
        if (value <= 0) throw new IllegalStateException("Publication generation is not positive");
        return new JSONObject().put("publisher_id", peer).put("library_generation", value);
    }

    private String ownedPublicationText(String name, int limit) throws Exception {
        File file = new File(root, "localswarm/" + name);
        checkPath(file, true);
        FileDescriptor fd = Os.open(file.getAbsolutePath(), OsConstants.O_RDONLY | OsConstants.O_NOFOLLOW
            | OsConstants.O_CLOEXEC | OsConstants.O_NONBLOCK, 0);
        try {
            StructStat before = Os.fstat(fd);
            if (!OsConstants.S_ISREG(before.st_mode) || before.st_uid != android.os.Process.myUid()
                    || before.st_nlink != 1 || before.st_size <= 0 || before.st_size > limit)
                throw new IllegalStateException("Invalid owned publication identity file");
            FileInputStream input = new FileInputStream(fd);
            ByteArrayOutputStream output = new ByteArrayOutputStream((int)before.st_size);
            byte[] bytes = new byte[limit + 1];
            int count;
            while ((count = input.read(bytes, 0, Math.min(bytes.length, limit + 1 - output.size()))) != -1) {
                check();
                output.write(bytes, 0, count);
                if (output.size() > limit) throw new IllegalStateException("Publication identity grew beyond bound");
            }
            StructStat after = Os.fstat(fd), named = Os.lstat(file.getAbsolutePath());
            if (output.size() != before.st_size || before.st_size != after.st_size || before.st_mtime != after.st_mtime
                    || before.st_ctime != after.st_ctime || before.st_ino != named.st_ino || before.st_dev != named.st_dev
                    || named.st_uid != before.st_uid || named.st_nlink != 1)
                throw new IllegalStateException("Publication identity changed while reading");
            checkPath(file, true);
            return new String(output.toByteArray(), StandardCharsets.UTF_8);
        } finally { Os.close(fd); }
    }

    private void verifyFile(JSONObject state, boolean requireActive) throws Exception {
        JSONObject book = findBook(state, arg("archive_id"));
        if (book == null || !book.optBoolean("available") || !book.optBoolean("managed")) throw new IllegalStateException("Expected available managed installation missing");
        if (!arg("title").equals(book.getString("title"))) throw new IllegalStateException("Installed title mismatch");
        JSONObject actual = digest(new File(book.getString("path")), true);
        long expected = Long.parseLong(arg("size"));
        if (actual.getLong("size") != expected || book.getLong("size") != expected || !actual.getString("sha256").equals(arg("sha256"))) throw new IllegalStateException("Actual managed received bytes differ: " + actual);
        String active = activeBook(state);
        if (requireActive && !arg("archive_id").equals(active)) throw new IllegalStateException("Expected UUID is not active reader");
        result.put("receivedhash", actual.getString("sha256")).put("receivedsize", actual.getLong("size"))
            .put("managed", true).put("activebookid", active).put("installation_id", book.getString("installationId"))
            .put("managed_path", book.getString("path"));
        emit("verified_bytes", actual.put("archive_id", arg("archive_id")).put("activebookid", active));
    }

    /** Evidence only: journal timestamps/claims never replace the actual file hash. */
    private void captureReceivedJournal() {
        JSONObject evidence = new JSONObject();
        try {
            JSONArray rows = new JSONArray();
            evidence.put("status", "unmatched").put("new_rows", rows);
            result.put("received_journal", evidence);
            JSONObject after = profileRecord("downloads/downloads.json");
            if (downloadsBefore == null) throw new IllegalStateException("Initial download journal unavailable; cannot prove a new job");
            JSONObject installedState = strictReceive() ? state() : null;
            JSONObject installedBook = strictReceive() ? findBook(installedState, arg("archive_id")) : null;
            if (strictReceive()) preserveJobs(after);
            int matches = 0;
            for (Iterator<String> keys = after.keys(); keys.hasNext();) {
                String id = keys.next();
                if (downloadsBefore.has(id)) continue;
                JSONObject row = new JSONObject().put("journal_key", id);
                rows.put(row);
                try {
                    JSONObject source = after.getJSONObject(id);
                    for (String key : new String[]{"id", "fileName", "url", "status", "transport", "error", "importError", "destination", "downloaded", "total", "sha256", "createdAt", "updatedAt", "publication"})
                        row.put(key, source.has(key) ? source.get(key) : JSONObject.NULL);
                    JSONObject publication = source.optJSONObject("publication");
                    String url = source.getString("url");
                    String[] parts = url.split("/", -1);
                    boolean ownedPublisher = !strictReceive() || (parts.length == 6 && "kiwix-library:".equals(parts[0])
                        && parts[1].isEmpty() && "public".equals(parts[2])
                        && parts[3].matches("[0-9a-f]{32}")
                        && arg("sender_device").equals(parts[4]) && arg("archive_id").equals(parts[5]));
                    boolean consumed = false;
                    if (strictReceive()) {
                        JSONArray receipts = installedState.getJSONArray("consumedAcquisitions");
                        for (int i = 0; i < receipts.length(); i++) if (id.equals(receipts.getString(i))) consumed = true;
                        row.put("consumed_acquisition", consumed).put("installation_id",
                            installedBook == null ? JSONObject.NULL : installedBook.getString("installationId"));
                    }
                    boolean match = id.equals(source.getString("id")) && arg("file_name").equals(source.getString("fileName"))
                        && url.startsWith("kiwix-library://public/") && url.endsWith("/" + arg("archive_id")) && ownedPublisher
                        && "complete".equals(source.getString("status"))
                        && source.getLong("downloaded") == result.getLong("receivedsize")
                        && source.getLong("total") == result.getLong("receivedsize")
                        && result.getString("managed_path").equals(source.getString("destination"))
                        && publication != null && publication.getLong("size") == result.getLong("receivedsize")
                        && publication.getString("sha256").equals(result.getString("receivedhash"))
                        && source.get("createdAt") instanceof String && !source.getString("createdAt").isEmpty()
                        && source.get("updatedAt") instanceof String && !source.getString("updatedAt").isEmpty();
                    if (strictReceive()) match = match && "quic-library".equals(source.optString("transport"))
                        && source.isNull("error") && source.isNull("importError") && consumed && installedBook != null
                        && installedBook.getBoolean("managed") && installedBook.getBoolean("available")
                        && result.getString("installation_id").equals(installedBook.getString("installationId"))
                        && result.getString("managed_path").equals(installedBook.getString("path"));
                    row.put("matches_independent_received_bytes", match);
                    if (match) { matches++; evidence.put("matched_job", row); }
                } catch (Exception error) {
                    row.put("matches_independent_received_bytes", false).put("error", error.toString());
                }
            }
            evidence.put("match_count", matches).put("status", matches == 1 ? "matched" : "unmatched");
        } catch (Exception error) {
            try { evidence.put("status", "error").put("error", error.toString()); result.put("received_journal", evidence); }
            catch (Exception ignored) { }
        }
        emit("received-journal", evidence);
    }

    private void preserveJobs(JSONObject after) throws Exception {
        if (downloadsBefore == null) throw new IllegalStateException("Missing original job journal");
        for (Iterator<String> keys = downloadsBefore.keys(); keys.hasNext();) {
            String id = keys.next();
            if (!after.has(id) || !canonical(downloadsBefore.get(id)).equals(canonical(after.get(id))))
                throw new IllegalStateException("Pre-existing receive job changed during acceptance");
        }
        result.put("preserved_before_jobs", true);
    }

    private JSONObject readJobs() throws Exception {
        try { return profileRecord("downloads/downloads.json"); }
        catch (android.system.ErrnoException error) {
            if (error.errno == OsConstants.ENOENT) return new JSONObject();
            throw error;
        }
    }

    private JSONObject state() throws Exception {
        return profileRecord("state.json");
    }

    /** Fixed own-profile native producer only; never accepts a caller path. */
    private JSONObject readCrashReport() throws Exception {
        File file = new File(root, "crash/pending.json");
        FileDescriptor fd;
        try {
            checkPath(file, true);
            fd = Os.open(file.getAbsolutePath(), OsConstants.O_RDONLY | OsConstants.O_NOFOLLOW
                | OsConstants.O_CLOEXEC | OsConstants.O_NONBLOCK, 0);
        } catch (android.system.ErrnoException error) {
            if (error.errno == OsConstants.ENOENT) return new JSONObject().put("present", false);
            throw error;
        }
        try {
            StructStat before = Os.fstat(fd);
            if (!OsConstants.S_ISREG(before.st_mode) || before.st_uid != android.os.Process.myUid()
                    || before.st_nlink != 1 || before.st_size <= 0 || before.st_size > 32768)
                throw new IllegalStateException("Invalid owned bounded native crash record");
            ByteArrayOutputStream output = new ByteArrayOutputStream((int)before.st_size);
            FileInputStream input = new FileInputStream(fd);
            byte[] buffer = new byte[4096];
            int count;
            while ((count = input.read(buffer, 0, Math.min(buffer.length, 32769 - output.size()))) != -1) {
                check();
                output.write(buffer, 0, count);
                if (output.size() > 32768) throw new IllegalStateException("Native crash record grew beyond bound");
            }
            StructStat after = Os.fstat(fd), named = Os.lstat(file.getAbsolutePath());
            if (output.size() != before.st_size || before.st_size != after.st_size
                    || before.st_mtime != after.st_mtime || before.st_ctime != after.st_ctime
                    || before.st_ino != named.st_ino || before.st_dev != named.st_dev
                    || named.st_uid != before.st_uid || named.st_nlink != 1)
                throw new IllegalStateException("Native crash record changed while capturing");
            checkPath(file, true);
            byte[] bytes = output.toByteArray();
            JSONObject record;
            try { record = new JSONObject(new String(bytes, StandardCharsets.UTF_8)); }
            catch (Exception error) { throw new IllegalStateException("Invalid native crash JSON"); }
            Object schema = record.opt("schema");
            if (!(schema instanceof Integer || schema instanceof Long) || ((Number)schema).longValue() != 1)
                throw new IllegalStateException("Unsupported native crash schema");
            // The identity-verified APK's crash producers redact before publishing
            // this exact private path. Retain the original bytes, not a reserialization.
            return new JSONObject().put("present", true).put("native_redacted", true)
                .put("size", bytes.length).put("sha256", hash(bytes)).put("base64", Base64.encodeToString(bytes, Base64.NO_WRAP));
        } finally { Os.close(fd); }
    }

    private void assertCrashUnchanged() throws Exception {
        JSONObject now = readCrashReport();
        if (crashSnapshot.getBoolean("present") != now.getBoolean("present")
                || (now.getBoolean("present") && !crashSnapshot.getString("sha256").equals(now.getString("sha256"))))
            throw new IllegalStateException("Pending native crash identity changed; no further input admitted");
    }

    private void closeReviewedCrash() throws Exception {
        if (crashRecord.has("reportedAt")) throw new IllegalStateException("Reviewed record is already reported; startup prompt cannot be assumed");
        // Wait for the actual prompt; navigation alone is not cold-start readiness.
        waitReady("crash-prompt");
        assertCrashUnchanged();
        tap("crash-view");
        JSONObject displayed = waitReady("crash-record");
        if (!canonical(crashRecord).equals(canonical(displayed.getJSONObject("record"))))
            throw new IllegalStateException("Displayed startup crash report differs from reviewed owned report");
        screenshot("crash-before-close.png");
        assertCrashUnchanged();
        tap("crash-close");
        JSONObject closed = waitReady("crash-closed");
        assertCrashUnchanged();
        screenshot("crash-after-close.png");
        JSONObject evidence = new JSONObject().put("sha256", crashSnapshot.getString("sha256"))
            .put("ordinary_ui_close", true).put("report_preserved", true).put("inspection", closed);
        result.put("reviewed_crash_closed", evidence);
        emit("reviewed-crash-closed", evidence);
    }

    private JSONObject profileRecord(String name) throws Exception {
        File file = new File(root, name);
        checkPath(file, true);
        FileDescriptor fd = Os.open(file.getAbsolutePath(), OsConstants.O_RDONLY | OsConstants.O_NOFOLLOW | OsConstants.O_CLOEXEC, 0);
        try {
            FileInputStream input = new FileInputStream(fd);
            StructStat stat = Os.fstat(fd);
            if (!OsConstants.S_ISREG(stat.st_mode) || stat.st_size > 64 * 1024 * 1024) throw new IllegalStateException("Invalid profile file");
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buf = new byte[32768]; int n;
            while ((n = input.read(buf)) != -1) { check(); output.write(buf, 0, n); }
            return new JSONObject(new String(output.toByteArray(), StandardCharsets.UTF_8));
        } finally { Os.close(fd); }
    }

    private void checkPath(File file, boolean contained) throws Exception {
        String absolute = file.getAbsolutePath(), canonical = file.getCanonicalPath();
        if (!absolute.equals(canonical)) throw new IllegalStateException("Noncanonical or symlink file path");
        if (contained && !canonical.startsWith(root.getCanonicalPath() + File.separator)) throw new IllegalStateException("File outside managed private profile");
        File current = file;
        while (current != null) {
            if (OsConstants.S_ISLNK(Os.lstat(current.getAbsolutePath()).st_mode)) throw new IllegalStateException("Symlink path component");
            current = current.getParentFile();
        }
    }

    private JSONObject digest(File file, boolean contained) throws Exception {
        checkPath(file, contained);
        FileDescriptor fd = Os.open(file.getAbsolutePath(), OsConstants.O_RDONLY | OsConstants.O_NOFOLLOW | OsConstants.O_CLOEXEC, 0);
        try {
            FileInputStream input = new FileInputStream(fd);
            StructStat before = Os.fstat(fd);
            if (!OsConstants.S_ISREG(before.st_mode)) throw new IllegalStateException("Not a regular file");
            MessageDigest hash = MessageDigest.getInstance("SHA-256"); byte[] buf = new byte[131072]; long size = 0; int n;
            while ((n = input.read(buf)) != -1) { check(); hash.update(buf, 0, n); size += n; }
            StructStat after = Os.fstat(fd), named = Os.lstat(file.getAbsolutePath());
            if (size != before.st_size || before.st_size != after.st_size || before.st_mtime != after.st_mtime || before.st_ino != named.st_ino || before.st_dev != named.st_dev) throw new IllegalStateException("File changed while hashing");
            checkPath(file, contained);
            return new JSONObject().put("size", size).put("sha256", hex(hash.digest())).put("path", file.getAbsolutePath());
        } finally { Os.close(fd); }
    }

    private JSONObject findBook(JSONObject state, String uuid) throws Exception {
        JSONArray books = state.getJSONArray("books"); JSONObject found = null;
        for (int i = 0; i < books.length(); i++) {
            JSONObject book = books.getJSONObject(i);
            if (uuid.equals(book.getString("id"))) {
                if (found != null) throw new IllegalStateException("Duplicate archive UUID in profile");
                found = book;
            }
        }
        return found;
    }

    private String activeBook(JSONObject state) throws Exception {
        JSONObject windows = state.optJSONObject("windows"), window = windows == null ? null : windows.optJSONObject("main");
        if (window == null) return "";
        JSONArray tabs = window.getJSONArray("tabs"); String id = window.optString("activeTab");
        for (int i = 0; i < tabs.length(); i++) if (id.equals(tabs.getJSONObject(i).getString("id"))) return tabs.getJSONObject(i).optString("bookId");
        return "";
    }

    private void preserve(JSONObject after) throws Exception {
        if (!canonical(before.getJSONObject("settings")).equals(canonical(after.getJSONObject("settings")))) throw new IllegalStateException("Settings changed during automation");
        JSONArray books = before.getJSONArray("books");
        for (int i = 0; i < books.length(); i++) {
            JSONObject old = books.getJSONObject(i), now = findBook(after, old.getString("id"));
            if (now == null || !canonical(old).equals(canonical(now))) throw new IllegalStateException("Pre-existing archive metadata changed: " + old.getString("id"));
        }
        JSONObject windows = before.optJSONObject("windows"), newWindows = after.optJSONObject("windows");
        if (windows != null) for (Iterator<String> keys = windows.keys(); keys.hasNext();) {
            String id = keys.next(); JSONObject old = windows.getJSONObject(id), now = newWindows == null ? null : newWindows.optJSONObject(id);
            if (now == null) throw new IllegalStateException("Old window removed");
            preserveTabs(old.getJSONArray("tabs"), now.getJSONArray("tabs"));
        }
        preserveTabs(before.optJSONArray("tabs"), after.optJSONArray("tabs"));
        if (downloadsBefore != null) preserveJobs(readJobs());
        JSONArray consumed = before.optJSONArray("consumedAcquisitions"), currentConsumed = after.optJSONArray("consumedAcquisitions");
        if (consumed != null) for (int i = 0; i < consumed.length(); i++) {
            boolean retained = false;
            if (currentConsumed != null) for (int j = 0; j < currentConsumed.length(); j++)
                if (consumed.getString(i).equals(currentConsumed.getString(j))) retained = true;
            if (!retained) throw new IllegalStateException("Pre-existing consumed acquisition receipt removed");
        }
        result.put("preserved_before_books_settings_tabs", true);
    }

    private void preserveTabs(JSONArray old, JSONArray now) throws Exception {
        if (old == null) return;
        for (int i = 0; i < old.length(); i++) {
            JSONObject tab = old.getJSONObject(i); JSONObject match = null;
            if (now != null) for (int j = 0; j < now.length(); j++) if (tab.getString("id").equals(now.getJSONObject(j).getString("id"))) match = now.getJSONObject(j);
            if (match == null || !canonical(tab).equals(canonical(match))) throw new IllegalStateException("Pre-existing tab changed: " + tab.getString("id"));
        }
    }

    private String canonical(Object value) throws Exception {
        if (value instanceof JSONObject) {
            JSONObject object = (JSONObject)value; List<String> keys = new ArrayList<>();
            for (Iterator<String> it = object.keys(); it.hasNext();) keys.add(it.next());
            Collections.sort(keys); StringBuilder text = new StringBuilder("{");
            for (String key : keys) text.append(JSONObject.quote(key)).append(':').append(canonical(object.get(key))).append(',');
            return text.append('}').toString();
        }
        if (value instanceof JSONArray) {
            StringBuilder text = new StringBuilder("["); JSONArray array = (JSONArray)value;
            for (int i = 0; i < array.length(); i++) text.append(canonical(array.get(i))).append(',');
            return text.append(']').toString();
        }
        if (value == null || value == JSONObject.NULL) return "null";
        return value instanceof String ? JSONObject.quote((String)value) : value.toString();
    }

    private byte[] readAsset(String name) throws Exception {
        try (java.io.InputStream input = getContext().getAssets().open(name); ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buf = new byte[8192]; int n; while ((n = input.read(buf)) != -1) out.write(buf, 0, n); return out.toByteArray();
        }
    }

    private static String hex(byte[] bytes) {
        char[] out = new char[bytes.length * 2];
        char[] digits = "0123456789abcdef".toCharArray();
        for (int i = 0; i < bytes.length; i++) {
            int value = bytes[i] & 255;
            out[i * 2] = digits[value >>> 4];
            out[i * 2 + 1] = digits[value & 15];
        }
        return new String(out);
    }
    private static String hash(byte[] bytes) throws Exception { return hex(MessageDigest.getInstance("SHA-256").digest(bytes)); }

    private android.app.UiAutomation ui;
    private android.app.UiAutomation automation() {
        if (ui == null) {
            ui = getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
            android.accessibilityservice.AccessibilityServiceInfo info = ui.getServiceInfo();
            info.flags |= android.accessibilityservice.AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
                | android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
            ui.setServiceInfo(info);
        }
        return ui;
    }

    private void screenshot(String name) throws Exception {
        Bitmap image = automation().takeScreenshot();
        if (image == null) throw new IllegalStateException("Actual display screenshot unavailable");
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        try { if (!image.compress(Bitmap.CompressFormat.PNG, 100, out)) throw new IllegalStateException("Screenshot encoding failed"); }
        finally { image.recycle(); }
        byte[] bytes = out.toByteArray(); String encoded = Base64.encodeToString(bytes, Base64.NO_WRAP);
        int chunks = (encoded.length() + 23999) / 24000;
        String imageHash = hash(bytes);
        for (int i = 0; i < chunks; i++) emit("screenshot", new JSONObject().put("name", name).put("index", i).put("chunks", chunks)
            .put("sha256", imageHash).put("base64", encoded.substring(i * 24000, Math.min(encoded.length(), (i + 1) * 24000))));
        if (result != null) result.put("screenshot", name).put("screenshot_sha256", imageHash);
    }

    /** Only permission-controller dialogs explicitly naming this app and Nearby/location. */
    private void allowScopedPermission() throws Exception {
        if (cleaning) return; // Cleanup cannot approve fresh permissions.
        AccessibilityNodeInfo node = automation().getRootInActiveWindow();
        if (node == null) return;
        String pkg = node.getPackageName() == null ? "" : node.getPackageName().toString();
        if (senderHotspot() && "receive".equals(arg("mode")) && pkg.equals("com.android.settings")) {
            joinConsent(node, false);
            return;
        }
        if (!pkg.equals("com.android.permissioncontroller") && !pkg.equals("com.google.android.permissioncontroller")
                && !(Build.VERSION.SDK_INT == 28 && pkg.equals("com.android.packageinstaller"))) {
            if (!pkg.equals(TARGET) && !pkg.isEmpty() && !pkg.contains("inputmethod"))
                throw new IllegalStateException("Unknown native consent/foreground package: " + pkg);
            return;
        }
        List<AccessibilityNodeInfo> nodes = new ArrayList<>(); collectNodes(node, nodes);
        String label = target.getApplicationInfo().loadLabel(target.getPackageManager()).toString();
        boolean named = false, scoped = false; List<AccessibilityNodeInfo> allowed = new ArrayList<>();
        for (AccessibilityNodeInfo item : nodes) {
            String text = item.getText() == null ? "" : item.getText().toString();
            if (text.contains(label)) named = true;
            if (text.toLowerCase(java.util.Locale.ROOT).matches("(?s).*(nearby devices|device.*location|location).*")) scoped = true;
            String id = item.getViewIdResourceName();
            if (item.isVisibleToUser() && item.isEnabled() && id != null && (id.endsWith(":id/permission_allow_button") || id.endsWith(":id/permission_allow_foreground_only_button"))) allowed.add(item);
        }
        if (!named || !scoped || allowed.size() != 1) throw new IllegalStateException("Unadmitted native permission dialog; no automatic approval");
        Rect rect = new Rect(); allowed.get(0).getBoundsInScreen(rect);
        if (rect.isEmpty()) throw new IllegalStateException("Permission control has no bounds");
        touch(rect.centerX(), rect.centerY(), false);
        emit("permission", new JSONObject().put("package", TARGET).put("scope", "Nearby/location foreground").put("control", allowed.get(0).getViewIdResourceName()));
    }

    /** AOSP NetworkRequestSingleSsidDialogFragment: exact app/name/message/control IDs. */
    private void joinConsent(AccessibilityNodeInfo rootNode, boolean cancel) throws Exception {
        if (!receiveOwned || initialAck.get() == null) throw new IllegalStateException("Unowned Wi-Fi consent");
        List<AccessibilityNodeInfo> nodes = new ArrayList<>(); collectNodes(rootNode, nodes);
        String label = target.getApplicationInfo().loadLabel(target.getPackageManager()).toString();
        boolean title = false, named = false, network = false;
        List<AccessibilityNodeInfo> buttons = new ArrayList<>();
        for (AccessibilityNodeInfo node : nodes) {
            if (!node.isVisibleToUser()) continue;
            String text = node.getText() == null ? "" : node.getText().toString();
            String id = node.getViewIdResourceName();
            if ("com.android.settings:id/network_request_title_text".equals(id) && !text.isEmpty()) title = true;
            if ("com.android.settings:id/network_request_summary_text".equals(id) && text.contains(label)) named = true;
            if ("android:id/message".equals(id) &&
                    hash(text.getBytes(StandardCharsets.UTF_8)).equals(initialAck.get().getString("sender_network_sha256"))) network = true;
            if ((cancel ? "android:id/button3" : "android:id/button1").equals(id) &&
                    (cancel ? "Cancel" : "Connect").equals(text) && node.isEnabled() && node.isClickable()) buttons.add(node);
        }
        if (!title || !named || !network || buttons.size() != 1)
            throw new IllegalStateException("Unknown Wi-Fi join consent schema; no automatic approval");
        Rect bounds = new Rect(); buttons.get(0).getBoundsInScreen(bounds);
        if (bounds.isEmpty()) throw new IllegalStateException("Wi-Fi consent button has no bounds");
        touch(bounds.centerX(), bounds.centerY(), false);
        emit(cancel ? "join-consent-cancelled" : "join-consent", new JSONObject().put("package", "com.android.settings")
            .put("sender_network_sha256", initialAck.get().getString("sender_network_sha256")));
    }
    private void collectNodes(AccessibilityNodeInfo node, List<AccessibilityNodeInfo> list) {
        list.add(node);
        for (int i = 0; i < node.getChildCount(); i++) { AccessibilityNodeInfo child = node.getChild(i); if (child != null) collectNodes(child, list); }
    }
}
