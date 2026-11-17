import java.io.File
import java.io.InputStream
import java.util.zip.ZipFile
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.TaskAction

/**
 * Android 15+ devices can run with 16 KiB memory pages. Native libraries must
 * then align every PT_LOAD segment to 16 KiB, and the APK must zip-align its
 * uncompressed .so entries to 16 KiB.
 *
 * NDK r28 and the Rust linker already emit that alignment because the mobile
 * SDK manifest's `-Wl,-z,max-page-size=16384` reaches the final link. This task
 * fails the build if a toolchain, SDK or packaging change ever regresses it.
 */
abstract class Verify16KbPageAlignmentTask : DefaultTask() {
    @get:InputFiles
    abstract val apkDirectory: DirectoryProperty

    @get:Internal
    abstract val sdkDirectory: DirectoryProperty

    @TaskAction
    fun verify() {
        val apks = apkDirectory.get().asFile.walkTopDown()
            .filter { it.isFile && it.extension == "apk" }
            .sortedBy { it.name }
            .toList()
        if (apks.isEmpty()) {
            throw GradleException("No APK produced; cannot verify 16 KB page compatibility")
        }
        val zipalign = zipalignExecutable(sdkDirectory.get().asFile)
        apks.forEach { verifyApkPageSize(it, zipalign) }
        logger.lifecycle("16 KB page compatibility verified for ${apks.joinToString { it.name }}")
    }

    private fun verifyApkPageSize(apk: File, zipalign: File) {
        ZipFile(apk).use { archive ->
            for (entry in archive.entries()) {
                if (entry.isDirectory || !entry.name.endsWith(".so")) continue
                val (elfClass, aligns) = archive.getInputStream(entry).use { loadAlignments(entry.name, it) }
                val required = if (elfClass == 2) SIXTEEN_KIB else FOUR_KIB
                for (align in aligns) {
                    if (align < required || align % required != 0L) {
                        throw GradleException(
                            "${apk.name}:${entry.name} has a PT_LOAD segment aligned to 0x${align.toString(16)}; " +
                                "Android requires 0x${required.toString(16)} alignment for " +
                                (if (elfClass == 2) "64-bit" else "32-bit") + " libraries"
                        )
                    }
                }
            }
        }
        val process = ProcessBuilder(zipalign.absolutePath, "-c", "-P", "16", "-v", "4", apk.absolutePath)
            .redirectErrorStream(true)
            .start()
        val output = process.inputStream.bufferedReader().use { it.readText() }
        val exit = process.waitFor()
        if (exit != 0) {
            val details = output.lineSequence()
                .filter { "BAD" in it || "FAILED" in it || "Verification" in it }
                .joinToString("\n")
            throw GradleException("${apk.name} is not 16 KB zip-aligned (zipalign exit $exit)\n$details")
        }
    }

    /** Returns the ELF class (1 = 32-bit, 2 = 64-bit) and every PT_LOAD p_align. */
    private fun loadAlignments(entryName: String, input: InputStream): Pair<Int, List<Long>> {
        val ident = readExactly(input, 16, entryName)
        if (ident[0] != 0x7f.toByte() || ident[1] != 'E'.code.toByte() ||
            ident[2] != 'L'.code.toByte() || ident[3] != 'F'.code.toByte()
        ) {
            throw GradleException("$entryName: not an ELF file")
        }
        val elfClass = ident[4].toInt()
        if (elfClass != 1 && elfClass != 2) throw GradleException("$entryName: unknown ELF class $elfClass")
        if (ident[5].toInt() != 1) throw GradleException("$entryName: big-endian ELF is not an Android target")
        val headerSize = if (elfClass == 2) 64 else 52
        val header = ident + readExactly(input, headerSize - ident.size, entryName)
        val programOffset = if (elfClass == 2) u64(header, 0x20) else u32(header, 0x1c)
        val entrySize = if (elfClass == 2) u16(header, 0x36) else u16(header, 0x2a)
        val entryCount = if (elfClass == 2) u16(header, 0x38) else u16(header, 0x2c)
        val minimumEntrySize = if (elfClass == 2) 56 else 32
        if (entryCount == 0 || entrySize < minimumEntrySize) {
            throw GradleException("$entryName: invalid ELF program header table")
        }
        if (programOffset > headerSize) skipExactly(input, programOffset - headerSize, entryName)
        val table = readExactly(input, entryCount * entrySize, entryName)
        val aligns = (0 until entryCount).mapNotNull { index ->
            val base = index * entrySize
            if (u32(table, base) != 1L) return@mapNotNull null
            if (elfClass == 2) u64(table, base + 0x30) else u32(table, base + 0x1c)
        }
        if (aligns.isEmpty()) throw GradleException("$entryName: no PT_LOAD segments")
        return elfClass to aligns
    }

    private fun zipalignExecutable(sdkDirectory: File): File {
        val buildTools = sdkDirectory.resolve("build-tools")
        val latest = buildTools.listFiles()?.filter { it.isDirectory }?.maxWithOrNull(
            Comparator { left, right ->
                val a = versionParts(left.name)
                val b = versionParts(right.name)
                (0 until maxOf(a.size, b.size)).firstNotNullOfOrNull { index ->
                    a.getOrElse(index) { 0 }.compareTo(b.getOrElse(index) { 0 }).takeIf { it != 0 }
                } ?: 0
            }
        ) ?: throw GradleException("No Android build-tools found under $buildTools")
        val name = if (System.getProperty("os.name").startsWith("Windows")) "zipalign.exe" else "zipalign"
        return latest.resolve(name).takeIf { it.isFile }
            ?: throw GradleException("Missing $name in $latest; Android build-tools are required")
    }

    private fun versionParts(name: String): List<Int> =
        name.split('.').map { part -> part.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }

    private fun readExactly(input: InputStream, count: Int, name: String): ByteArray {
        val bytes = ByteArray(count)
        var offset = 0
        while (offset < count) {
            val read = input.read(bytes, offset, count - offset)
            if (read < 0) throw GradleException("$name: truncated ELF data")
            offset += read
        }
        return bytes
    }

    private fun skipExactly(input: InputStream, count: Long, name: String) {
        var remaining = count
        while (remaining > 0) {
            val skipped = input.skip(remaining)
            if (skipped <= 0) {
                if (input.read() < 0) throw GradleException("$name: truncated ELF program headers")
                remaining -= 1
            } else {
                remaining -= skipped
            }
        }
    }

    private fun u16(bytes: ByteArray, offset: Int): Int =
        (bytes[offset].toInt() and 0xff) or ((bytes[offset + 1].toInt() and 0xff) shl 8)

    private fun u32(bytes: ByteArray, offset: Int): Long =
        u16(bytes, offset).toLong() or (u16(bytes, offset + 2).toLong() shl 16)

    private fun u64(bytes: ByteArray, offset: Int): Long =
        u32(bytes, offset) or (u32(bytes, offset + 4) shl 32)

    private companion object {
        const val SIXTEEN_KIB = 0x4000L
        const val FOUR_KIB = 0x1000L
    }
}
