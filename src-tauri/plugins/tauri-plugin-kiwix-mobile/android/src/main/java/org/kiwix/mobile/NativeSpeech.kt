package org.kiwix.mobile

import android.content.Context
import android.media.AudioAttributes
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.speech.tts.Voice
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.util.IllformedLocaleException
import java.util.Locale
import java.util.UUID

/** Main-thread owned. Only the trusted Rust mobile transport calls these methods. */
internal class NativeSpeech(
    private val context: Context,
    private val foreground: () -> Boolean,
    private val emit: (JSObject) -> Unit
) {
    private val main = Handler(Looper.getMainLooper())
    private var engine: TextToSpeech? = null
    private var ready = false
    private var initializing = false
    private var engineGeneration = 0L
    private var shutdownError: String? = null
    private var initTimeout: Runnable? = null
    private var startTimeout: Runnable? = null
    private val voiceRequests = mutableListOf<Invoke>()
    private var nextToken = 0L
    private var nextRegistration = 0L
    private var latestStarted = 0L
    private var current: Session? = null
    private val registrations = mutableMapOf<String, Registration>()
    private var labels = SpeechLabels.from(null)
    private var lastState: JSObject? = null

    private enum class Status { PREPARED, STARTED, CANCELLED, FINISHED }
    private class Registration(val id: String, val capability: String, val ordinal: Long) {
        var status = Status.PREPARED
        var offset = 0
    }
    private class Session(
        val registration: Registration, val token: Long, val text: String, val language: Locale,
        val voice: String, val rate: Float, val labels: SpeechLabels, var invoke: Invoke?
    ) {
        val id: String get() = registration.id
        var offset = 0
        var chunkStart = 0
        var chunkEnd = 0
        var utterance = ""
        var paused = false
        var utteranceRevision = 0L
        var progressAt = 0L
        var progressOffset = 0
        var stateAt = 0L
        var charactersPerSecond = 15.0 * rate
    }

    fun voices(invoke: Invoke) {
        if (!foreground()) { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
        if (ready) { resolveVoices(invoke); return }
        if (voiceRequests.size >= 16) { reject(invoke, "SPEECH_BUSY"); return }
        voiceRequests.add(invoke)
        ensureEngine()
    }

    fun prepare(invoke: Invoke) {
        val id = try { checkedId(invoke.getArgs().getString("id")) }
            catch (_: Exception) { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
        if (!foreground()) { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
        if (registrations.containsKey(id)) { reject(invoke, "SPEECH_SESSION_REGISTERED"); return }
        if (registrations.size >= 8) { reject(invoke, "SPEECH_BUSY"); return }
        val registration = Registration(id, UUID.randomUUID().toString(), ++nextRegistration)
        registrations[id] = registration
        invoke.resolve(JSObject().put("id", id).put("token", registration.capability))
    }

    fun start(invoke: Invoke) {
        val request: Session
        try {
            val args = invoke.getArgs()
            val id = checkedId(args.getString("id"))
            val capability = checkedId(args.getString("token"))
            val registration = registrations[id]?.takeIf { it.capability == capability }
            if (registration == null) { reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return }
            if (registration.status != Status.PREPARED) { reject(invoke, "SPEECH_SESSION_CONSUMED"); return }
            if (registration.ordinal < latestStarted) {
                registration.status = Status.CANCELLED
                reject(invoke, "SPEECH_SESSION_SUPERSEDED"); return
            }
            val text = args.getString("text")
            val language = checkedLanguage(args.getString("lang"))
            val voice = args.getString("voice")
            val rate = args.getDouble("rate")
            val labels = SpeechLabels.from(args.optJSONObject("labels"))
            if (text.length !in 1..1_000_000 || text.isBlank() || voice.length > 4096 ||
                !rate.isFinite() || rate < 0.25 || rate > 2.0) throw IllegalArgumentException()
            request = Session(registration, ++nextToken, text, language, voice, rate.toFloat(), labels, invoke)
        } catch (_: Exception) { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
        if (!foreground()) { reject(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
        // Consume the prepared capability before touching existing playback. A cancelled,
        // retired, or duplicate Start can never replace a newer session.
        request.registration.status = Status.STARTED
        latestStarted = request.registration.ordinal
        labels = request.labels
        forceStop(keepPlayback = true)
        current = request
        // The media service is foreground admission for the whole session, so
        // it starts before engine initialization can wait on the system TTS.
        if (!SpeechPlayback.start(context, labels, speaking = true, canPrevious = false)) {
            finish(request, "SPEECH_FOREGROUND_REQUIRED")
            return
        }
        // Let the playback service reach onStartCommand before the TTS engine's
        // synchronous binder work can occupy the main thread.
        main.post {
            if (current !== request) return@post
            if (ready) begin(request) else ensureEngine()
        }
    }

    fun stop(invoke: Invoke) {
        val id: String
        val capability: String
        try {
            val args = invoke.getArgs()
            id = checkedId(args.getString("id"))
            capability = checkedId(args.getString("token"))
        } catch (_: Exception) { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
        val registration = registrations[id]?.takeIf { it.capability == capability }
        if (registration != null) {
            if (current?.registration === registration) forceStop()
            else if (registration.status == Status.PREPARED) registration.status = Status.CANCELLED
        }
        invoke.resolve(JSObject().put("id", id).put("speaking", false)
            .put("offset", registration?.offset ?: 0).put("stopped", registration != null))
    }

    fun retire(invoke: Invoke) {
        val id: String
        val capability: String
        try {
            val args = invoke.getArgs()
            id = checkedId(args.getString("id"))
            capability = checkedId(args.getString("token"))
        } catch (_: Exception) { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
        val registration = registrations[id]?.takeIf { it.capability == capability }
        if (registration != null) {
            registrations.remove(id)
            if (current?.registration === registration) forceStop()
        }
        invoke.resolve(JSObject().put("id", id).put("retired", registration != null))
    }

    fun pause(invoke: Invoke) {
        val session = try { ownedSession(invoke) } catch (_: Exception) { null }
        if (session == null) { reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return }
        if (session.paused) {
            invoke.resolve(JSObject().put("id", session.id).put("paused", true).put("offset", session.offset))
            return
        }
        session.paused = true
        clearStartTimeout()
        val stopError = haltEngine()
        if (stopError != null) {
            finish(session, stopError)
            reject(invoke, stopError)
            return
        }
        session.registration.offset = session.offset
        // A transport request may arrive while engine initialization owns Start.
        // Settle that admission before pausing; never overwrite its callback.
        session.invoke?.resolve(JSObject().put("id", session.id).put("accepted", true))
        session.invoke = null
        state(session, false, paused = true)
        invoke.resolve(JSObject().put("id", session.id).put("paused", true).put("offset", session.offset))
    }

    fun resume(invoke: Invoke) {
        val session = try { ownedSession(invoke) } catch (_: Exception) { null }
        if (session == null) { reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return }
        if (!session.paused) {
            invoke.resolve(JSObject().put("id", session.id).put("paused", false).put("offset", session.offset))
            return
        }
        session.paused = false
        // TTS discards the interrupted utterance; resume from the last engine
        // progress offset reported before pause.
        session.chunkStart = session.offset
        session.chunkEnd = session.offset
        session.invoke = invoke
        if (ready) begin(session) else ensureEngine()
    }

    fun previous(invoke: Invoke) {
        val session = try { ownedSession(invoke) } catch (_: Exception) { null }
        if (session == null) { reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return }
        reposition(session, SpeechPosition.previous(session.text, session.offset), invoke)
    }

    fun next(invoke: Invoke) = move(invoke, "next")
    fun rewind(invoke: Invoke) = move(invoke, "rewind")
    fun forward(invoke: Invoke) = move(invoke, "forward")
    fun seek(invoke: Invoke) = move(invoke, "seek")

    private fun move(invoke: Invoke, action: String) {
        val session = try { ownedSession(invoke) } catch (_: Exception) { null }
        if (session == null) { reject(invoke, "SPEECH_SESSION_UNREGISTERED"); return }
        val offset = try {
            val args = invoke.getArgs()
            when (action) {
                "next" -> SpeechPosition.next(session.text, session.offset)
                "seek" -> {
                    val value = args.getDouble("offset")
                    require(value.isFinite() && value >= 0 && value <= session.text.length && value % 1.0 == 0.0)
                    SpeechPosition.boundary(session.text, value.toInt())
                }
                else -> {
                    val seconds = args.optDouble("seconds", 15.0)
                    require(seconds.isFinite() && seconds > 0 && seconds <= 300)
                    SpeechPosition.timed(session.text, session.offset,
                        if (action == "rewind") -seconds else seconds, session.charactersPerSecond)
                }
            }
        } catch (_: Exception) { reject(invoke, "SPEECH_INVALID_REQUEST"); return }
        reposition(session, offset, invoke)
    }

    private fun reposition(session: Session, offset: Int, invoke: Invoke) {
        session.invoke?.resolve(JSObject().put("id", session.id).put("accepted", true))
        session.invoke = null
        // Invalidate before stop, including seeks to the same chunk. A delayed
        // cancellation/range callback cannot claim the replacement utterance.
        session.utterance = ""
        session.progressAt = 0
        session.offset = offset
        session.chunkStart = offset
        session.chunkEnd = offset
        session.registration.offset = offset
        if (offset == session.text.length) {
            val stopError = haltEngine()
            invoke.resolve(JSObject().put("id", session.id).put("offset", offset).put("paused", false))
            finish(session, stopError)
            return
        }
        if (session.paused) {
            state(session, false, paused = true)
            invoke.resolve(JSObject().put("id", session.id).put("offset", offset).put("paused", true))
            return
        }
        // Ignore the engine's cancellation callback for the discarded utterance.
        session.utterance = ""
        clearStartTimeout()
        session.invoke = invoke
        if (!ready) { ensureEngine(); return }
        val stopError = haltEngine()
        if (stopError != null) {
            finish(session, stopError)
            reject(invoke, stopError)
            return
        }
        begin(session)
    }

    fun reemit() {
        lastState?.let { emit(it) }
    }

    fun cancelAll(error: String) {
        forceStop(error)
        // Unknown capabilities reject Start too: clearing does not permit resurrection.
        registrations.clear()
    }

    fun forceStop(error: String? = null, keepPlayback: Boolean = false) {
        val session = current ?: return
        // Invalidate before stop(): engines may synchronously deliver cancellation callbacks.
        current = null
        clearStartTimeout()
        session.registration.status = Status.CANCELLED
        session.registration.offset = session.offset
        session.invoke?.let { reject(it, error ?: "SPEECH_CANCELLED") }
        session.invoke = null
        val stopError = haltEngine()
        state(session, false, error ?: stopError)
        if (!keepPlayback) SpeechPlayback.stop(context)
    }

    fun shutdown(): String? {
        cancelAll("SPEECH_ENGINE_CLOSED")
        ++engineGeneration
        ready = false
        initializing = false
        initTimeout?.let { main.removeCallbacks(it) }
        initTimeout = null
        val pending = voiceRequests.toList()
        voiceRequests.clear()
        pending.forEach { unavailable(it, "SPEECH_ENGINE_CLOSED") }
        val old = engine
        engine = null
        try { old?.shutdown() } catch (error: Exception) { shutdownError = error.toString() }
        return shutdownError
    }

    private fun ensureEngine() {
        if (ready || initializing) return
        shutdownError = null
        initializing = true
        val generation = ++engineGeneration
        val timeout = Runnable { finishInit(generation, TextToSpeech.ERROR, "SPEECH_INIT_TIMEOUT") }
        initTimeout = timeout
        main.postDelayed(timeout, 15_000)
        try {
            engine = TextToSpeech(context) { status -> main.post { finishInit(generation, status) } }
        } catch (_: Exception) { finishInit(generation, TextToSpeech.ERROR) }
    }

    private fun finishInit(generation: Long, status: Int, failure: String = "SPEECH_INIT_FAILED") {
        if (generation != engineGeneration || !initializing) return
        initializing = false
        initTimeout?.let { main.removeCallbacks(it) }
        initTimeout = null
        val tts = engine
        var success = status == TextToSpeech.SUCCESS && tts != null
        if (success && tts != null) {
            try {
                success = tts.setOnUtteranceProgressListener(listener) == TextToSpeech.SUCCESS &&
                    tts.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()) == TextToSpeech.SUCCESS
            } catch (_: Exception) { success = false }
        }
        ready = success
        if (!success) {
            engine = null
            try { tts?.shutdown() } catch (_: Exception) { }
        }
        val pending = voiceRequests.toList()
        voiceRequests.clear()
        pending.forEach { if (success) resolveVoices(it) else unavailable(it, failure) }
        val session = current ?: return
        if (!success) finish(session, failure)
        else if (!session.paused) begin(session)
    }

    private var voiceCacheGeneration = -1L
    private var voiceCacheAt = 0L
    private var voiceCache: List<Voice> = emptyList()

    /** The engine's voice list is a binder call: read it once per engine, not
     * on every start/resume/seek/next/previous. Re-enumerate after ten seconds
     * so a voice installed or removed in system settings becomes visible. */
    private fun installedVoices(): List<Voice> {
        if (voiceCacheGeneration == engineGeneration &&
            System.currentTimeMillis() - voiceCacheAt < 10_000L) return voiceCache
        val voices = (engine?.voices ?: emptySet()).filter {
            !it.isNetworkConnectionRequired &&
                !it.features.orEmpty().contains(TextToSpeech.Engine.KEY_FEATURE_NOT_INSTALLED) &&
                it.name.isNotEmpty() && it.locale.language.isNotEmpty() && it.locale.language != "und"
        }.sortedWith(compareBy<Voice> { it.locale.toLanguageTag() }.thenBy { it.name })
        voiceCache = voices
        voiceCacheGeneration = engineGeneration
        voiceCacheAt = System.currentTimeMillis()
        return voices
    }

    private fun resolveVoices(invoke: Invoke) {
        if (!foreground()) { unavailable(invoke, "SPEECH_FOREGROUND_REQUIRED"); return }
        try {
            val voices = installedVoices()
            val preferred = engine?.defaultVoice?.name
            val default = voices.firstOrNull { it.name == preferred }?.name ?: voices.firstOrNull()?.name
            val result = JSArray()
            voices.forEach { voice -> result.put(JSObject().put("name", voice.name)
                .put("lang", voice.locale.toLanguageTag()).put("voiceURI", voice.name)
                .put("default", voice.name == default).put("quality", 0)
                .put("localService", true)) }
            val reply = JSObject().put("available", voices.isNotEmpty()).put("voices", result)
            if (voices.isEmpty()) reply.put("error", "SPEECH_OFFLINE_VOICES_UNAVAILABLE")
            invoke.resolve(reply)
        } catch (_: Exception) { unavailable(invoke, "SPEECH_VOICES_FAILED") }
    }

    private fun begin(session: Session) {
        if (current !== session) return
        try {
            val voices = installedVoices()
            val preferred = engine?.defaultVoice?.name
            val deviceLanguage = Locale.getDefault().toLanguageTag()
            val voice = if (session.voice.isNotEmpty()) voices.firstOrNull { it.name == session.voice }
                else voices.firstOrNull {
                    it.name == preferred &&
                        it.locale.toLanguageTag().equals(session.language.toLanguageTag(), true)
                }
                    ?: voices.firstOrNull { it.locale.toLanguageTag().equals(session.language.toLanguageTag(), true) }
                    ?: voices.firstOrNull {
                        it.locale.toLanguageTag().equals(deviceLanguage, true) &&
                            it.locale.language == session.language.language
                    }
                    ?: voices.firstOrNull { it.locale.language == session.language.language }
            if (voice == null) {
                finish(session, if (session.voice.isEmpty()) "SPEECH_LANGUAGE_UNAVAILABLE" else "SPEECH_VOICE_UNAVAILABLE")
                return
            }
            val tts = engine ?: throw IllegalStateException()
            if (tts.setVoice(voice) != TextToSpeech.SUCCESS) { finish(session, "SPEECH_VOICE_UNAVAILABLE"); return }
            if (tts.setSpeechRate(session.rate) != TextToSpeech.SUCCESS) { finish(session, "SPEECH_RATE_FAILED"); return }
            if (queueChunk(session)) {
                state(session, true)
                session.invoke?.resolve(JSObject().put("id", session.id).put("accepted", true))
                session.invoke = null
            }
        } catch (_: Exception) { finish(session, "SPEECH_ENGINE_FAILED") }
    }

    private fun queueChunk(session: Session): Boolean {
        if (current !== session) return false
        // Some engines reject a chunk exactly equal to the advertised maximum
        // (Ivona on Fire OS logs "Text too long" for 4000 characters). Leave a
        // small margin so the advertised limit remains inclusive-safe.
        val maximum = TextToSpeech.getMaxSpeechInputLength()
        val limit = if (maximum > 200) maximum - 100 else maximum
        if (limit < 2) { finish(session, "SPEECH_INPUT_LIMIT_UNAVAILABLE"); return false }
        session.paused = false
        session.chunkStart = session.chunkEnd
        var end = minOf(session.text.length, session.chunkStart + limit)
        if (end < session.text.length && Character.isHighSurrogate(session.text[end - 1]) &&
            Character.isLowSurrogate(session.text[end])) --end
        session.chunkEnd = end
        session.utterance = "kiwix-speech-${session.token}-${++session.utteranceRevision}"
        val parameters = Bundle().apply {
            // Explicitly request embedded synthesis as well as selecting an offline-only Voice.
            putBoolean(TextToSpeech.Engine.KEY_FEATURE_NETWORK_SYNTHESIS, false)
            putBoolean(TextToSpeech.Engine.KEY_FEATURE_EMBEDDED_SYNTHESIS, true)
        }
        val result = engine?.speak(session.text.substring(session.chunkStart, end),
            TextToSpeech.QUEUE_FLUSH, parameters, session.utterance)
        if (result != TextToSpeech.SUCCESS) { finish(session, "SPEECH_QUEUE_FAILED"); return false }
        val utterance = session.utterance
        val timeout = Runnable {
            val active = current
            if (active != null && active.utterance == utterance) finish(active, "SPEECH_START_TIMEOUT")
        }
        clearStartTimeout()
        startTimeout = timeout
        main.postDelayed(timeout, 15_000)
        return true
    }

    private val listener = object : UtteranceProgressListener() {
        override fun onStart(utteranceId: String?) = callback(utteranceId) { session ->
            clearStartTimeout()
            session.progressAt = android.os.SystemClock.elapsedRealtime()
            session.stateAt = session.progressAt
            session.progressOffset = session.chunkStart
            session.offset = maxOf(session.offset, session.chunkStart)
            state(session, true)
        }
        override fun onDone(utteranceId: String?) = callback(utteranceId) { session ->
            clearStartTimeout()
            session.offset = session.chunkEnd
            if (session.offset == session.text.length) finish(session) else {
                try { queueChunk(session) } catch (_: Exception) { finish(session, "SPEECH_QUEUE_FAILED") }
            }
        }
        @Deprecated("Required by UtteranceProgressListener")
        override fun onError(utteranceId: String?) = callback(utteranceId) { finish(it, "SPEECH_ENGINE_ERROR") }
        override fun onError(utteranceId: String?, errorCode: Int) = callback(utteranceId) {
            finish(it, "SPEECH_ENGINE_ERROR_$errorCode")
        }
        override fun onStop(utteranceId: String?, interrupted: Boolean) = callback(utteranceId) {
            finish(it, "SPEECH_INTERRUPTED")
        }
        override fun onRangeStart(utteranceId: String?, start: Int, end: Int, frame: Int) = callback(utteranceId) { session ->
            if (start >= 0 && start <= session.chunkEnd - session.chunkStart) {
                val now = android.os.SystemClock.elapsedRealtime()
                val position = session.chunkStart + start
                val elapsed = now - session.progressAt
                if (session.progressAt > 0 && elapsed >= 1000 && position > session.progressOffset) {
                    session.charactersPerSecond = ((position - session.progressOffset) * 1000.0 / elapsed)
                        .coerceIn(1.0, 200.0)
                    session.progressAt = now
                    session.progressOffset = position
                }
                session.offset = maxOf(session.offset, session.chunkStart + start)
                // Word boundaries arrive several times per second. Coalesce the
                // presentation update; onStart/onDone/finish always emit the
                // final offset.
                if (now - session.stateAt >= STATE_INTERVAL_MS) {
                    session.stateAt = now
                    state(session, true)
                }
            }
        }
    }

    private fun callback(id: String?, action: (Session) -> Unit) {
        main.post {
            val session = current ?: return@post
            if (session.utterance != id || session.paused) return@post
            action(session)
        }
    }

    private fun finish(session: Session, error: String? = null) {
        if (current !== session) return
        current = null
        clearStartTimeout()
        session.registration.status = if (error == null) Status.FINISHED else Status.CANCELLED
        session.registration.offset = session.offset
        session.invoke?.let { reject(it, error ?: "SPEECH_CANCELLED") }
        session.invoke = null
        if (error != null) haltEngine()
        state(session, false, error)
        SpeechPlayback.stop(context)
    }

    private fun haltEngine(): String? {
        val tts = engine ?: return null
        // An initializing engine has not received any text to synthesize.
        if (!ready) return null
        val stopped = try { tts.stop() == TextToSpeech.SUCCESS } catch (_: Exception) { false }
        if (stopped) return null
        // A failed stop must not leave a reusable engine holding playback ownership.
        engine = null
        ready = false
        ++engineGeneration
        shutdownError = "SPEECH_STOP_FAILED"
        try { tts.shutdown() } catch (error: Exception) { shutdownError = "$shutdownError; $error" }
        return "SPEECH_STOP_FAILED"
    }

    private fun clearStartTimeout() {
        startTimeout?.let { main.removeCallbacks(it) }
        startTimeout = null
    }

    private fun state(session: Session, speaking: Boolean, error: String? = null, paused: Boolean = false) {
        val payload = JSObject().put("id", session.id).put("speaking", speaking)
            .put("offset", session.offset).put("paused", paused)
        if (error != null) payload.put("error", error)
        lastState = payload
        emit(payload)
        if (current === session) {
            SpeechPlayback.update(speaking, paused, session.offset > 0)
        }
    }

    private fun ownedSession(invoke: Invoke): Session? {
        val args = invoke.getArgs()
        val id = checkedId(args.getString("id"))
        val capability = checkedId(args.getString("token"))
        return current?.takeIf { it.id == id && it.registration.capability == capability }
    }


    private fun unavailable(invoke: Invoke, error: String) {
        invoke.resolve(JSObject().put("available", false).put("voices", JSArray()).put("error", error))
    }
    private fun reject(invoke: Invoke, code: String) { invoke.reject(code, code) }
    private fun checkedId(id: String): String {
        require(id.length == 36 && UUID.fromString(id).toString().equals(id, true))
        return id
    }
    private fun checkedLanguage(language: String): Locale {
        require(language.length in 1..128 && !language.contains('_'))
        val locale = try { Locale.Builder().setLanguageTag(language).build() }
            catch (_: IllformedLocaleException) { throw IllegalArgumentException() }
        require(locale.language.isNotEmpty() && locale.language != "und")
        return locale
    }

    private companion object {
        /** Coalescing window for word-boundary progress events. */
        const val STATE_INTERVAL_MS = 300L
    }
}
