package org.kiwix.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/** Notification labels supplied by the localized Rust root. */
internal data class SpeechLabels(
    val title: String,
    val pause: String,
    val resume: String,
    val stop: String,
    val previous: String,
    val rewind: String,
    val forward: String,
) {
    companion object {
        private val fallback = SpeechLabels(
            "Reading aloud", "Pause", "Resume", "Stop", "Previous paragraph",
            "Rewind 15 seconds", "Forward 15 seconds",
        )
        fun from(json: org.json.JSONObject?): SpeechLabels {
            if (json == null) return fallback
            fun read(key: String, value: String): String {
                val text = json.optString(key, "").trim()
                return if (text.isEmpty()) value else text.take(256)
            }
            return SpeechLabels(
                read("title", fallback.title), read("pause", fallback.pause),
                read("resume", fallback.resume), read("stop", fallback.stop),
                read("previous", fallback.previous),
                read("rewind", fallback.rewind), read("forward", fallback.forward),
            )
        }
    }
}

/**
 * Application-scoped media presentation for native read-aloud.
 *
 * The foreground service keeps the process alive while the Activity is in the
 * background; the MediaSession owns lock-screen and media-button transport. It
 * is presentation only: every action is reported to Rust, which remains the
 * sole owner of speech admission and forwards the action to the engine.
 */
internal object SpeechPlayback {
    const val ACTION_START = "org.kiwix.mobile.SPEECH_START"
    const val ACTION_PLAY = "org.kiwix.mobile.SPEECH_PLAY"
    const val ACTION_PAUSE = "org.kiwix.mobile.SPEECH_PAUSE"
    const val ACTION_STOP = "org.kiwix.mobile.SPEECH_STOP"
    const val ACTION_PREVIOUS = "org.kiwix.mobile.SPEECH_PREVIOUS"
    const val ACTION_REWIND = "org.kiwix.mobile.SPEECH_REWIND"
    const val ACTION_FORWARD = "org.kiwix.mobile.SPEECH_FORWARD"
    private const val CHANNEL = "kiwix-speech"
    private const val NOTIFICATION = 7402

    private var context: Context? = null
    private var session: MediaSession? = null
    private var labels = SpeechLabels.from(null)
    private var speaking = false
    private var paused = false
    private var canPrevious = false
    private var running = false

    /** Set by the plugin. Routes a transport action back through Rust. */
    var dispatch: ((String) -> Unit)? = null

    fun start(context: Context, labels: SpeechLabels, speaking: Boolean, canPrevious: Boolean): Boolean {
        checkMain()
        val app = context.applicationContext
        this.context = app
        this.labels = labels
        this.speaking = speaking
        this.paused = !speaking
        this.canPrevious = canPrevious
        ensureChannel(app)
        if (!running) {
            running = true
            val intent = Intent(app, KiwixSpeechService::class.java).setAction(ACTION_START)
            // Read-aloud is admitted from a visible reader. startService avoids
            // the five-second startForegroundService contract while the TTS
            // engine performs synchronous binder work on the main thread; the
            // service still enters the foreground in onStartCommand. Background
            // starts (media buttons, restored sessions) fall back to the
            // foreground-service contract.
            try {
                try {
                    app.startService(intent)
                } catch (_: Exception) {
                    ContextCompat.startForegroundService(app, intent)
                }
            } catch (_: Exception) {
                running = false
                this.context = null
                return false
            }
        }
        try {
            publish()
        } catch (_: Exception) {
            running = false
            this.context = null
            app.stopService(Intent(app, KiwixSpeechService::class.java))
            return false
        }
        return true
    }

    fun update(speaking: Boolean, paused: Boolean, canPrevious: Boolean) {
        checkMain()
        if (!running) return
        if (this.speaking == speaking && this.paused == paused && this.canPrevious == canPrevious) return
        this.speaking = speaking
        this.paused = paused
        this.canPrevious = canPrevious
        publish()
    }

    fun stop(context: Context) {
        checkMain()
        val app = context.applicationContext
        if (running) {
            running = false
            NotificationManagerCompat.from(app).cancel(NOTIFICATION)
            releaseSession()
            app.stopService(Intent(app, KiwixSpeechService::class.java))
        }
        this.context = null
    }

    /** Called by the service when it becomes foreground. */
    internal fun foreground(service: Service): Boolean {
        try {
            ensureChannel(service)
            if (!running) {
                // A stale transport intent can still arrive through
                // startForegroundService. The platform contract requires one
                // startForeground call even when the service is about to stop.
                ServiceCompat.startForeground(
                    service, NOTIFICATION, notification(service),
                    if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0,
                )
                service.stopSelf()
                return false
            }
            ServiceCompat.startForeground(
                service, NOTIFICATION, notification(service),
                if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0,
            )
            return true
        } catch (error: Exception) {
            // Android 12+ can reject foreground admission after a background
            // transition. Fail closed: retire the service and let Rust stop
            // the speech session instead of crashing the process.
            running = false
            context = null
            releaseSession()
            service.stopSelf()
            dispatch?.invoke("stop")
            return false
        }
    }

    internal fun dispatchAction(action: String) {
        if (running) dispatch?.invoke(action)
    }

    /** The service disappeared without an explicit stop: release playback. */
    internal fun onServiceDestroyed() {
        if (!running) return
        running = false
        releaseSession()
        dispatch?.invoke("stop")
    }

    internal fun onTaskRemoved() {
        if (!running) return
        dispatchAction("stop")
        running = false
        releaseSession()
    }

    private fun publish(notify: Boolean = true) {
        val app = context ?: return
        val media = ensureSession(app)
        media.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, labels.title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, app.applicationInfo.loadLabel(app.packageManager).toString())
                .build(),
        )
        val state = when {
            speaking -> PlaybackState.STATE_PLAYING
            paused -> PlaybackState.STATE_PAUSED
            else -> PlaybackState.STATE_STOPPED
        }
        val actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_STOP or
            PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_REWIND or PlaybackState.ACTION_FAST_FORWARD or
            if (canPrevious) PlaybackState.ACTION_SKIP_TO_PREVIOUS else 0L
        media.setPlaybackState(
            PlaybackState.Builder().setActions(actions).setState(state, 0, 1f).build(),
        )
        if (notify) NotificationManagerCompat.from(app).notify(NOTIFICATION, notification(app))
    }

    private fun ensureSession(context: Context): MediaSession {
        session?.let { return it }
        val created = MediaSession(context, "KiwixReadAloud").apply {
            setFlags(
                MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or
                    MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS,
            )
            setCallback(object : MediaSession.Callback() {
                override fun onPlay() = dispatchAction("play")
                override fun onPause() = dispatchAction("pause")
                override fun onStop() = dispatchAction("stop")
                override fun onSkipToPrevious() = dispatchAction("previous")
                override fun onSkipToNext() = dispatchAction("next")
                override fun onRewind() = dispatchAction("rewind")
                override fun onFastForward() = dispatchAction("forward")
            })
            isActive = true
        }
        session = created
        return created
    }

    private fun releaseSession() {
        session?.let {
            it.isActive = false
            it.release()
        }
        session = null
    }

    private fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, labels.title, NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }

    private fun notification(context: Context): Notification {
        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(context, CHANNEL)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(context)
        }
        val style = Notification.MediaStyle().setShowActionsInCompactView(0, 1, 2)
        session?.sessionToken?.let { style.setMediaSession(it) }
        builder
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(context.applicationInfo.loadLabel(context.packageManager).toString())
            .setContentText(labels.title)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setStyle(style)
            .addAction(action(context, android.R.drawable.ic_media_previous, labels.previous, ACTION_PREVIOUS, 1))
            .addAction(
                action(
                    context,
                    if (speaking) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play,
                    if (speaking) labels.pause else labels.resume,
                    if (speaking) ACTION_PAUSE else ACTION_PLAY,
                    2,
                ),
            )
            .addAction(action(context, android.R.drawable.ic_menu_close_clear_cancel, labels.stop, ACTION_STOP, 3))
            .addAction(action(context, android.R.drawable.ic_media_rew, labels.rewind, ACTION_REWIND, 4))
            .addAction(action(context, android.R.drawable.ic_media_ff, labels.forward, ACTION_FORWARD, 5))
        context.packageManager.getLaunchIntentForPackage(context.packageName)?.let { launch ->
            builder.setContentIntent(
                PendingIntent.getActivity(
                    context, 0, launch,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
        }
        return builder.build()
    }

    private fun action(
        context: Context, icon: Int, title: String, action: String, request: Int,
    ): Notification.Action {
        val intent = Intent(context, KiwixSpeechService::class.java).setAction(action)
        val pending = PendingIntent.getService(
            context, request, intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Action.Builder(Icon.createWithResource(context, icon), title, pending).build()
    }

    private fun checkMain() {
        check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) {
            "SPEECH_PLAYBACK_REQUIRES_MAIN_THREAD"
        }
    }
}
