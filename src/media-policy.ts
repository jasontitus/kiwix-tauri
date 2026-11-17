/**
 * Background media exemption for the mobile reader.
 *
 * A mobile suspension normally retires reader storage (`freezeReaderStorage`),
 * which navigates the archive document away. That is correct for privacy,
 * read-only, shutdown and ordinary reading, but it also destroys an audible
 * media element or an active Picture-in-Picture presentation. Keep the
 * document alive only for the bounded suspension window while one of those
 * user-started surfaces is actually active.
 */
export type ReaderMediaState = {
  /** An unmuted audio/video element is playing. */
  audible: boolean;
  /** A video is currently presented in Picture-in-Picture. */
  pictureInPicture: boolean;
};

export type BackgroundAudioDecision = ReaderMediaState & {
  privateMode: boolean;
  readOnly: boolean;
  /** Privacy retirement, a tab/window close or another explicit retirement. */
  retiring: boolean;
  /** Application shutdown must never be held open by media. */
  quitting: boolean;
};

/** Whether a mobile suspension may keep the reader document alive. */
export function shouldKeepReaderForBackgroundMedia(
  state: BackgroundAudioDecision,
): boolean {
  if (state.privateMode || state.readOnly || state.retiring || state.quitting)
    return false;
  return state.audible || state.pictureInPicture;
}
