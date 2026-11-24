import Foundation
import MediaPlayer

@MainActor
protocol SpeechNowPlayingController: AnyObject {
  func activate()
  func setPlaying(_ playing: Bool)
  func stop()
}

@MainActor
func makeSpeechNowPlaying(id: String, title: String, subtitle: String?, advanced: Bool,
                          action: @escaping @MainActor (String) -> Void) -> any SpeechNowPlayingController {
  #if canImport(NowPlaying)
  if #available(iOS 27.0, *) {
    return SpeechNowPlaying(id: id, title: title, subtitle: subtitle, advanced: advanced, action: action)
  }
  #endif
  return LegacySpeechNowPlaying(title: title, subtitle: subtitle, advanced: advanced, action: action)
}

@MainActor
private final class LegacySpeechNowPlaying: SpeechNowPlayingController {
  private static weak var owner: LegacySpeechNowPlaying?
  private let title: String
  private let subtitle: String?
  private let advanced: Bool
  private let action: @MainActor (String) -> Void
  private var targets: [(MPRemoteCommand, Any, Bool)] = []
  private var playing = false

  init(title: String, subtitle: String?, advanced: Bool, action: @escaping @MainActor (String) -> Void) {
    self.title = title; self.subtitle = subtitle; self.advanced = advanced; self.action = action
  }

  func activate() {
    Self.owner?.stop()
    Self.owner = self
    let commands = MPRemoteCommandCenter.shared()
    add(commands.playCommand, "play")
    add(commands.pauseCommand, "pause")
    add(commands.togglePlayPauseCommand, "toggle")
    add(commands.previousTrackCommand, "previous")
    add(commands.stopCommand, "stop")
    if advanced {
      commands.skipBackwardCommand.preferredIntervals = [15]
      commands.skipForwardCommand.preferredIntervals = [15]
      add(commands.nextTrackCommand, "next")
      add(commands.skipBackwardCommand, "rewind")
      add(commands.skipForwardCommand, "forward")
    }
    publish()
  }

  private func add(_ command: MPRemoteCommand, _ name: String) {
    let wasEnabled = command.isEnabled
    command.isEnabled = true
    let token = command.addTarget { [weak self] _ in
      Task { @MainActor in
        guard let self, Self.owner === self else { return }
        self.action(name == "toggle" ? (self.playing ? "pause" : "play") : name)
      }
      return .success
    }
    targets.append((command, token, wasEnabled))
  }

  private func publish() {
    var metadata: [String: Any] = [
      MPMediaItemPropertyTitle: title,
      MPNowPlayingInfoPropertyPlaybackRate: playing ? 1.0 : 0.0
    ]
    if let subtitle { metadata[MPMediaItemPropertyArtist] = subtitle }
    MPNowPlayingInfoCenter.default().nowPlayingInfo = metadata
  }

  func setPlaying(_ playing: Bool) {
    guard Self.owner === self, playing != self.playing else { return }
    self.playing = playing
    publish()
  }

  func stop() {
    guard Self.owner === self else { return }
    for (command, token, wasEnabled) in targets {
      command.removeTarget(token)
      command.isEnabled = wasEnabled
    }
    targets.removeAll()
    MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    Self.owner = nil
  }
}

#if canImport(NowPlaying)
import NowPlaying
import Observation

/// Publishes read-aloud as a system Now Playing session on iOS 27+.
@available(iOS 27.0, *)
@MainActor
final class SpeechNowPlaying: SpeechNowPlayingController {
  private let model: SpeechNowPlayingModel
  private var session: MediaSession<SpeechNowPlayingModel>?
  private var activation: Task<Void, Never>?

  init(id: String, title: String, subtitle: String?, advanced: Bool, action: @escaping @MainActor (String) -> Void) {
    model = SpeechNowPlayingModel(id: id, title: title, subtitle: subtitle, advanced: advanced, action: action)
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
    guard playing != model.isPlaying else { return }
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
  private let advanced: Bool
  private let action: @MainActor (String) -> Void

  init(id: String, title: String, subtitle: String?, advanced: Bool, action: @escaping @MainActor (String) -> Void) {
    self.id = id
    self.title = title
    self.subtitle = subtitle
    self.advanced = advanced
    self.action = action
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
    var commands: [MediaCommand] = [
      .play { [weak self] in await MainActor.run { self?.action("play") } }.enabled(!isPlaying),
      .pause { [weak self] in await MainActor.run { self?.action("pause") } }.enabled(isPlaying),
      .previous { [weak self] in await MainActor.run { self?.action("previous") } },
      .stop { [weak self] in await MainActor.run { self?.action("stop") } },
    ]
    if advanced {
      commands.append(.next { [weak self] in await MainActor.run { self?.action("next") } })
      commands.append(.skipBackward(preferredIntervals: [15]) { [weak self] _ in
        await MainActor.run { self?.action("rewind") }
      })
      commands.append(.skipForward(preferredIntervals: [15]) { [weak self] _ in
        await MainActor.run { self?.action("forward") }
      })
    }
    return commands
  }
}
#endif
