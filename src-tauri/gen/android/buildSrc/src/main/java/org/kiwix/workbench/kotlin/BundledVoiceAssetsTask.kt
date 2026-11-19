import java.io.BufferedOutputStream
import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.nio.file.StandardOpenOption.READ
import java.security.MessageDigest
import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.work.DisableCachingByDefault

@DisableCachingByDefault(because = "Verify the supplied offline distribution on every package build")
abstract class BundledVoiceAssetsTask : DefaultTask() {
    // The selected directory can contain unrelated files. Gradle must not
    // recursively fingerprint them or follow links into another directory.
    @get:Internal abstract val model: DirectoryProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val manifest: RegularFileProperty
    @get:OutputDirectory abstract val output: DirectoryProperty

    init { outputs.upToDateWhen { false } }

    @TaskAction fun stage() {
        stageBundledVoice(model.get().asFile.toPath(), manifest.get().asFile, output.get().asFile.toPath())
    }
}

internal fun stageBundledVoice(model: Path, manifest: File, output: Path) {
    require(Files.isDirectory(model, NOFOLLOW_LINKS)) { "Missing offline Kokoro model directory" }
    Files.createDirectories(output)
    val temporary = Files.createTempFile(output, ".kokoro-", ".tmp")
    try {
        BufferedOutputStream(Files.newOutputStream(temporary)).use { stream ->
            val buffer = ByteArray(128 * 1024)
            val directories = mutableSetOf<Path>()
            val entries = mutableSetOf<Path>()
            manifest.forEachLine { line ->
                val fields = line.split('\t')
                require(fields.size == 3)
                val (sizeText, digest, name) = fields
                val size = sizeText.toLong()
                val path = model.resolve(name).normalize()
                require(size >= 0 && path.startsWith(model) && entries.add(path))
                require(path == model || path.parent in directories)
                if (digest == "-") {
                    require(size == 0L && Files.isDirectory(path, NOFOLLOW_LINKS)) { "Missing model directory: $name" }
                    directories.add(path)
                } else {
                    require(Files.isRegularFile(path, NOFOLLOW_LINKS)) { "Missing model file: $name" }
                    require(Files.size(path) == size) { "Size mismatch for model file: $name" }
                    val hash = MessageDigest.getInstance("SHA-256")
                    Files.newInputStream(path, READ, NOFOLLOW_LINKS).use { source ->
                        var remaining = size
                        while (remaining > 0) {
                            val count = source.read(buffer, 0, minOf(remaining, buffer.size.toLong()).toInt())
                            require(count > 0) { "Incomplete model file: $name" }
                            hash.update(buffer, 0, count)
                            stream.write(buffer, 0, count)
                            remaining -= count
                        }
                        require(source.read() == -1) { "Oversized model file: $name" }
                    }
                    require(hash.digest().joinToString("") { "%02x".format(it) } == digest) {
                        "Checksum mismatch for model file: $name"
                    }
                }
            }
        }
        Files.move(temporary, output.resolve("kiwix-kokoro-v1.voicepack"), ATOMIC_MOVE, REPLACE_EXISTING)
    } finally {
        Files.deleteIfExists(temporary)
    }
}
