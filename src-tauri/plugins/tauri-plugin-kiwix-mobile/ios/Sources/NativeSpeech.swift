import AVFoundation
import Foundation
import Tauri
import UIKit

struct NativeSpeechLabels: Decodable {
  let title: String?
  let pause: String?
  let resume: String?
  let stop: String?
  let previous: String?
  let rewind: String?
  let forward: String?
}
struct NativeSpeechStartArgs: Decodable {
  let id: String
  let token: String
  let text: String
  let lang: String
  let voice: String
  let rate: Double
  let labels: NativeSpeechLabels?

  init(id: String, token: String, text: String, lang: String, voice: String,
       rate: Double, labels: NativeSpeechLabels? = nil) {
    self.id = id; self.token = token; self.text = text; self.lang = lang
    self.voice = voice; self.rate = rate; self.labels = labels
  }
}
struct NativeSpeechPrepareArgs: Decodable { let id: String }
struct NativeSpeechSessionArgs: Decodable { let id: String; let token: String }

/// Main-thread owned; the plugin exposes these methods only to trusted RustMobile.call.
final class NativeSpeech: NSObject, AVSpeechSynthesizerDelegate {
  private let foreground: () -> Bool
  private let emit: (JSObject) -> Void
  /// Shared with archive media: speech and media never deactivate each other's
  /// session while the other surface is still audible.
  private let setSpeechActive: (Bool) -> Bool
  private let setSpeechPaused: (Bool) -> Bool
  private let makeSynthesizer: () -> AVSpeechSynthesizer
  private let availableVoices: () -> [AVSpeechSynthesisVoice]
  private var synthesizer: AVSpeechSynthesizer?
  private var current: Session?
  #if canImport(NowPlaying)
  private var nowPlaying: AnyObject?
  #endif
  private var registrations: [String: Registration] = [:]
  private var nextRegistration = 0
  private var latestStarted = 0
  private var interruption: NSObjectProtocol?
  private var audioObservers: [NSObjectProtocol] = []
  /// Sorted catalogue from `availableVoices()`. Enumerating installed voices is
  /// a system query that allocates the whole catalogue, so it is cached until
  /// the audio session or speech engine generation changes.
  private var voicesCache: [AVSpeechSynthesisVoice]?
  private var startTimeout: DispatchWorkItem?
  private var ownsAudioSession = false
  private static let languagePattern = try! NSRegularExpression(pattern: "^[A-Za-z]{2,8}(?:-[A-Za-z0-9]{1,8})*\\z")

  private enum Status { case prepared, started, cancelled, finished }
  private final class Registration {
    let id: String
    let capability: String
    let ordinal: Int
    var status = Status.prepared
    var offset = 0
    init(id: String, ordinal: Int) { self.id = id; self.ordinal = ordinal; capability = UUID().uuidString }
  }

  private final class Session {
    let registration: Registration
    var id: String { registration.id }
    let text: NSString
    let voice: AVSpeechSynthesisVoice
    let rate: Float
    var offset = 0
    var paused = false
    var chunkStart = 0
    var chunkEnd = 0
    var utterance: AVSpeechUtterance?
    init(registration: Registration, text: String, voice: AVSpeechSynthesisVoice, rate: Float) {
      self.registration = registration; self.text = text as NSString; self.voice = voice; self.rate = rate
    }
  }

  init(foreground: @escaping () -> Bool, setSpeechActive: @escaping (Bool) -> Bool,
       setSpeechPaused: @escaping (Bool) -> Bool = { _ in true },
       makeSynthesizer: @escaping () -> AVSpeechSynthesizer = { AVSpeechSynthesizer() },
       availableVoices: @escaping () -> [AVSpeechSynthesisVoice] = { NativeSpeech.installedVoices() },
       emit: @escaping (JSObject) -> Void) {
    self.foreground = foreground; self.setSpeechActive = setSpeechActive
    self.setSpeechPaused = setSpeechPaused; self.emit = emit
    self.makeSynthesizer = makeSynthesizer; self.availableVoices = availableVoices
    super.init()
    interruption = NotificationCenter.default.addObserver(
      forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
    ) { [weak self] notification in
      guard let type = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
            type == AVAudioSession.InterruptionType.began.rawValue else { return }
      self?.cancelAll(error: "SPEECH_AUDIO_INTERRUPTED")
    }
    // Route and media-service changes are the only events that can change the
    // installed voice catalogue for a live session; returning from Settings
    // (where voices are installed) is the other.
    for name in [AVAudioSession.routeChangeNotification, AVAudioSession.mediaServicesWereResetNotification,
                 UIApplication.didBecomeActiveNotification] {
      audioObservers.append(NotificationCenter.default.addObserver(
        forName: name, object: nil, queue: .main
      ) { [weak self] _ in self?.invalidateVoices() })
    }
  }

  deinit {
    if let observer = interruption { NotificationCenter.default.removeObserver(observer) }
    audioObservers.forEach { NotificationCenter.default.removeObserver($0) }
    startTimeout?.cancel()
    // Plugin teardown normally calls shutdown first. Keep final release main-thread safe too.
    let engine = synthesizer
    let deactivate = ownsAudioSession
    let setSpeechActive = self.setSpeechActive
    let release = {
      engine?.delegate = nil
      engine?.stopSpeaking(at: .immediate)
      if deactivate { _ = setSpeechActive(false) }
    }
    if Thread.isMainThread { release() } else { DispatchQueue.main.async(execute: release) }
  }

  func voices(_ invoke: Invoke) {
    guard foreground() else { unavailable(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
    let voices = cachedVoices()
    let language = AVSpeechSynthesisVoice.currentLanguageCode()
    let preferred = voices.first { $0.language.caseInsensitiveCompare(language) == .orderedSame }
    let result: [[String: Any]] = voices.map {
      ["name": $0.name, "lang": $0.language, "voiceURI": $0.identifier,
       "default": $0.identifier == preferred?.identifier, "quality": $0.quality.rawValue,
       "localService": true]
    }
    var reply: JsonObject = ["available": !voices.isEmpty, "voices": result]
    if voices.isEmpty { reply["error"] = "SPEECH_OFFLINE_VOICES_UNAVAILABLE" }
    invoke.resolve(reply)
  }

  private func cachedVoices() -> [AVSpeechSynthesisVoice] {
    if let cached = voicesCache { return cached }
    let voices = availableVoices()
    voicesCache = voices
    return voices
  }

  private func invalidateVoices() { voicesCache = nil }

  func prepare(_ args: NativeSpeechPrepareArgs, invoke: Invoke) {
    guard validID(args.id) else { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
    guard foreground() else { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
    guard registrations[args.id] == nil else { reject(invoke, "SPEECH_SESSION_REGISTERED"); return }
    guard registrations.count < 8 else { reject(invoke, "SPEECH_BUSY"); return }
    nextRegistration += 1
    let registration = Registration(id: args.id, ordinal: nextRegistration)
    registrations[args.id] = registration
    invoke.resolve(["id": args.id, "token": registration.capability])
  }

  func start(_ args: NativeSpeechStartArgs, invoke: Invoke) {
    guard validID(args.id), validID(args.token), args.text.utf16.count <= 1_000_000,
          args.text.unicodeScalars.contains(where: { !CharacterSet.whitespacesAndNewlines.contains($0) }),
          args.voice.utf16.count <= 4096, validLanguage(args.lang),
          args.rate.isFinite, (0.25...2).contains(args.rate) else {
      reject(invoke, "SPEECH_INVALID_REQUEST"); return
    }
    guard foreground() else { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
    guard let registration = registrations[args.id], registration.capability == args.token else {
      reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return
    }
    guard registration.status == .prepared else { reject(invoke, "SPEECH_SESSION_CONSUMED"); return }
    guard registration.ordinal >= latestStarted else {
      registration.status = .cancelled
      reject(invoke, "SPEECH_SESSION_SUPERSEDED"); return
    }
    let voices = cachedVoices()
    let voice: AVSpeechSynthesisVoice?
    if !args.voice.isEmpty {
      voice = voices.first { $0.identifier == args.voice }
    } else {
      let language = Locale(identifier: args.lang).languageCode
      let deviceLanguage = AVSpeechSynthesisVoice.currentLanguageCode()
      voice = voices.first { $0.language.caseInsensitiveCompare(args.lang) == .orderedSame }
        ?? voices.first {
          $0.language.caseInsensitiveCompare(deviceLanguage) == .orderedSame &&
            (language == nil || Locale(identifier: $0.language).languageCode == language)
        }
        ?? voices.first { language != nil && Locale(identifier: $0.language).languageCode == language }
    }
    guard let selected = voice else {
      reject(invoke, args.voice.isEmpty ? "SPEECH_LANGUAGE_UNAVAILABLE" : "SPEECH_VOICE_UNAVAILABLE"); return
    }
    // Consume before touching current playback. Stop/Retire before delayed Start stays final.
    registration.status = .started
    latestStarted = registration.ordinal
    forceStop()
    guard setSpeechActive(true) else {
      registration.status = .cancelled
      reject(invoke, "SPEECH_AUDIO_SESSION_FAILED"); return
    }
    ownsAudioSession = true
    if synthesizer == nil {
      let engine = makeSynthesizer()
      engine.delegate = self
      synthesizer = engine
      invalidateVoices()
    }
    // AVSpeechUtterance's default (not literal rate=1) represents the user's 1x.
    let rate = min(AVSpeechUtteranceMaximumSpeechRate,
                   max(AVSpeechUtteranceMinimumSpeechRate, AVSpeechUtteranceDefaultSpeechRate * Float(args.rate)))
    let session = Session(registration: registration, text: args.text, voice: selected, rate: rate)
    current = session
    startNowPlaying(session, labels: args.labels, voice: selected)
    queueChunk(session)
    invoke.resolve(["id": args.id, "accepted": true])
  }

  private func ownedSession(_ args: NativeSpeechSessionArgs, invoke: Invoke) -> Session? {
    guard validID(args.id), validID(args.token) else {
      reject(invoke, "SPEECH_INVALID_REQUEST"); return nil
    }
    guard let session = current, session.id == args.id,
          session.registration.capability == args.token else {
      reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return nil
    }
    guard foreground() else { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return nil }
    return session
  }

  // Invalidate the utterance before stopping. Delayed progress/cancellation from
  // it cannot advance a paused session or cancel its replacement on Resume.
  private func halt(_ session: Session) {
    session.utterance = nil
    startTimeout?.cancel(); startTimeout = nil
    synthesizer?.stopSpeaking(at: .immediate)
    session.chunkStart = session.offset
    session.chunkEnd = session.offset
    session.registration.offset = session.offset
  }

  func pause(_ args: NativeSpeechSessionArgs, invoke: Invoke) {
    guard let session = ownedSession(args, invoke: invoke) else { return }
    pauseCurrent()
    invoke.resolve(["id": session.id, "paused": true, "offset": session.offset])
  }

  func resume(_ args: NativeSpeechSessionArgs, invoke: Invoke) {
    guard let session = ownedSession(args, invoke: invoke) else { return }
    guard resumeCurrent() else {
      reject(invoke, "SPEECH_AUDIO_SESSION_FAILED"); return
    }
    invoke.resolve(["id": session.id, "paused": false, "offset": session.offset])
  }

  func previous(_ args: NativeSpeechSessionArgs, invoke: Invoke) {
    guard let session = ownedSession(args, invoke: invoke) else { return }
    previousCurrent()
    invoke.resolve(["id": session.id, "paused": session.paused, "offset": session.offset])
  }

  private func pauseCurrent() {
    guard let session = current, !session.paused else { return }
    session.paused = true
    halt(session)
    // Keep the audio session alive while a Now Playing panel can resume this
    // session from the Lock Screen; otherwise iOS may suspend the app first.
    if hasNowPlaying {
      _ = setSpeechPaused(true)
    } else {
      deactivateAudio()
    }
    state(session, speaking: false)
  }

  @discardableResult
  private func resumeCurrent() -> Bool {
    guard let session = current, session.paused else { return true }
    guard setSpeechActive(true) else { return false }
    ownsAudioSession = true
    session.paused = false
    queueChunk(session)
    return true
  }

  private func previousCurrent() {
    guard let session = current else { return }
    halt(session)
    let end = min(max(session.offset, 0), session.text.length)
    let currentParagraph = session.text.range(of: "\n", options: .backwards,
      range: NSRange(location: 0, length: end))
    let previousParagraph = currentParagraph.location == NSNotFound || currentParagraph.location == 0
      ? NSRange(location: NSNotFound, length: 0)
      : session.text.range(of: "\n", options: .backwards,
          range: NSRange(location: 0, length: currentParagraph.location))
    session.offset = previousParagraph.location == NSNotFound ? 0 : previousParagraph.location + 1
    session.chunkStart = session.offset
    session.chunkEnd = session.offset
    session.registration.offset = session.offset
    if session.paused { state(session, speaking: false) }
    else { queueChunk(session) }
  }

  func stop(_ args: NativeSpeechSessionArgs, invoke: Invoke) {
    guard validID(args.id), validID(args.token) else { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
    let registration = registrations[args.id].flatMap { $0.capability == args.token ? $0 : nil }
    if let registration = registration {
      if current?.registration === registration { forceStop() }
      else if registration.status == .prepared { registration.status = .cancelled }
    }
    invoke.resolve(["id": args.id, "speaking": false, "offset": registration?.offset ?? 0,
                    "stopped": registration != nil])
  }

  func retire(_ args: NativeSpeechSessionArgs, invoke: Invoke) {
    guard validID(args.id), validID(args.token) else { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
    let registration = registrations[args.id].flatMap { $0.capability == args.token ? $0 : nil }
    if let registration = registration {
      registrations.removeValue(forKey: args.id)
      if current?.registration === registration { forceStop() }
    }
    invoke.resolve(["id": args.id, "retired": registration != nil])
  }

  func cancelAll(error: String) {
    forceStop(error: error)
    // Unregistered capabilities also reject Start; no unbounded cancellation tombstones.
    registrations.removeAll(keepingCapacity: true)
  }

  func forceStop(error: String? = nil) {
    guard let session = current else { return }
    // Invalidate before stopping: didCancel for the previous utterance must not retire a new one.
    current = nil
    startTimeout?.cancel(); startTimeout = nil
    session.registration.status = .cancelled
    session.registration.offset = session.offset
    session.paused = false
    synthesizer?.stopSpeaking(at: .immediate)
    deactivateAudio()
    state(session, speaking: false, error: error)
    stopNowPlaying()
  }

  func shutdown() {
    if !Thread.isMainThread { DispatchQueue.main.async { self.shutdown() }; return }
    cancelAll(error: "SPEECH_ENGINE_CLOSED")
    startTimeout?.cancel(); startTimeout = nil
    synthesizer?.delegate = nil
    synthesizer?.stopSpeaking(at: .immediate)
    synthesizer = nil
    invalidateVoices()
    deactivateAudio()
  }

  static func installedVoices() -> [AVSpeechSynthesisVoice] {
    // Third-party provider extensions have no API promising offline synthesis. Only installed
    // system voices are admitted; never initialize a language voice that could acquire assets.
    AVSpeechSynthesisVoice.speechVoices().filter { voice in
      guard voice.identifier.hasPrefix("com.apple.") else { return false }
      // Obsolete catalog entries can be listed but never start on current iOS.
      if voice.identifier == "com.apple.speech.synthesis.voice.Junior" { return false }
      if #available(iOS 17.0, *) {
        if voice.voiceTraits.contains(.isPersonalVoice) ||
            voice.voiceTraits.contains(.isNoveltyVoice) { return false }
      }
      return true
    }.sorted {
      if $0.language != $1.language { return $0.language < $1.language }
      if $0.quality.rawValue != $1.quality.rawValue {
        return $0.quality.rawValue > $1.quality.rawValue
      }
      return $0.identifier < $1.identifier
    }
  }

  private func queueChunk(_ session: Session) {
    guard current === session, !session.paused, let engine = synthesizer else { return }
    session.chunkStart = session.chunkEnd
    var end = min(session.text.length, session.chunkStart + 4000)
    if end < session.text.length {
      let high = session.text.character(at: end - 1), low = session.text.character(at: end)
      if (0xD800...0xDBFF).contains(high) && (0xDC00...0xDFFF).contains(low) { end -= 1 }
    }
    session.chunkEnd = end
    let utterance = AVSpeechUtterance(string: session.text.substring(with: NSRange(location: session.chunkStart, length: end - session.chunkStart)))
    utterance.voice = session.voice
    utterance.rate = session.rate
    session.utterance = utterance
    let timeout = DispatchWorkItem { [weak self, weak utterance] in
      guard let self = self, let utterance = utterance,
            self.current?.utterance === utterance else { return }
      self.forceStop(error: "SPEECH_START_TIMEOUT")
    }
    startTimeout?.cancel(); startTimeout = timeout
    DispatchQueue.main.asyncAfter(deadline: .now() + 15, execute: timeout)
    engine.speak(utterance)
  }

  private func handle(_ engine: AVSpeechSynthesizer, _ utterance: AVSpeechUtterance,
                      _ action: @escaping (Session) -> Void) {
    // AVFoundation callbacks are marshalled even if an OS implementation calls off-main.
    DispatchQueue.main.async { [weak self] in
      guard let self = self, self.synthesizer === engine, let session = self.current,
            session.utterance === utterance else { return }
      action(session)
    }
  }

  func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
    handle(synthesizer, utterance) { [weak self] session in
      self?.startTimeout?.cancel(); self?.startTimeout = nil
      session.offset = max(session.offset, session.chunkStart)
      self?.state(session, speaking: true)
    }
  }
  func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, willSpeakRangeOfSpeechString range: NSRange,
                         utterance: AVSpeechUtterance) {
    handle(synthesizer, utterance) { [weak self] session in
      guard range.location != NSNotFound, range.location <= session.chunkEnd - session.chunkStart else { return }
      session.offset = max(session.offset, session.chunkStart + range.location)
      self?.state(session, speaking: true)
    }
  }
  func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
    handle(synthesizer, utterance) { [weak self] session in
      guard let self = self else { return }
      self.startTimeout?.cancel(); self.startTimeout = nil
      session.offset = session.chunkEnd
      if session.offset < session.text.length { self.queueChunk(session); return }
      self.current = nil
      session.registration.status = .finished
      session.registration.offset = session.offset
      self.deactivateAudio()
      self.state(session, speaking: false)
      self.stopNowPlaying()
    }
  }
  func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
    handle(synthesizer, utterance) { [weak self] _ in self?.forceStop(error: "SPEECH_INTERRUPTED") }
  }

  private var hasNowPlaying: Bool {
    #if canImport(NowPlaying)
    if #available(iOS 27.0, *), nowPlaying != nil { return true }
    #endif
    return false
  }

  private func startNowPlaying(_ session: Session, labels: NativeSpeechLabels?, voice: AVSpeechSynthesisVoice) {
    #if canImport(NowPlaying)
    if #available(iOS 27.0, *) {
      MainActor.assumeIsolated {
        stopNowPlaying()
        let controller = SpeechNowPlaying(
          id: session.id,
          title: labels?.title ?? "Reading aloud",
          subtitle: voice.name,
          onPlay: { [weak self] in _ = self?.resumeCurrent() },
          onPause: { [weak self] in self?.pauseCurrent() },
          onPrevious: { [weak self] in self?.previousCurrent() },
          onStop: { [weak self] in self?.forceStop() })
        nowPlaying = controller
        controller.activate()
      }
    }
    #endif
  }

  private func stopNowPlaying() {
    #if canImport(NowPlaying)
    if #available(iOS 27.0, *) {
      MainActor.assumeIsolated {
        (nowPlaying as? SpeechNowPlaying)?.stop()
        nowPlaying = nil
      }
    }
    #endif
  }

  private func deactivateAudio() {
    guard ownsAudioSession else { return }
    ownsAudioSession = false
    _ = setSpeechActive(false)
  }
  private func state(_ session: Session, speaking: Bool, error: String? = nil) {
    var data: JSObject = ["id": session.id, "speaking": speaking,
                          "paused": session.paused, "offset": session.offset]
    if let error = error { data["error"] = error }
    #if canImport(NowPlaying)
    if #available(iOS 27.0, *) {
      MainActor.assumeIsolated {
        (nowPlaying as? SpeechNowPlaying)?.setPlaying(speaking && !session.paused)
      }
    }
    #endif
    emit(data)
  }
  private func validID(_ id: String) -> Bool {
    id.utf8.count == 36 && UUID(uuidString: id)?.uuidString.caseInsensitiveCompare(id) == .orderedSame
  }
  private func validLanguage(_ language: String) -> Bool {
    language.utf8.count <= 128 && Self.languagePattern.firstMatch(in: language, range: NSRange(language.startIndex..., in: language)) != nil
  }
  private func reject(_ invoke: Invoke, _ code: String) { invoke.reject(code, code: code) }
  private func unavailable(_ invoke: Invoke, _ error: String) {
    invoke.resolve(["available": false, "voices": [], "error": error])
  }
}
