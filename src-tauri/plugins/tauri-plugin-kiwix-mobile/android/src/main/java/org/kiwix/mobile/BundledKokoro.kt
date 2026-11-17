package org.kiwix.mobile

import android.content.Context
import android.content.res.AssetManager
import android.os.StatFs
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Installs the optional Kokoro pack that an offline build embeds under
 * `assets/kokoro-en-fp16-v1_0`. Standard builds have no such asset and this is
 * a no-op. The APK asset tree is copied once into the same app-private model
 * directory used by the download path. Completion is published only after the
 * full required set and the pinned model hash pass.
 */
internal object BundledKokoro {
    private const val MODEL_DIR = "kokoro-en-fp16-v1_0"
    private const val MARKER = "bundled-model"
    private const val INSTALLING = "bundled-model-installing"
    private const val REMOVED = "bundled-model-removed"
    private const val MODEL_SHA256 = "7983eb4baa16c3b9a81832afd570e5bec06da12538f89b58d32c5c9ed11a00d9"
    private val required = listOf(
        "model.fp16.onnx", "voices.bin", "tokens.txt",
        "lexicon-us-en.txt", "lexicon-gb-en.txt",
    )
    private val installing = AtomicBoolean(false)
    private val executor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "kiwix-bundled-kokoro").apply { isDaemon = true }
    }

    fun install(context: Context) {
        val assets = context.assets
        if (assets.list(MODEL_DIR).isNullOrEmpty()) return
        val models = File(context.filesDir, "kiwix/models").apply { mkdirs() }
        if (File(models, REMOVED).exists()) return
        val target = File(models, MODEL_DIR)
        val marker = File(models, MARKER)
        if (marker.isFile && complete(target)) return
        if (complete(target)) {
            if (sha256(File(target, "model.fp16.onnx")) == MODEL_SHA256) {
                marker.createNewFile()
                File(models, INSTALLING).delete()
                return
            }
            target.deleteRecursively()
            marker.delete()
        }
        if (!installing.compareAndSet(false, true)) return
        File(models, INSTALLING).createNewFile()
        executor.execute {
            val temporary = File(models, ".bundled-${UUID.randomUUID()}")
            try {
                models.listFiles { file -> file.name.startsWith(".bundled-") }?.forEach { it.deleteRecursively() }
                val needed = 240L * 1024 * 1024
                if (StatFs(models.absolutePath).availableBytes < needed) throw IllegalStateException("not enough free space")
                temporary.mkdirs()
                copy(assets, MODEL_DIR, temporary)
                require(complete(temporary)) { "bundled model is incomplete" }
                require(sha256(File(temporary, "model.fp16.onnx")) == MODEL_SHA256) { "bundled model hash mismatch" }
                if (target.exists()) target.deleteRecursively()
                require(temporary.renameTo(target)) { "bundled model publish failed" }
                marker.createNewFile()
            } catch (_: Exception) {
                temporary.deleteRecursively()
                target.deleteRecursively()
                marker.delete()
            } finally {
                File(models, INSTALLING).delete()
                installing.set(false)
            }
        }
    }

    private fun complete(dir: File): Boolean =
        required.all { File(dir, it).isFile } && File(dir, "espeak-ng-data").isDirectory

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(256 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it.toInt() and 0xff) }
    }

    private fun copy(assets: AssetManager, path: String, destination: File) {
        val children = assets.list(path).orEmpty()
        if (children.isEmpty()) {
            destination.parentFile?.mkdirs()
            assets.open(path).use { input ->
                FileOutputStream(destination).use { output -> input.copyTo(output, 128 * 1024) }
            }
            return
        }
        destination.mkdirs()
        for (child in children) copy(assets, "$path/$child", File(destination, child))
    }
}
