#if canImport(NowPlaying)
import Foundation
import NowPlaying
import Observation

/// Publishes read-aloud as a system Now Playing session on iOS 27+.
@available(iOS 27.0, *)
@MainActor
final class SpeechNowPlaying {
  private let model: SpeechNowPlayingModel
  private var session: MediaSession<SpeechNowPlayingModel>?
  private var activation: Task<Void, Never>?

  init(id: String, title: String, subtitle: String?, onPlay: @escaping @MainActor () -> Void,
       onPause: @escaping @MainActor () -> Void, onPrevious: @escaping @MainActor () -> Void,
       onStop: @escaping @MainActor () -> Void) {
    model = SpeechNowPlayingModel(
      id: id, title: title, subtitle: subtitle, onPlay: onPlay, onPause: onPause,
      onPrevious: onPrevious, onStop: onStop)
  }

  func activate() {
    let session = MediaSession(model)
    self.session = session
    activation = Task { [weak self] in
      guard let self, self.session === session, !Task.isCancelled else { return }
      try? await session.requestToBecomeApplicationPrimary()
    }
  }

  func setPlaying(_ playing: Bool) {
    model.isPlaying = playing
    model.timestamp = Date()
  }

  func stop() {
    activation?.cancel()
    activation = nil
    session = nil
  }
}

@available(iOS 27.0, *)
@Observable
@MainActor
final class SpeechNowPlayingModel: MediaSessionRepresentable {
  let id: String
  let title: String
  let subtitle: String?
  var isPlaying = false
  var timestamp = Date()
  private let onPlay: @MainActor () -> Void
  private let onPause: @MainActor () -> Void
  private let onPrevious: @MainActor () -> Void
  private let onStop: @MainActor () -> Void

  init(id: String, title: String, subtitle: String?, onPlay: @escaping @MainActor () -> Void,
       onPause: @escaping @MainActor () -> Void, onPrevious: @escaping @MainActor () -> Void,
       onStop: @escaping @MainActor () -> Void) {
    self.id = id
    self.title = title
    self.subtitle = subtitle
    self.onPlay = onPlay
    self.onPause = onPause
    self.onPrevious = onPrevious
    self.onStop = onStop
  }

  var content: (any MediaContentRepresentable)? {
    GenericContent(
      id: id,
      title: title,
      subtitle: subtitle,
      type: .audio,
      duration: nil,
      artwork: nil)
  }

  var playbackSnapshot: MediaPlaybackSnapshot? {
    MediaPlaybackSnapshot(
      state: isPlaying ? .playing() : .paused,
      elapsedTime: nil,
      timestamp: timestamp)
  }

  var commands: [MediaCommand] {
    [
      .play { [weak self] in
        await MainActor.run { self?.onPlay() }
      }.enabled(!isPlaying),
      .pause { [weak self] in
        await MainActor.run { self?.onPause() }
      }.enabled(isPlaying),
      .previous { [weak self] in
        await MainActor.run { self?.onPrevious() }
      },
      .stop { [weak self] in
        await MainActor.run { self?.onStop() }
      },
    ]
  }
}
#endif
