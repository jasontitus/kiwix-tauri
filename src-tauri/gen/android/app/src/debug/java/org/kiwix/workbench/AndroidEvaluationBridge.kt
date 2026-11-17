package org.kiwix.workbench

import android.content.Context

/** Instrumentation-only bridge: never registered with Tauri or JavaScript. */
object AndroidEvaluationBridge {
    init { System.loadLibrary("kiwix_app") }

    @JvmStatic external fun evaluate(context: Context, runId: String, caseId: String, mode: String): String
    @JvmStatic external fun phase(): Int
}
