package org.kiwix.mobile

import android.app.Service
import android.content.Intent
import android.os.IBinder

/**
 * Foreground media-playback host for native read-aloud.
 *
 * It owns no speech state and never touches the engine directly: transport
 * intents and media buttons are reported to Rust, which validates ownership
 * and issues the ordinary speech command. Stopping the task retires playback.
 */
class KiwixSpeechService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Every delivery, including a transport action, may arrive through
        // startForegroundService. Satisfy that contract before dispatching.
        if (!SpeechPlayback.foreground(this)) return START_NOT_STICKY
        when (intent?.action) {
            SpeechPlayback.ACTION_START, null -> Unit
            SpeechPlayback.ACTION_PLAY -> SpeechPlayback.dispatchAction("play")
            SpeechPlayback.ACTION_PAUSE -> SpeechPlayback.dispatchAction("pause")
            SpeechPlayback.ACTION_STOP -> SpeechPlayback.dispatchAction("stop")
            SpeechPlayback.ACTION_PREVIOUS -> SpeechPlayback.dispatchAction("previous")
            SpeechPlayback.ACTION_REWIND -> SpeechPlayback.dispatchAction("rewind")
            SpeechPlayback.ACTION_FORWARD -> SpeechPlayback.dispatchAction("forward")
            else -> return START_NOT_STICKY
        }
        return START_NOT_STICKY
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        SpeechPlayback.onTaskRemoved()
        stopSelf()
    }

    override fun onDestroy() {
        try {
            SpeechPlayback.onServiceDestroyed()
        } finally {
            super.onDestroy()
        }
    }
}
