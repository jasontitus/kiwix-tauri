import java.nio.file.Files
import java.security.MessageDigest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class BundledVoiceAssetsTest {
    @get:Rule val temp = TemporaryFolder()

    @Test fun stagesOnlyPinnedBytesInManifestOrderAndPreservesTheLastGoodOutput() {
        val model = temp.newFolder("model").toPath()
        val output = temp.newFolder("generated").toPath()
        val manifest = temp.newFile("layout.tsv")
        val nested = Files.createDirectory(model.resolve("voices"))
        val a = byteArrayOf(0, 1, 2)
        val b = byteArrayOf(44, 55)
        val digest = { v: ByteArray -> MessageDigest.getInstance("SHA-256").digest(v).joinToString("") { "%02x".format(it) } }
        manifest.writeText("0\t-\t.\n0\t-\tvoices\n2\t${digest(b)}\tvoices/b\n3\t${digest(a)}\ta\n")
        Files.write(nested.resolve("b"), b)
        Files.write(model.resolve("a"), a)
        Files.writeString(model.resolve("unrelated"), "preserve this file")
        stageBundledVoice(model, manifest, output)
        val asset = output.resolve("kiwix-kokoro-v1.voicepack")
        assertArrayEquals(b + a, Files.readAllBytes(asset))
        assertFalse(Files.exists(output.resolve("unrelated")))
        Files.write(model.resolve("a"), byteArrayOf(3, 2, 1))
        assertThrows(IllegalArgumentException::class.java) { stageBundledVoice(model, manifest, output) }
        assertArrayEquals(b + a, Files.readAllBytes(asset))
        assertArrayEquals("preserve this file".toByteArray(), Files.readAllBytes(model.resolve("unrelated")))
    }
}
