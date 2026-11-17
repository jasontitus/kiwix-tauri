import Foundation
import Tauri
import UIKit
import UniformTypeIdentifiers
import WebKit
import PdfExportBridge

private struct PickerLabels: Decodable {
  let title: String
  let cancel: String
  let copying: String?
}
private struct ImportArgs: Decodable { let destinationDirectory: String; let labels: PickerLabels }
private struct ExternalPickArgs: Decodable { let persistent: Bool; let folder: Bool; let labels: PickerLabels }
private struct ExternalReferenceArgs: Decodable { let reference: String }
private struct ExternalLeaseArgs: Decodable { let lease: String }
private struct ExternalPickReply: Encodable { let selection: ExternalSelection?; let cancelled: Bool }
private struct IncomingOriginalArgs: Decodable { let uri: String; let persistent: Bool; let labels: PickerLabels }
private struct IncomingOriginalReleaseArgs: Decodable { let uris: [String] }
private struct IncomingArgs: Decodable {
  let uris: [String]
  let destinationDirectory: String
  let labels: PickerLabels
}
private struct TextArgs: Decodable { let maxBytes: Int; let labels: PickerLabels }
private struct ExportArgs: Decodable {
  let path: String
  let mimeType: String
  let suggestedName: String
  let open: Bool
  let labels: PickerLabels
  let pdfAuthority: String?
}
private struct PdfCancelArgs: Decodable { let pdfAuthority: String }
private struct URLArgs: Decodable { let url: String }
private struct CheckpointArgs: Decodable { let token: Int }
private struct BeginCheckpointArgs: Decodable { let transition: Int }
private struct ReaderAudioArgs: Decodable {
  let active: Bool
  /// The reader surface left the screen; the latched session must not outlive it.
  let release: Bool?
}
private struct LocalTransferArgs: Decodable { let active: Bool }
private struct CrashCapturePolicyArgs: Decodable { let enabled: Bool }
private enum CheckpointReservation {
  case active(Int)
  case unavailable(Int)
  case expired(Int)
  var transition: Int {
    switch self {
    case .active(let token), .unavailable(let token), .expired(let token): return token
    }
  }
}
private struct ImportReply: Encodable {
  let paths: [String]
  let cancelled: Bool
  let directory: String?
}
private struct TextReply: Encodable { let text: String?; let cancelled: Bool }
private struct CancelReply: Encodable { let cancelled: Bool }
private enum OperationKind {
  case archives(ImportArgs)
  case directory(ImportArgs)
  case external(ExternalPickArgs)
  case externalBatch(ExternalPickArgs)
  case text(TextArgs)
  case export(ExportArgs)
}

private final class DocumentOperation {
  let invoke: Invoke
  let kind: OperationKind
  let cancellation = WorkCancellation()
  weak var scene: UIScene?
  var controller: UIViewController?
  var progress: CopyProgressController?
  var working = false
  var settled = false
  var exportScratch: ExportScratch?
  init(invoke: Invoke, kind: OperationKind, scene: UIScene) {
    self.invoke = invoke; self.kind = kind; self.scene = scene
  }
}

private final class CopyProgressController: UIViewController {
  private let heading: String
  private let copying: String
  private let cancelTitle: String
  private let cancelWork: () -> Void
  private let detail = UILabel()
  private let spinner = UIActivityIndicatorView(style: .large)
  init(labels: PickerLabels, cancel: @escaping () -> Void) {
    heading = labels.title; copying = labels.copying ?? labels.title
    cancelTitle = labels.cancel; cancelWork = cancel
    super.init(nibName: nil, bundle: nil)
    modalPresentationStyle = .formSheet
    isModalInPresentation = true
  }
  required init?(coder: NSCoder) { return nil }
  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    let title = UILabel(); title.text = heading; title.font = .preferredFont(forTextStyle: .headline)
    title.numberOfLines = 0
    let caption = UILabel(); caption.text = copying; caption.numberOfLines = 0
    caption.font = .preferredFont(forTextStyle: .body)
    detail.numberOfLines = 3; detail.font = .preferredFont(forTextStyle: .footnote)
    detail.lineBreakMode = .byTruncatingMiddle
    let cancel = UIButton(type: .system); cancel.setTitle(cancelTitle, for: .normal)
    cancel.addTarget(self, action: #selector(cancelPressed), for: .touchUpInside)
    let stack = UIStackView(arrangedSubviews: [title, caption, spinner, detail, cancel])
    stack.axis = .vertical; stack.spacing = 20; stack.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
      stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
      stack.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerYAnchor),
      stack.topAnchor.constraint(greaterThanOrEqualTo: view.safeAreaLayoutGuide.topAnchor, constant: 24),
      stack.bottomAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24)
    ])
    spinner.startAnimating()
    preferredContentSize = CGSize(width: 460, height: 340)
  }
  @objc private func cancelPressed() { cancelWork() }
  func update(name: String, bytes: Int64) {
    detail.text = name + "\n" + ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
  }
}

final class KiwixMobilePlugin: Plugin, UIDocumentPickerDelegate, UIAdaptivePresentationControllerDelegate {
  private lazy var networkPolicy = MobileNetworkPolicy { [weak self] state in
    self?.trigger("network-policy-changed", data: state)
  }
  private let worker = DispatchQueue(label: "org.kiwix.mobile.documents", qos: .userInitiated)
  private var operation: DocumentOperation?
  private var observers: [NSObjectProtocol] = []
  private var checkpoints: [Int: UIBackgroundTaskIdentifier] = [:]
  private var checkpointCleanup: [Int: DispatchWorkItem] = [:]
  private var retiringCheckpoints: Set<Int> = []
  private var activeLocalTransfer = false
  private var transferIdleTimer: MobileIdleTimerLease?
  private var pendingCheckpoint: CheckpointReservation?
  private var nextCheckpoint = 1
  private weak var webView: WKWebView?
  private var lifecycleSuspended = false
  private var lifecycleInactive = false
  /// Export scratch directories whose cleanup failed. Each value owns two
  /// descriptors, so the set is bounded and retried instead of only growing.
  private var retainedExports: [ExportScratch] = []
  private var retainedExportAttempts = 0
  private var retainedExportRetry: DispatchWorkItem?
  private var drainingRetainedExports = false
  private static let retainedExportLimit = 16
  private static let retainedExportAttemptLimit = 4
  private let externalDocuments = ExternalDocuments()
  private var speech: NativeSpeech?
  private var engineNowPlaying: AnyObject?
  private weak var speechScene: UIScene?
  private var nearbyRadio: NearbyBluetooth?
  private var nearbyAssociation: NearbyJoin?
  private let nearbyRoutes = NearbyRoutes()
  /// Owns the playback category for archive media and read-aloud. Created on
  /// first use, but `load(webview:)` installs the category before playback.
  private lazy var readerAudio = ReaderAudioSession { [weak self] state in
    self?.trigger("reader-audio", data: state)
  }
  private lazy var hardwareShortcuts: HardwareShortcutRouter = HardwareShortcutRouter { [weak self] action in
    guard let self = self else { return }
    guard self.hardwareShortcuts.hostController === self.manager.viewController else {
      self.bindHardwareShortcuts()
      return
    }
    self.trigger("hardware-shortcut", data: ["action": action])
  }

  override init() {
    super.init()
    let center = NotificationCenter.default
    observers.append(center.addObserver(forName: UIContentSizeCategory.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
      self?.publishDynamicType()
    })
    observers.append(center.addObserver(forName: UIWindow.didBecomeKeyNotification, object: nil, queue: .main) { [weak self] notification in
      guard let self = self, let window = notification.object as? UIWindow,
            self.manager.viewController?.viewIfLoaded?.window === window else { return }
      self.bindHardwareShortcuts()
    })
    observers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
      self?.resumeActiveScene()
    })
    observers.append(center.addObserver(forName: UIScene.didActivateNotification, object: nil, queue: .main) { [weak self] notification in
      guard let self = self, let scene = notification.object as? UIScene,
            self.hostScene === scene else { return }
      self.resumeActiveScene()
    })
    observers.append(center.addObserver(forName: UIScene.willDeactivateNotification, object: nil, queue: .main) { [weak self] notification in
      guard let self = self, let scene = notification.object as? UIScene,
            self.hostScene === scene else { return }
      self.pauseCaptureForInactivity()
    })
    observers.append(center.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
      self?.pauseCaptureForInactivity()
    })
    observers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
      self?.revokeCamera(reason: "background")
      self?.retireNearby()
    })
    observers.append(center.addObserver(forName: UIScene.didDisconnectNotification, object: nil, queue: .main) { [weak self] notification in
      guard let self = self, let scene = notification.object as? UIScene else { return }
      self.nearbyRadio?.retire(scene: scene)
      self.nearbyAssociation?.retire(scene: scene)
      if self.hostScene === scene { self.nearbyRoutes.retire() }
      guard self.hostScene === scene || self.speechScene === scene ||
            self.hardwareShortcuts.hostScene === scene || self.operation?.scene === scene else { return }
      self.speech?.cancelAll(error: "SPEECH_BACKGROUND")
      if self.operation?.scene === scene { self.cancelCurrent() }
      if self.hardwareShortcuts.hostScene === scene {
        self.hardwareShortcuts.unbind()
      }
      self.expireAllCheckpoints()
      self.setLocalTransferActive(false)
    })
    observers.append(center.addObserver(forName: UIScene.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] notification in
      guard let self = self, let scene = notification.object as? UIScene else { return }
      if self.hostScene === scene { self.revokeCamera(reason: "background") }
      self.nearbyRadio?.retire(scene: scene)
      self.nearbyAssociation?.retire(scene: scene)
      if self.hostScene === scene { self.nearbyRoutes.retire() }
      if self.operation?.scene === scene { self.cancelCurrent() }
    })
    observers.append(center.addObserver(forName: UIApplication.willTerminateNotification, object: nil, queue: .main) { [weak self] _ in
      self?.speech?.shutdown()
      self?.readerAudio.shutdown()
      self?.retireNearby()
      self?.cancelCurrent(); self?.expireAllCheckpoints()
      self?.setLocalTransferActive(false)
    })
  }

  deinit {
    observers.forEach { NotificationCenter.default.removeObserver($0) }
    operation?.cancellation.cancel()
    speech?.shutdown()
    let audio = readerAudio
    let radio = nearbyRadio
    let association = nearbyAssociation
    let routes = nearbyRoutes
    // UIKit background task APIs are main-thread only. Capture values, never self.
    let tasks = Array(checkpoints.values)
    let transferIdleTimer = self.transferIdleTimer
    DispatchQueue.main.async {
      audio.shutdown()
      radio?.retire(); association?.retire(); routes.retire()
      tasks.forEach { UIApplication.shared.endBackgroundTask($0) }
      transferIdleTimer?.release()
    }
  }

  override func load(webview: WKWebView) {
    CrashIntake.install()
    webView = webview
    if Thread.isMainThread {
      retireNearby()
      readerAudio.installCategory()
      bindHardwareShortcuts()
      publishDynamicType()
    } else {
      DispatchQueue.main.async { [weak self] in
        self?.retireNearby()
        self?.readerAudio.installCategory()
        self?.bindHardwareShortcuts()
        self?.publishDynamicType()
      }
    }
  }

  private func bindHardwareShortcuts() {
    hardwareShortcuts.bind(controller: manager.viewController, webView: webView)
  }
  private func dynamicTypeSnapshot() -> JSObject {
    let category = UIApplication.shared.preferredContentSizeCategory
    let traits = UITraitCollection(preferredContentSizeCategory: category)
    let scale = UIFontMetrics.default.scaledValue(for: 17, compatibleWith: traits) / 17
    return ["category": category.rawValue, "scale": Double(scale)]
  }

  private func publishDynamicType() {
    trigger("dynamic-type", data: dynamicTypeSnapshot())
  }

  @objc public func dynamicTypeState(_ invoke: Invoke) {
    DispatchQueue.main.async { invoke.resolve(self.dynamicTypeSnapshot()) }
  }

  @objc public func crashCapturePolicy(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CrashCapturePolicyArgs.self)
    CrashIntake.setEnabled(args.enabled)
    invoke.resolve()
  }


  private var hostScene: UIScene? {
    webView?.window?.windowScene ?? manager.viewController?.viewIfLoaded?.window?.windowScene
  }

  private func resumeActiveScene() {
    guard hostScene?.activationState == .foregroundActive else { return }
    // A fast foreground return can supersede Rust's queued background
    // transition before it adopts the reservation. Do not retain that unused
    // UIKit task until expiry or hand it to a later transition.
    if case .active(let token) = pendingCheckpoint {
      releaseCheckpoint(token)
    }
    pendingCheckpoint = nil
    bindHardwareShortcuts()
    publishDynamicType()
    guard lifecycleInactive || lifecycleSuspended else { return }
    lifecycleInactive = false
    lifecycleSuspended = false
    updateTransferIdleTimer()
    // A transient permission prompt preserves its pending authorization session.
    // Genuine background retirement is handled separately by mobile-suspended.
    trigger("mobile-resumed", data: [:])
  }

  private func pauseCaptureForInactivity() {
    stopCameraCapture()
    if !lifecycleInactive {
      lifecycleInactive = true
      trigger("mobile-inactive", data: [:])
    }
  }

  private func setLocalTransferActive(_ active: Bool) {
    activeLocalTransfer = active
    updateTransferIdleTimer()
  }

  private func updateTransferIdleTimer() {
    let foregroundScene = hostScene?.activationState == .foregroundActive
      || hostScene?.activationState == .foregroundInactive
    let preventSleep = activeLocalTransfer && foregroundScene && !lifecycleSuspended
      && UIApplication.shared.applicationState != .background
    if preventSleep {
      if transferIdleTimer == nil { transferIdleTimer = MobileIdleTimerLease() }
    } else {
      transferIdleTimer?.release()
      transferIdleTimer = nil
    }
  }

  // Internal Rust-only authority: idle discovery/advertisement never calls this
  // with active=true. Background execution uses the finite checkpoint, not this.
  @objc public func localTransferState(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(LocalTransferArgs.self)
    DispatchQueue.main.async {
      self.setLocalTransferActive(args.active)
      invoke.resolve()
    }
  }

  private func stopCameraCapture() {
    if #available(iOS 15.0, *) {
      webView?.setCameraCaptureState(.none, completionHandler: nil)
    }
  }

  private func revokeCamera(reason: String) {
    // Read-aloud is audio-background capable; its terminal state retires the
    // native session instead of an app-background transition.
    stopCameraCapture()
    hardwareShortcuts.unbind()
    trigger("camera-revoked", data: ["reason": reason])
    if !lifecycleSuspended {
      // UIKit must grant execution time synchronously while delivering the
      // background notification, before Rust's asynchronous retirement begins.
      // beginCheckpoint adopts this reservation rather than starting a second
      // task after the process may already have been suspended.
      let reservation = reserveCheckpoint()
      pendingCheckpoint = reservation
      lifecycleSuspended = true
      updateTransferIdleTimer()
      trigger("mobile-suspended", data: ["transition": reservation.transition])
    }
  }

  private func speechEngine() -> NativeSpeech {
    if let speech = speech { return speech }
    let engine = NativeSpeech(foreground: { [weak self] in
      guard let self = self else { return false }
      return self.hostScene?.activationState == .foregroundActive && UIApplication.shared.applicationState == .active
    }, setSpeechActive: { [audio = readerAudio] active in audio.setSpeechActive(active) },
       setSpeechPaused: { [audio = readerAudio] paused in audio.setSpeechPaused(paused) },
       emit: { [weak self] state in self?.trigger("speech-state", data: state) })
    speech = engine
    return engine
  }

  // Internal RustMobile.call transport only; no JavaScript command exports.
  @objc public func speechVoices(_ invoke: Invoke) {
    DispatchQueue.main.async { self.speechEngine().voices(invoke) }
  }
  @objc public func speechPrepare(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechPrepareArgs.self)
      DispatchQueue.main.async {
        self.speechScene = self.hostScene
        self.speechEngine().prepare(args, invoke: invoke)
      }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechStart(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechStartArgs.self)
      DispatchQueue.main.async { self.speechEngine().start(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechPause(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechSessionArgs.self)
      DispatchQueue.main.async { self.speechEngine().pause(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechResume(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechSessionArgs.self)
      DispatchQueue.main.async { self.speechEngine().resume(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechPrevious(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechSessionArgs.self)
      DispatchQueue.main.async { self.speechEngine().previous(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechStop(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechSessionArgs.self)
      DispatchQueue.main.async { self.speechEngine().stop(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }
  @objc public func speechRetire(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(NativeSpeechSessionArgs.self)
      DispatchQueue.main.async { self.speechEngine().retire(args, invoke: invoke) }
    } catch { invoke.reject("SPEECH_INVALID_REQUEST", code: "SPEECH_INVALID_REQUEST") }
  }

  /// Internal RustMobile.call transport only. The shell reports audible media
  /// or PiP; the first audible surface latches the session for the reader
  /// session, and `release` drops it when that reader surface is gone.
  @objc public func readerAudioState(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(ReaderAudioArgs.self)
      DispatchQueue.main.async {
        if self.readerAudio.setMediaActive(args.active, release: args.release ?? false) {
          invoke.resolve(["active": args.active])
        } else {
          invoke.reject("kiwix_mobile.audio_session_failed", code: "audio_session_failed")
        }
      }
    } catch { invoke.reject("kiwix_mobile.invalid_audio_state", code: "invalid_audio_state") }
  }

  /// Internal RustMobile.call transport for the cross-platform Kokoro player.
  /// The Rust audio thread owns the stream; this only claims/releases the
  /// process-wide playback category so audio survives backgrounding.
  @objc public func speechEngineStart(_ invoke: Invoke) {
    DispatchQueue.main.async {
      if self.readerAudio.setSpeechActive(true) { invoke.resolve(["active": true]) }
      else { invoke.reject("kiwix_mobile.audio_session_failed", code: "audio_session_failed") }
    }
  }

  private struct SpeechEngineUpdateArgs: Decodable { let paused: Bool }

  @objc public func speechEngineUpdate(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(SpeechEngineUpdateArgs.self)
      DispatchQueue.main.async {
        _ = self.readerAudio.setSpeechPaused(args.paused)
        invoke.resolve(["paused": args.paused])
      }
    } catch { invoke.reject("kiwix_mobile.invalid_audio_state", code: "invalid_audio_state") }
  }

  @objc public func speechEngineStop(_ invoke: Invoke) {
    DispatchQueue.main.async {
      _ = self.readerAudio.setSpeechActive(false)
      #if canImport(NowPlaying)
      if #available(iOS 27.0, *) {
        (self.engineNowPlaying as? SpeechNowPlaying)?.stop()
        self.engineNowPlaying = nil
      }
      #endif
      invoke.resolve(["active": false])
    }
  }

  private struct SpeechEngineNowPlayingArgs: Decodable {
    let active: Bool
    let speaking: Bool?
    let title: String?
    let subtitle: String?
  }

  /// Publishes the Rust Kokoro player as a Now Playing session so the lock
  /// screen and Dynamic Island show what is being read and offer transport.
  @objc public func speechEngineNowPlaying(_ invoke: Invoke) {
    do {
      let args = try invoke.parseArgs(SpeechEngineNowPlayingArgs.self)
      DispatchQueue.main.async {
        #if canImport(NowPlaying)
        if #available(iOS 27.0, *) {
          if args.active {
            if self.engineNowPlaying == nil {
              let controller = SpeechNowPlaying(
                id: UUID().uuidString,
                title: args.title ?? "Reading aloud",
                subtitle: args.subtitle,
                onPlay: { [weak self] in self?.trigger("speech-action", data: ["action": "play"]) },
                onPause: { [weak self] in self?.trigger("speech-action", data: ["action": "pause"]) },
                onPrevious: { [weak self] in self?.trigger("speech-action", data: ["action": "previous"]) },
                onStop: { [weak self] in self?.trigger("speech-action", data: ["action": "stop"]) })
              self.engineNowPlaying = controller
              controller.activate()
            }
            (self.engineNowPlaying as? SpeechNowPlaying)?.setPlaying(args.speaking ?? true)
          } else {
            (self.engineNowPlaying as? SpeechNowPlaying)?.stop()
            self.engineNowPlaying = nil
          }
        }
        #endif
        invoke.resolve(["active": args.active])
      }
    } catch { invoke.reject("kiwix_mobile.invalid_audio_state", code: "invalid_audio_state") }
  }

  @objc public func networkState(_ invoke: Invoke) {
    DispatchQueue.main.async { invoke.resolve(self.networkPolicy.snapshot()) }
  }

  private func nearbyBluetoothManager() -> NearbyBluetooth {
    if let nearbyRadio = nearbyRadio { return nearbyRadio }
    let radio = NearbyBluetooth(currentScene: { [weak self] in self?.hostScene },
      emit: { [weak self] state in self?.trigger("nearby-bluetooth", data: state) })
    nearbyRadio = radio
    return radio
  }
  private func nearbyJoinManager() -> NearbyJoin {
    if let nearbyAssociation = nearbyAssociation { return nearbyAssociation }
    let association = NearbyJoin(currentScene: { [weak self] in self?.hostScene },
      emit: { [weak self] state in self?.trigger("nearby-join", data: state) })
    nearbyAssociation = association
    return association
  }
  private func retireNearby(completion: ((String?) -> Void)? = nil) {
    nearbyRadio?.retire(); nearbyRoutes.retire()
    if let association = nearbyAssociation { association.retire(completion: completion) }
    else { completion?(nil) }
  }

  // Internal Rust plugin invokes only: none of these methods is a browser grant.
  @objc public func nearbyRetire(_ invoke: Invoke) {
    DispatchQueue.main.async {
      self.retireNearby { error in
        if let error = error { invoke.reject(error) } else { invoke.resolve() }
      }
    }
  }
  @objc public func nearbyBluetoothStart(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyBluetoothStartArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().start(args, scene: self.hostScene, invoke: invoke) }
  }
  @objc public func nearbyBluetoothConnect(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyBluetoothConnectArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().connect(args, invoke: invoke) }
  }
  @objc public func nearbyBluetoothSend(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyBluetoothSendArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().send(args, invoke: invoke) }
  }
  @objc public func nearbyBluetoothDisconnect(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyBluetoothConnectionArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().disconnect(args, invoke: invoke) }
  }
  @objc public func nearbyBluetoothStop(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyOwnerArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().stop(args.owner, invoke: invoke) }
  }
  @objc public func nearbyBluetoothScan(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyBluetoothScanArgs.self)
    DispatchQueue.main.async { self.nearbyBluetoothManager().scan(args.owner, enabled: args.enabled, invoke: invoke) }
  }
  @objc public func nearbyJoin(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyJoinArgs.self)
    DispatchQueue.main.async {
      _ = self.networkPolicy.snapshot()
      self.nearbyJoinManager().join(args, scene: self.hostScene, invoke: invoke)
    }
  }
  @objc public func nearbyJoinState(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyJoinLeaseArgs.self)
    DispatchQueue.main.async { self.nearbyJoinManager().state(args, invoke: invoke) }
  }
  @objc public func nearbyJoinBindSocket(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyJoinBindArgs.self)
    DispatchQueue.main.async { self.nearbyJoinManager().bind(args, invoke: invoke) }
  }
  @objc public func nearbyJoinRelease(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyJoinLeaseArgs.self)
    DispatchQueue.main.async { self.nearbyJoinManager().release(args, invoke: invoke) }
  }
  @objc public func nearbyJoinCancel(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyOwnerArgs.self)
    DispatchQueue.main.async { self.nearbyJoinManager().cancel(args.owner, invoke: invoke) }
  }
  @objc public func nearbyRouteBindSocket(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyRouteBindArgs.self)
    DispatchQueue.main.async {
      _ = self.networkPolicy.snapshot()
      self.nearbyRoutes.bind(args, invoke: invoke)
    }
  }
  @objc public func nearbyRouteState(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(NearbyRouteStateArgs.self)
    DispatchQueue.main.async { self.nearbyRoutes.state(args, invoke: invoke) }
  }

  @objc public func pickArchiveCopies(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ImportArgs.self)
    DispatchQueue.main.async { self.startPicker(invoke, kind: .archives(args), labels: args.labels, types: [.data], multiple: true) }
  }

  @objc public func pickExternalArchive(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExternalPickArgs.self)
    DispatchQueue.main.async {
      self.startPicker(invoke, kind: .external(args), labels: args.labels,
                       types: args.folder ? [.folder] : [.data], multiple: false)
    }
  }
  @objc public func pickExternalArchives(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExternalPickArgs.self)
    DispatchQueue.main.async {
      self.startPicker(invoke, kind: .externalBatch(args), labels: args.labels,
                       types: args.folder ? [.folder] : [.data], multiple: !args.folder)
    }
  }
  @objc public func openIncomingArchive(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(IncomingOriginalArgs.self)
    guard let url = URL(string: args.uri), url.isFileURL,
          url.host == nil || url.host == "" || url.host == "localhost",
          url.query == nil, url.fragment == nil else {
      invoke.reject("kiwix_mobile.invalid_incoming_documents", code: "invalid_incoming_documents"); return
    }
    DispatchQueue.main.async {
      let pick = ExternalPickArgs(persistent: args.persistent, folder: false, labels: args.labels)
      guard let (current, _) = self.start(invoke, kind: .external(pick)) else { return }
      current.working = true
      self.worker.async {
        do {
          try current.cancellation.check()
          let selection = try self.externalDocuments.selectIncoming(url,
            persistent: args.persistent, cancellation: current.cancellation)
          DispatchQueue.main.async {
            if current.cancellation.isCancelled {
              self.worker.async {
                do {
                  try self.externalDocuments.forget(selection.reference)
                  DispatchQueue.main.async { self.finishCancelled(current) }
                } catch {
                  DispatchQueue.main.async { self.fail(current, error) }
                }
              }
            } else {
              self.finish(current, ExternalPickReply(selection: selection, cancelled: false))
            }
          }
        } catch {
          DispatchQueue.main.async { self.fail(current, error) }
        }
      }
    }
  }
  @objc public func releaseIncomingOriginals(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(IncomingOriginalReleaseArgs.self)
    let urls = args.uris.compactMap { URL(string: $0) }
    guard urls.count == args.uris.count, urls.allSatisfy({ $0.isFileURL }) else {
      invoke.reject("kiwix_mobile.invalid_incoming_documents", code: "invalid_incoming_documents"); return
    }
    worker.async {
      IncomingOriginalAuthority.forget(urls)
      invoke.resolve(CancelReply(cancelled: false))
    }
  }
  @objc public func acquireExternalArchive(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExternalReferenceArgs.self)
    worker.async {
      do { invoke.resolve(try self.externalDocuments.acquire(args.reference, cancellation: WorkCancellation())) }
      catch { invoke.resolve(["error": ["code": providerAcquisitionCode(error)]]) }
    }
  }
  @objc public func releaseExternalArchive(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExternalLeaseArgs.self)
    worker.async { self.externalDocuments.release(args.lease); invoke.resolve(CancelReply(cancelled: false)) }
  }
  @objc public func forgetExternalArchive(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExternalReferenceArgs.self)
    worker.async {
      do { try self.externalDocuments.forget(args.reference); invoke.resolve(CancelReply(cancelled: false)) }
      catch { invoke.reject("kiwix_mobile.invalid_provider_reference", code: "invalid_provider_reference") }
    }
  }

  // Rust alone supplies URLs from its private, explicitly confirmed Open-In
  // request. This command must never be exposed directly to archive frames.
  @objc public func copyIncomingArchives(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(IncomingArgs.self)
    let urls = args.uris.compactMap { URL(string: $0) }
    guard !urls.isEmpty, urls.count == args.uris.count,
          urls.allSatisfy({ $0.isFileURL && ($0.host == nil || $0.host == "" || $0.host == "localhost") && $0.query == nil && $0.fragment == nil }),
          let copying = args.labels.copying, !copying.isEmpty else {
      invoke.reject("kiwix_mobile.invalid_incoming_documents", code: "invalid_incoming_documents"); return
    }
    let pick = ImportArgs(destinationDirectory: args.destinationDirectory, labels: args.labels)
    DispatchQueue.main.async {
      guard let (current, _) = self.start(invoke, kind: .archives(pick)) else { return }
      current.working = true
      self.showProgress(current)
      self.worker.async { self.processSelection(current, urls: urls) }
    }
  }
  @objc public func pickLibraryDirectoryCopy(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ImportArgs.self)
    DispatchQueue.main.async { self.startPicker(invoke, kind: .directory(args), labels: args.labels, types: [.folder], multiple: false) }
  }
  @objc public func readTextDocument(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(TextArgs.self)
    guard args.maxBytes > 0 else {
      invoke.reject("kiwix_mobile.invalid_text_limit", code: "invalid_text_limit"); return
    }
    DispatchQueue.main.async { self.startPicker(invoke, kind: .text(args), labels: args.labels, types: [.json, .xml], multiple: false) }
  }

  private func visiblePresenter() -> UIViewController? {
    guard let base = manager.viewController, let scene = base.viewIfLoaded?.window?.windowScene,
          scene.activationState == .foregroundActive, !base.isBeingDismissed else { return nil }
    var presenter = base
    while let shown = presenter.presentedViewController {
      guard !shown.isBeingDismissed else { return nil }
      presenter = shown
    }
    // Do not cover another app/system interaction with our picker.
    guard presenter === base else { return nil }
    return presenter
  }

  private func start(_ invoke: Invoke, kind: OperationKind) -> (DocumentOperation, UIViewController)? {
    guard operation == nil else { invoke.reject("kiwix_mobile.operation_busy", code: "operation_busy"); return nil }
    guard let presenter = visiblePresenter(), let scene = presenter.view.window?.windowScene else {
      invoke.reject("kiwix_mobile.no_visible_scene", code: "no_visible_scene"); return nil
    }
    let current = DocumentOperation(invoke: invoke, kind: kind, scene: scene)
    operation = current
    return (current, presenter)
  }

  private func startPicker(_ invoke: Invoke, kind: OperationKind, labels: PickerLabels, types: [UTType], multiple: Bool) {
    guard let (current, presenter) = start(invoke, kind: kind) else { return }
    switch kind {
    case .archives, .directory:
      guard let copying = labels.copying, !copying.isEmpty else {
        fail(current, MobileFailure("missing_copying_label")); return
      }
    default: break
    }
    let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
    picker.title = labels.title
    picker.allowsMultipleSelection = multiple
    picker.delegate = self
    picker.modalPresentationStyle = .formSheet
    current.controller = picker
    presenter.present(picker, animated: true)
    picker.presentationController?.delegate = self
  }

  public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    guard let current = operation, current.controller === controller else { return }
    cancelCurrent()
  }

  public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
    guard let current = operation, !current.settled, current.controller === controller else { return }
    if case .export = current.kind { finish(current, CancelReply(cancelled: false)); return }
    guard !urls.isEmpty else { cancelCurrent(); return }
    current.working = true
    let run = { [weak self] in
      guard let self = self, self.operation === current, !current.settled else { return }
      if current.cancellation.isCancelled { return }
      self.showProgress(current)
    }
    // UIDocumentPicker dismisses itself. Wait for its transition instead of
    // presenting over a disappearing provider controller.
    controller.dismiss(animated: true, completion: run)
    worker.async { self.processSelection(current, urls: urls) }
  }

  public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard let current = operation, !current.working,
          current.controller === presentationController.presentedViewController else { return }
    cancelCurrent()
  }

  private func labels(for kind: OperationKind) -> PickerLabels {
    switch kind {
    case .archives(let args), .directory(let args): return args.labels
    case .external(let args), .externalBatch(let args): return args.labels
    case .text(let args): return args.labels
    case .export(let args): return args.labels
    }
  }

  private func showProgress(_ current: DocumentOperation) {
    guard operation === current, !current.settled, !current.cancellation.isCancelled,
          current.scene?.activationState == .foregroundActive, let presenter = visiblePresenter() else { return }
    let progress = CopyProgressController(labels: labels(for: current.kind)) { [weak self] in self?.cancelCurrent() }
    current.progress = progress
    presenter.present(progress, animated: true)
  }

  private func progressCallback(_ current: DocumentOperation) -> (String, Int64) -> Void {
    // Throttle queued UI work: a multi-gigabyte archive must not enqueue a
    // main-thread closure for every 256 KiB chunk.
    var lastUpdate = DispatchTime.now().uptimeNanoseconds
    return { [weak self] name, bytes in
      let now = DispatchTime.now().uptimeNanoseconds
      guard bytes == 0 || now - lastUpdate >= 150_000_000 else { return }
      lastUpdate = now
      DispatchQueue.main.async {
        guard let self = self, self.operation === current, !current.settled else { return }
        current.progress?.update(name: name, bytes: bytes)
      }
    }
  }

  private func processSelection(_ current: DocumentOperation, urls: [URL]) {
    var staging: ArchiveStaging?
    var committed = false
    do {
      try current.cancellation.check()
      switch current.kind {
      case .archives(let args), .directory(let args):
        let destination = try ArchiveStaging(path: args.destinationDirectory)
        staging = destination
        let update = progressCallback(current)
        var folder = false
        if case .directory = current.kind { folder = true }
        if folder {
          guard urls.count == 1 else { throw MobileFailure("single_directory_required") }
          try withSecurityScope(urls[0]) { source in
            try destination.copyTree(source, cancellation: current.cancellation, progress: update)
          }
        } else {
          for url in urls {
            try current.cancellation.check()
            guard isArchiveName(url.lastPathComponent) else { throw MobileFailure("not_archive", url.lastPathComponent) }
            try coordinatedRead(url, cancellation: current.cancellation) { source in
              // Keep the chosen basename, not a provider's temporary alias.
              try destination.copy(source, components: [url.lastPathComponent], cancellation: current.cancellation, progress: update)
            }
          }
        }
        try current.cancellation.check()
        try destination.finish()
        committed = true
        if folder { try persistFolderBookmark(urls[0], destination: destination.url) }
        let reply = ImportReply(paths: destination.paths, cancelled: false, directory: folder ? destination.url.path : nil)
        DispatchQueue.main.async {
          // Once fsync succeeds this is an owned publication candidate; a late
          // scene cancellation must not silently discard it.
          self.finish(current, reply)
        }
      case .external(let args):
        guard urls.count == 1 else { throw MobileFailure("single_document_required") }
        let selection = try externalDocuments.select(urls[0], folder: args.folder,
          persistent: args.persistent, cancellation: current.cancellation)
        DispatchQueue.main.async {
          if current.cancellation.isCancelled {
            self.worker.async { try? self.externalDocuments.forget(selection.reference) }
            self.finishCancelled(current)
          } else {
            self.finish(current, ExternalPickReply(selection: selection, cancelled: false))
          }
        }
      case .externalBatch(let args):
        if args.folder && urls.count != 1 { throw MobileFailure("single_directory_required") }
        let result = try externalDocuments.selectBatch(urls, folder: args.folder,
          persistent: args.persistent, cancellation: current.cancellation)
        DispatchQueue.main.async {
          if current.cancellation.isCancelled {
            self.worker.async {
              do {
                for selection in result.selections { try self.externalDocuments.forget(selection.reference) }
                DispatchQueue.main.async { self.finishCancelled(current) }
              } catch {
                DispatchQueue.main.async { self.fail(current, error) }
              }
            }
          } else { self.finish(current, result) }
        }
      case .text(let args):
        guard urls.count == 1, ["json", "xml"].contains(urls[0].pathExtension.lowercased()) else { throw MobileFailure("not_text_document") }
        let text = try readBoundedText(urls[0], maxBytes: args.maxBytes, cancellation: current.cancellation)
        try current.cancellation.check()
        DispatchQueue.main.async { self.finish(current, TextReply(text: text, cancelled: false)) }
      case .export:
        throw MobileFailure("invalid_operation")
      }
    } catch {
      if committed || (error as? MobileFailure)?.code == "sync_failed" {
        let retained = staging?.url.path
        DispatchQueue.main.async { self.fail(current, error, retainedDirectory: retained) }
      } else if current.cancellation.isCancelled, let destination = staging {
        do {
          try destination.cancelAndRemoveOwnedFiles()
          DispatchQueue.main.async { self.finishCancelled(current) }
        } catch {
          let retained = destination.url.path
          DispatchQueue.main.async { self.fail(current, error, retainedDirectory: retained) }
        }
      } else if current.cancellation.isCancelled {
        DispatchQueue.main.async { self.finishCancelled(current) }
      } else {
        let retained = staging?.url.path
        DispatchQueue.main.async { self.fail(current, error, retainedDirectory: retained) }
      }
    }
  }

  private func cancelCurrent() {
    guard let current = operation, !current.settled else { return }
    current.cancellation.cancel()
    current.progress?.dismiss(animated: false)
    current.controller?.dismiss(animated: false)
    // An in-flight coordinated stream owns its scope and scratch until it exits.
    if !current.working { finishCancelled(current) }
  }

  private func finishCancelled(_ current: DocumentOperation) {
    switch current.kind {
    case .archives, .directory: finish(current, ImportReply(paths: [], cancelled: true, directory: nil))
    case .external: finish(current, ExternalPickReply(selection: nil, cancelled: true))
    case .externalBatch: finish(current, CancelReply(cancelled: true))
    case .text: finish(current, TextReply(text: nil, cancelled: true))
    case .export: finish(current, CancelReply(cancelled: true))
    }
  }

  private func release(_ current: DocumentOperation, completion: @escaping () -> Void) {
    guard operation === current, !current.settled else { return }
    current.settled = true
    current.progress?.dismiss(animated: false)
    current.controller?.dismiss(animated: false)
    guard let scratch = current.exportScratch else {
      operation = nil
      completion()
      return
    }
    worker.async {
      let result = Result { try scratch.cleanup() }
      DispatchQueue.main.async {
        switch result {
        case .success:
          current.exportScratch = nil
          self.operation = nil
          completion()
        case .failure(let error):
          let path = scratch.directoryURL.path
          // Keep descriptor/identity ownership when cleanup fails. The caller
          // receives a recovery path, never a false successful cancellation.
          self.retainExport(scratch)
          self.operation = nil
          let cause = (error as? MobileFailure)?.code ?? "provider_error"
          self.rejectWithRecovery(current.invoke, code: "export_cleanup_failed",
                                  details: ["retainedScratchPath": path, "cleanupCause": cause])
        }
      }
    }
  }

  /// Retains a scratch whose cleanup failed and retries cleanup so a transient
  /// failure cannot pin two descriptors and a temporary directory for the
  /// process lifetime. Main-thread owned, like `retainedExports`.
  private func retainExport(_ scratch: ExportScratch) {
    retainedExports.append(scratch)
    retainedExportAttempts = 0
    retainedExportRetry?.cancel(); retainedExportRetry = nil
    drainRetainedExports()
  }

  private func drainRetainedExports() {
    guard !drainingRetainedExports, !retainedExports.isEmpty else { return }
    drainingRetainedExports = true
    let pending = retainedExports
    retainedExports.removeAll()
    var survivors: [ExportScratch] = []
    let group = DispatchGroup()
    for scratch in pending {
      group.enter()
      // The document worker is serial, so survivors keep their insertion order.
      worker.async {
        do { try scratch.cleanup() } catch { survivors.append(scratch) }
        group.leave()
      }
    }
    group.notify(queue: .main) {
      self.drainingRetainedExports = false
      self.retainedExports.insert(contentsOf: survivors, at: 0)
      if self.retainedExports.count > Self.retainedExportLimit {
        // Releasing a scratch closes both descriptors. Its temporary directory
        // is left for the system rather than pinning descriptors forever.
        self.retainedExports.removeFirst(self.retainedExports.count - Self.retainedExportLimit)
      }
      guard !self.retainedExports.isEmpty else { self.retainedExportAttempts = 0; return }
      self.retainedExportAttempts += 1
      guard self.retainedExportAttempts < Self.retainedExportAttemptLimit else {
        self.retainedExports.removeAll(); self.retainedExportAttempts = 0
        return
      }
      let retry = DispatchWorkItem { [weak self] in
        guard let self = self else { return }
        self.retainedExportRetry = nil
        self.drainRetainedExports()
      }
      self.retainedExportRetry = retry
      DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: retry)
    }
  }

  private func finish<T: Encodable>(_ current: DocumentOperation, _ reply: T) {
    release(current) { current.invoke.resolve(reply) }
  }

  private func fail(_ current: DocumentOperation, _ error: Error, retainedDirectory: String? = nil) {
    let failure = (error as? MobileFailure) ?? MobileFailure("provider_error", error.localizedDescription)
    release(current) {
      var details: JsonObject = [:]
      if let directory = retainedDirectory { details["retainedDirectory"] = directory }
      if let detail = failure.detail { details["detail"] = detail }
      if let path = failure.retainedScratchPath { details["retainedScratchPath"] = path }
      // These are machine keys. The shell localizes error messages; no English
      // app-authored native error dialog bypasses the requested language.
      self.rejectWithRecovery(current.invoke, code: failure.code, details: details)
    }
  }

  private func rejectWithRecovery(_ invoke: Invoke, code: String, details: JsonObject) {
    // Tauri's Rust ErrorResponse<()> drops extra native rejection fields. Keep
    // the same recovery payload inside message so error.to_string() preserves it.
    let message: String
    do {
      let encoded = try JSONSerialization.data(withJSONObject: ["code": code, "data": details], options: [.sortedKeys])
      message = String(decoding: encoded, as: UTF8.self)
    } catch {
      // Recovery paths must remain visible even if a future caller introduces
      // a value that Foundation cannot serialize as JSON.
      let recovery = details.keys.sorted().map { key in key + "=" + String(describing: details[key] ?? nil) }
      message = "kiwix_mobile." + code + "\n" + recovery.joined(separator: "\n")
    }
    invoke.reject(message, code: code, data: .dictionary(details))
  }

  @objc public func cancelPdfExport(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PdfCancelArgs.self)
    DispatchQueue.main.async {
      if let current = self.operation, case .export(let export) = current.kind,
         export.pdfAuthority == args.pdfAuthority {
        self.cancelCurrent()
      }
      invoke.resolve()
    }
  }

  @objc public func exportFile(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ExportArgs.self)
    DispatchQueue.main.async {
      guard let (current, _) = self.start(invoke, kind: .export(args)) else { return }
      guard !args.mimeType.isEmpty, !args.mimeType.contains(where: { $0.isNewline }),
            !args.labels.title.isEmpty, !args.labels.cancel.isEmpty else {
        self.fail(current, MobileFailure("invalid_export_arguments")); return
      }
      current.working = true
      self.showProgress(current)
      self.worker.async {
        do {
          let export = try prepareExport(path: args.path, suggestedName: args.suggestedName,
                                         cancellation: current.cancellation, progress: self.progressCallback(current))
          DispatchQueue.main.async {
            current.exportScratch = export
            current.working = false
            if current.cancellation.isCancelled { self.finishCancelled(current); return }
            let present = {
              guard self.operation === current, !current.settled else { return }
              if current.cancellation.isCancelled { self.finishCancelled(current); return }
              current.progress = nil
              guard let presenter = self.visiblePresenter() else { self.fail(current, MobileFailure("no_visible_scene")); return }
              // Both the share sheet (article PDF sharing) and the document
              // picker are irreversible system handoffs when a one-use PDF
              // authority is supplied; present them only inside that commit.
              let presentSystemUI: () -> Void
              if args.open {
                let activity = UIActivityViewController(activityItems: [export.url], applicationActivities: nil)
                activity.title = args.labels.title
                activity.completionWithItemsHandler = { [weak self] _, completed, _, error in
                  DispatchQueue.main.async {
                    guard let self = self else { return }
                    if let error = error { self.fail(current, error) }
                    else { self.finish(current, CancelReply(cancelled: !completed)) }
                  }
                }
                if let popover = activity.popoverPresentationController {
                  popover.sourceView = presenter.view
                  popover.sourceRect = CGRect(x: presenter.view.bounds.midX, y: presenter.view.bounds.midY, width: 1, height: 1)
                  popover.permittedArrowDirections = []
                }
                current.controller = activity
                presentSystemUI = {
                  presenter.present(activity, animated: true)
                  activity.presentationController?.delegate = self
                }
              } else {
                let picker = UIDocumentPickerViewController(forExporting: [export.url], asCopy: true)
                picker.title = args.labels.title
                picker.delegate = self
                current.controller = picker
                presentSystemUI = {
                  presenter.present(picker, animated: true)
                  picker.presentationController?.delegate = self
                }
              }
              if let token = args.pdfAuthority {
                let opened = token.withCString { token in
                  kiwix_pdf_export_commit(token) { presentSystemUI() }
                }
                if !opened { self.fail(current, MobileFailure("pdf_authority_expired")) }
              } else {
                presentSystemUI()
              }
            }
            if let progress = current.progress, progress.presentingViewController != nil {
              progress.dismiss(animated: true, completion: present)
            } else { present() }
          }
        } catch {
          DispatchQueue.main.async {
            let preparation = error as? ExportPreparationFailure
            if let preparation = preparation { current.exportScratch = preparation.scratch }
            let original = preparation?.underlying ?? error
            if current.cancellation.isCancelled, (original as? MobileFailure)?.retainedScratchPath == nil {
              self.finishCancelled(current)
            } else {
              self.fail(current, original)
            }
          }
        }
      }
    }
  }

  @objc public func openUrl(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(URLArgs.self)
    guard let parts = URLComponents(string: args.url), let scheme = parts.scheme?.lowercased(),
          ["http", "https", "mailto"].contains(scheme),
          (scheme == "mailto" || !(parts.host ?? "").isEmpty),
          parts.user == nil, parts.password == nil, let url = parts.url,
          !args.url.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
      invoke.reject("kiwix_mobile.invalid_external_url", code: "invalid_external_url"); return
    }
    DispatchQueue.main.async {
      guard self.visiblePresenter() != nil else { invoke.reject("kiwix_mobile.no_visible_scene", code: "no_visible_scene"); return }
      UIApplication.shared.open(url, options: [:]) { opened in
        if opened { invoke.resolve() }
        else { invoke.reject("kiwix_mobile.external_url_unavailable", code: "external_url_unavailable") }
      }
    }
  }

  private func reserveCheckpoint() -> CheckpointReservation {
    let token = nextCheckpoint
    nextCheckpoint += 1
    let task = UIApplication.shared.beginBackgroundTask(withName: "kiwix-checkpoint") { [weak self] in
      self?.expireCheckpoint(token)
    }
    guard task != .invalid else { return .unavailable(token) }
    checkpoints[token] = task
    let cleanup = DispatchWorkItem { [weak self] in
      guard let self, self.checkpoints[token] != nil else { return }
      self.retiringCheckpoints.insert(token)
      self.trigger("checkpoint-retiring", data: ["token": token])
    }
    checkpointCleanup[token] = cleanup
    // Keep time for cancellation, verified-chunk persistence and native joins.
    // UIKit can still expire earlier; its expiration handler remains authoritative.
    DispatchQueue.main.asyncAfter(deadline: .now() + checkpointGraceSeconds(), execute: cleanup)
    return .active(token)
  }

  @objc public func beginCheckpoint(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(BeginCheckpointArgs.self)
    DispatchQueue.main.async {
      // An obsolete Rust transition must not consume a newer reservation.
      guard let reserved = self.pendingCheckpoint, reserved.transition == args.transition else {
        invoke.reject("kiwix_mobile.checkpoint_superseded", code: "checkpoint_superseded"); return
      }
      self.pendingCheckpoint = nil
      // A reservation is single-use, including failure and expiry. Never renew
      // UIKit execution time for the same background transition.
      switch reserved {
      case .active(let token) where self.checkpoints[token] != nil:
        invoke.resolve(["token": token])
      case .expired, .active:
        invoke.reject("kiwix_mobile.checkpoint_expired", code: "checkpoint_expired")
      case .unavailable:
        invoke.reject("kiwix_mobile.checkpoint_unavailable", code: "checkpoint_unavailable")
      }
    }
  }

  private func checkpointBudgetSeconds() -> TimeInterval {
    let remaining = UIApplication.shared.backgroundTimeRemaining
    return remaining.isFinite && remaining < 86400 ? max(0, remaining) : 30
  }

  private func checkpointGraceSeconds() -> TimeInterval {
    max(0, checkpointBudgetSeconds() - 5)
  }

  private func releaseCheckpoint(_ token: Int) -> Bool {
    checkpointCleanup.removeValue(forKey: token)?.cancel()
    retiringCheckpoints.remove(token)
    guard let task = checkpoints.removeValue(forKey: token) else { return false }
    UIApplication.shared.endBackgroundTask(task)
    return true
  }

  @objc public func checkpointState(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CheckpointArgs.self)
    DispatchQueue.main.async {
      invoke.resolve(["active": self.checkpoints[args.token] != nil,
                      "retiring": self.retiringCheckpoints.contains(args.token),
                      "graceSeconds": self.checkpointGraceSeconds(),
                      "osSeconds": max(0, self.checkpointBudgetSeconds() - 1)] as [String: Any])
    }
  }

  @objc public func endCheckpoint(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CheckpointArgs.self)
    DispatchQueue.main.async {
      // Idempotent after expiry: the native identifier is ended only once.
      let active = self.releaseCheckpoint(args.token)
      // Rust can detect expiry during retirement even if bridge event delivery
      // is delayed until after this completion reply.
      invoke.resolve(["active": active])
    }
  }

  private func expireCheckpoint(_ token: Int) {
    guard checkpoints[token] != nil else { return }
    if case .active(let reserved) = pendingCheckpoint, reserved == token { pendingCheckpoint = .expired(token) }
    // End immediately, before bridge event dispatch can do any work.
    _ = releaseCheckpoint(token)
    trigger("checkpoint-expired", data: ["token": token])
  }
  private func expireAllCheckpoints() { Array(checkpoints.keys).forEach { expireCheckpoint($0) } }
}

@_cdecl("init_plugin_kiwix_mobile")
public func initPlugin() -> Plugin { KiwixMobilePlugin() }
