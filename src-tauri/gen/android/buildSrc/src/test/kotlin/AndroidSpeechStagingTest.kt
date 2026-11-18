import java.nio.file.Files
import org.gradle.api.GradleException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class AndroidSpeechStagingTest {
    @get:Rule
    val temporary = TemporaryFolder()

    @Test
    fun restoresMissingAndStaleProvidersFromTheLinkedProfile() {
        val profile = temporary.newFolder("external-target")
        val destination = temporary.newFolder("arm64-v8a")
        val app = profile.resolve("libkiwix_app.so").apply { writeText("app") }
        Files.createSymbolicLink(destination.resolve(app.name).toPath(), app.toPath())

        val names = listOf("libonnxruntime.so", "libsherpa-onnx-c-api.so")
        for (name in names) profile.resolve(name).writeText(name)
        destination.resolve(names.first()).writeText("stale")
        val marker = destination.resolve(".sherpa-onnx-version").apply { writeText("already staged") }
        val other = destination.resolve("unrelated.so").apply { writeText("keep") }

        stageAndroidSpeechLibraries(destination)

        for (name in names) assertEquals(name, destination.resolve(name).readText())
        assertEquals("already staged", marker.readText())
        assertEquals("keep", other.readText())
    }

    @Test
    fun missingRequiredProducerPreservesExistingOutputs() {
        val profile = temporary.newFolder("external-target")
        val destination = temporary.newFolder("arm64-v8a")
        val app = profile.resolve("libkiwix_app.so").apply { writeText("app") }
        Files.createSymbolicLink(destination.resolve(app.name).toPath(), app.toPath())
        profile.resolve("libonnxruntime.so").writeText("new onnx")
        val prior = destination.resolve("libonnxruntime.so").apply { writeText("prior onnx") }

        assertThrows(GradleException::class.java) { stageAndroidSpeechLibraries(destination) }

        assertEquals("prior onnx", prior.readText())
        assertFalse(destination.resolve("libsherpa-onnx-c-api.so").exists())
    }
}
