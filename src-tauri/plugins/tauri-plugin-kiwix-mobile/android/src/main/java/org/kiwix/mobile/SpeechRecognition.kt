package org.kiwix.mobile

import android.content.Context
import android.content.Intent
import android.speech.RecognizerIntent

/**
 * Delegates dictation to an installed recognizer activity. Kiwix never records
 * audio itself: there is no RECORD_AUDIO request, no SpeechRecognizer instance
 * and no retained microphone authority. A recognizer activity owns the audio
 * session; the app only receives the final transcript it returns.
 */
internal object SpeechRecognition {
    /** Package visibility does not change while the process lives, but a
     * recognizer can be installed or disabled; re-query after ten seconds. */
    @Volatile private var availability: Boolean? = null
    @Volatile private var checkedAt = 0L

    /** Package-visibility queries in the plugin manifest make this check truthful on Android 11+. */
    fun available(context: Context): Boolean {
        val now = System.currentTimeMillis()
        availability?.let { if (now - checkedAt < 10_000L) return it }
        val value = context.packageManager.queryIntentActivities(intent(context, null), 0).isNotEmpty()
        availability = value
        checkedAt = now
        return value
    }

    fun intent(context: Context, language: String?): Intent {
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1)
            .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, false)
        if (!language.isNullOrBlank()) intent.putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
        return intent
    }
}
