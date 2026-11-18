import java.io.File
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction
import javax.inject.Inject
import org.gradle.process.ExecOperations

abstract class BuildTask : DefaultTask() {
    @get:Inject
    abstract val execOperations: ExecOperations

    @Input
    var rootDirRel: String? = null
    @Input
    var projectDir: String? = null
    @Input
    var target: String? = null
    @Input
    lateinit var abi: String
    @Input
    var release: Boolean? = null

    @TaskAction
    fun assemble() {
        buildWithTauri()
        stageAndroidSpeechLibraries(File(projectDir, "src/main/jniLibs/$abi"))
    }

    private fun buildWithTauri() {
        val executable = """npm""";
        try {
            runTauriCli(executable)
        } catch (e: Exception) {
            if (Os.isFamily(Os.FAMILY_WINDOWS)) {
                // Try different Windows-specific extensions
                val fallbacks = listOf(
                    "$executable.exe",
                    "$executable.cmd",
                    "$executable.bat",
                )

                var lastException: Exception = e
                for (fallback in fallbacks) {
                    try {
                        runTauriCli(fallback)
                        return
                    } catch (fallbackException: Exception) {
                        lastException = fallbackException
                    }
                }
                throw lastException
            } else {
                throw e;
            }
        }
    }

    fun runTauriCli(executable: String) {
        val rootDirRel = rootDirRel ?: throw GradleException("rootDirRel cannot be null")
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")
        val args = listOf("run", "--", "tauri", "android", "android-studio-script");

        execOperations.exec {
            workingDir(File(projectDir, rootDirRel))
            executable(executable)
            args(args)
            if (logger.isEnabled(LogLevel.DEBUG)) {
                args("-vv")
            } else if (logger.isEnabled(LogLevel.INFO)) {
                args("-v")
            }
            if (release) {
                args("--release")
            }
            args(listOf("--target", target))
        }.assertNormalExitValue()
    }
}

/** Tauri links the selected ABI's app ELF to its exact Cargo profile on all hosts.
 * sherpa-onnx-sys stages its runtime beside that ELF, including in external targets.
 */
internal fun stageAndroidSpeechLibraries(jniDirectory: File) {
    val destination = jniDirectory.toPath()
    if (Files.isSymbolicLink(destination) || Files.isSymbolicLink(destination.parent)) {
        throw GradleException("Expected a generated JNI directory at $destination, found a linked directory.")
    }
    val app = destination.resolve("libkiwix_app.so")
    if (!Files.isSymbolicLink(app) || !Files.isRegularFile(app)) {
        throw GradleException("Missing Tauri build output at $app; rebuild the selected Android target.")
    }
    val profile = app.toRealPath().parent
    val required = listOf("libonnxruntime.so", "libsherpa-onnx-c-api.so")
    val optional = listOf("libsherpa-onnx-cxx-api.so", "libsherpa-onnx-jni.so")
    val libraries = required + optional.filter { name ->
        Files.exists(profile.resolve(name), NOFOLLOW_LINKS) || Files.exists(destination.resolve(name), NOFOLLOW_LINKS)
    }
    for (name in libraries) {
        val source = profile.resolve(name)
        if (!Files.isRegularFile(source) || Files.size(source) == 0L) {
            throw GradleException("Missing or empty $source. Rebuild the pinned sherpa-onnx-sys dependency for this ABI, or use a fresh generated JNI folder.")
        }
    }
    for (name in libraries) {
        val source = profile.resolve(name)
        val output = destination.resolve(name)
        if (Files.isRegularFile(output, NOFOLLOW_LINKS) && Files.mismatch(source, output) == -1L) continue
        val temporary = Files.createTempFile(destination, ".$name-", ".tmp")
        try {
            Files.copy(source, temporary, REPLACE_EXISTING)
            Files.move(temporary, output, ATOMIC_MOVE, REPLACE_EXISTING)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }
}
