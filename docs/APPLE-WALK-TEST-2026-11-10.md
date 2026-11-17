# iPhone 17 walk test — 2026-11-10

Device: iPhone 17 Pro Max test device, UDID `<test-iphone-17-udid>`.
App: Kiwix `com.tiltastech.zimreader` 0.1.0.

Fixtures staged in the app container `Documents/`:

- `walk-test.zim` — 180 s MP3 and AAC audio; 120 s H.264/AAC video; 30 s VP8/VP9 WebM.
- `media-test.zim` — short codec fixture.
- `hawaii.zim` — real StreetZim Hawaii.

At handoff, iOS showed **“Add walk-test.zim to your library?”** with **Choose files…**. Import each fixture (or choose all three) before walking.

## Checks

### Audio / background
- Play MP3 and M4A; seek, pause, resume.
- Lock the phone; wait 10 s; confirm audio continues.
- Lock screen / Control Center: play/pause/scrub; no duplicate session.
- Unplug headphones or disconnect Bluetooth; record pause/route behavior.
- Trigger Siri/incoming call; record whether playback resumes.

### Video / PiP
- Play 120 s H.264/AAC; fullscreen, rotate, scrub.
- Swipe Home during playback; PiP should appear and keep playing.
- Lock while PiP; use PiP controls; return to Kiwix.
- Try VP8 and VP9 WebM; record support/error.

### StreetZim
- Open Hawaii; confirm map canvas, search, routing.
- Test location/camera prompts and image-recognition controls if present.
- Confirm normal mode is unaffected by restricted mode; optionally toggle restricted and verify the map/scripts stop, then leave restricted.

Record Pass/Fail/Blocked per check with screen recording for lock-screen/PiP/interruption tests.
