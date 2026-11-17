package org.kiwix.mobile

/** UTF-16 positions, matching WebView extraction and Android TTS callbacks. */
internal object SpeechPosition {
    fun boundary(text: String, requested: Int): Int {
        var offset = requested.coerceIn(0, text.length)
        if (offset > 0 && offset < text.length && Character.isHighSurrogate(text[offset - 1]) &&
            Character.isLowSurrogate(text[offset])) --offset
        return offset
    }

    fun previous(text: String, offset: Int): Int {
        val end = offset.coerceIn(0, text.length)
        if (end == 0) return 0
        val current = text.lastIndexOf('\n', end - 1)
        return if (current <= 0) 0 else text.lastIndexOf('\n', current - 1) + 1
    }

    fun next(text: String, offset: Int): Int {
        val newline = text.indexOf('\n', offset.coerceIn(0, text.length))
        return if (newline < 0) text.length else newline + 1
    }

    /** TTS has no PCM timeline; use observed UTF-16 progress per speaking second.
     * Until range callbacks establish a rate, use the conventional 180 wpm /
     * five-character word estimate, scaled by the selected speech rate. */
    fun timed(text: String, offset: Int, seconds: Double, charactersPerSecond: Double): Int {
        require(seconds.isFinite() && kotlin.math.abs(seconds) <= 300)
        require(charactersPerSecond.isFinite() && charactersPerSecond > 0)
        val target = (offset.toDouble() + seconds * charactersPerSecond)
            .coerceIn(0.0, text.length.toDouble()).toInt()
        return boundary(text, target)
    }
}
