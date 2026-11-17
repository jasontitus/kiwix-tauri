import AVFoundation
import Foundation
import Tauri

/// WebKit owns activation for archive audio/video. The app installs the
/// background-playback category before loading media and only activates the
/// session itself for native read-aloud.
///
/// A DOM media-state notification arrives after WebKit has started playback.
/// It is bookkeeping, not a request to take the audio session from WebKit.
final class ReaderAudioSession {
  private let emit: (JSObject) -> Void
  private var mediaActive = false
  private var speechActive = false
  private var speechPaused = false
  private var speechOwnsActivation = false
  private var observations: [NSObjectProtocol] = []

  init(emit: @escaping (JSObject) -> Void) {
    self.emit = emit
    let center = NotificationCenter.default
    observations.append(center.addObserver(
      forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
    ) { [weak self] notification in
      self?.interruption(notification)
    })
    observations.append(center.addObserver(
      forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main
    ) { [weak self] notification in
      self?.routeChange(notification)
    })
  }

  deinit {
    observations.forEach { NotificationCenter.default.removeObserver($0) }
  }

  func installCategory() {
    do {
      try AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback, options: [])
    } catch {
      failed(error)
    }
  }

  @discardableResult
  func setMediaActive(_ active: Bool, release: Bool = false) -> Bool {
    mediaActive = active && !release
    // A finished speech session may still be waiting for the shell's final
    // audible=false report (which includes speech). Release only our claim.
    if !mediaActive && !speechActive { return releaseSpeechActivation() }
    return true
  }

  @discardableResult
  func setSpeechActive(_ active: Bool) -> Bool {
    if speechActive == active {
      if !active { return releaseSpeechActivation() }
      if speechPaused {
        do {
          try AVAudioSession.sharedInstance().setCategory(
            .playback, mode: .spokenAudio, options: .duckOthers)
          try AVAudioSession.sharedInstance().setActive(true)
          speechPaused = false
          return true
        } catch {
          failed(error)
          return false
        }
      }
      return true
    }
    let audio = AVAudioSession.sharedInstance()
    do {
      if active {
        try audio.setCategory(.playback, mode: .spokenAudio, options: .duckOthers)
        try audio.setActive(true)
        speechOwnsActivation = true
        speechActive = true
        speechPaused = false
      } else {
        speechActive = false
        speechPaused = false
        // Native speech is finished, but the asynchronous shell report may
        // still include it. Keep our claim until that report becomes quiet.
        try audio.setCategory(.playback, mode: .moviePlayback, options: [])
        return releaseSpeechActivation()
      }
      return true
    } catch {
      failed(error)
      return false
    }
  }

  /// Keep the app alive for Lock Screen resume without ducking other audio.
  @discardableResult
  func setSpeechPaused(_ paused: Bool) -> Bool {
    if !paused { return setSpeechActive(true) }
    guard speechActive else { return true }
    do {
      try AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback, options: [])
      speechPaused = true
      return true
    } catch {
      failed(error)
      return false
    }
  }

  func shutdown() {
    mediaActive = false
    _ = setSpeechActive(false)
  }

  private func interruption(_ notification: Notification) {
    guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
          let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
    switch type {
    case .began:
      mediaActive = false
      speechOwnsActivation = false
      emit(["event": "interruption-began"])
    case .ended:
      let rawOptions = notification.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
      let shouldResume = AVAudioSession.InterruptionOptions(rawValue: rawOptions).contains(.shouldResume)
      emit(["event": "interruption-ended", "shouldResume": shouldResume])
      // WebKit decides when its media session may resume. No reactivation,
      // category reinstall, or notification debounce on this path.
    @unknown default:
      break
    }
  }

  private func routeChange(_ notification: Notification) {
    guard let raw = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
          let reason = AVAudioSession.RouteChangeReason(rawValue: raw) else { return }
    if reason == .oldDeviceUnavailable {
      mediaActive = false
      emit(["event": "route-unavailable"])
    }
    // Category/route notifications do not mean activation was lost. Reasserting
    // activation here feeds our own category changes back into setActive.
  }

  private func failed(_ error: Error) {
    emit(["event": "session-failed", "message": error.localizedDescription])
  }

  private func releaseSpeechActivation() -> Bool {
    guard speechOwnsActivation && !speechActive && !mediaActive else { return true }
    speechPaused = false
    do {
      try AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
      speechOwnsActivation = false
      return true
    } catch {
      failed(error)
      return false
    }
  }
}
