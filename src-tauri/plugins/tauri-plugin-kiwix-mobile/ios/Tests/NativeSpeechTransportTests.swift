import AVFoundation
import Foundation
import Tauri
import XCTest
@testable import KiwixMobile

private final class RecordingSynthesizer: AVSpeechSynthesizer {
  var utterances: [AVSpeechUtterance] = []
  override func speak(_ utterance: AVSpeechUtterance) { utterances.append(utterance) }
  override func stopSpeaking(at boundary: AVSpeechBoundary) -> Bool { true }
}

final class NativeSpeechTransportTests: XCTestCase {
  private func invocation(_ response: @escaping (UInt64, [String: Any]) -> Void) -> Invoke {
    Invoke(command: "test", callback: 1, error: 2, sendResponse: { callback, text in
      let result = text.flatMap { $0.data(using: .utf8) }
        .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
      response(callback, result)
    }, sendChannelData: { _, _ in }, data: "{}")
  }

  @MainActor
  func testPauseResumePreviousIgnoreRetiredUtteranceCallbacks() async throws {
    let voice = try XCTUnwrap(AVSpeechSynthesisVoice(language: "en-US"))
    let synth = RecordingSynthesizer()
    var states: [JSObject] = []
    var activation: [Bool] = []
    var admission = true
    let speech = NativeSpeech(foreground: { true }, setSpeechActive: { active in
      activation.append(active); return !active || admission
    }, makeSynthesizer: { synth }, availableVoices: { [voice] }, emit: { states.append($0) })
    defer { speech.shutdown() }
    let id = UUID().uuidString
    var token = ""
    speech.prepare(NativeSpeechPrepareArgs(id: id), invoke: invocation { callback, reply in
      XCTAssertEqual(callback, 1); token = reply["token"] as? String ?? ""
    })
    let args = NativeSpeechSessionArgs(id: id, token: token)
    let text = "First paragraph.\nSecond paragraph.\nThird paragraph."
    let accept = invocation { callback, _ in XCTAssertEqual(callback, 1) }
    speech.start(NativeSpeechStartArgs(id: id, token: token, text: text,
      lang: voice.language, voice: voice.identifier, rate: 1), invoke: accept)
    let original = try XCTUnwrap(synth.utterances.last)
    let offset = (text as NSString).range(of: "Third").location
    speech.speechSynthesizer(synth, willSpeakRangeOfSpeechString: NSRange(location: offset, length: 5), utterance: original)
    await withCheckedContinuation { continuation in DispatchQueue.main.async { continuation.resume() } }
    speech.pause(args, invoke: accept)
    XCTAssertEqual(states.last?["paused"] as? Bool, true)
    XCTAssertEqual(states.last?["speaking"] as? Bool, false)
    XCTAssertEqual(activation, [true, false])
    speech.speechSynthesizer(synth, didFinish: original)
    speech.speechSynthesizer(synth, didCancel: original)
    await withCheckedContinuation { continuation in DispatchQueue.main.async { continuation.resume() } }
    XCTAssertEqual(synth.utterances.count, 1, "Old completion cannot start another chunk while paused")
    XCTAssertEqual(states.last?["offset"] as? Int, offset)
    admission = false
    speech.resume(args, invoke: invocation { callback, reply in
      XCTAssertEqual(callback, 2); XCTAssertEqual(reply["code"] as? String, "SPEECH_AUDIO_SESSION_FAILED")
    })
    XCTAssertEqual(states.last?["paused"] as? Bool, true)
    admission = true
    speech.previous(args, invoke: accept)
    XCTAssertEqual(states.last?["paused"] as? Bool, true)
    let previous = (text as NSString).range(of: "Second").location
    XCTAssertEqual(states.last?["offset"] as? Int, previous)
    speech.resume(args, invoke: accept)
    XCTAssertEqual(synth.utterances.last?.speechString, String(text.dropFirst(previous)))
    let resumed = try XCTUnwrap(synth.utterances.last)
    speech.speechSynthesizer(synth, didStart: resumed)
    speech.speechSynthesizer(synth, didCancel: original)
    await withCheckedContinuation { continuation in DispatchQueue.main.async { continuation.resume() } }
    XCTAssertEqual(states.last?["speaking"] as? Bool, true)
    XCTAssertEqual(states.last?["paused"] as? Bool, false)
    let count = states.count
    speech.previous(args, invoke: accept)
    XCTAssertEqual(states.count, count,
      "An active rewind must not emit terminal speaking=false and make the shell retire its session")
    XCTAssertEqual(synth.utterances.last?.speechString, text)
    let rewound = try XCTUnwrap(synth.utterances.last)
    speech.speechSynthesizer(synth, didCancel: resumed)
    speech.speechSynthesizer(synth, didStart: rewound)
    await withCheckedContinuation { continuation in DispatchQueue.main.async { continuation.resume() } }
    XCTAssertEqual(states.last?["speaking"] as? Bool, true)
    speech.stop(args, invoke: accept)
    speech.resume(args, invoke: invocation { callback, reply in
      XCTAssertEqual(callback, 2); XCTAssertEqual(reply["code"] as? String, "SPEECH_SESSION_UNREGISTERED")
    })
    XCTAssertEqual(synth.utterances.count, 3)
  }
}
