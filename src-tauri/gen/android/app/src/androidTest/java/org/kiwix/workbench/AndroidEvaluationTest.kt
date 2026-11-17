package org.kiwix.workbench

import android.app.ActivityManager
import android.content.Context
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Bundle
import android.os.Debug
import android.os.ParcelFileDescriptor
import android.os.Process
import android.os.StatFs
import android.os.SystemClock
import android.system.Os
import android.system.OsConstants
import android.util.Base64
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.FileDescriptor
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.InputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidEvaluationTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val target = instrumentation.targetContext
    private val assets = instrumentation.context.assets
    private val arguments = InstrumentationRegistry.getArguments()
    private val samplePeriodMs = 100L
    private val sha256Pattern = Regex("[0-9a-fA-F]{64}")
    private var externalFixtures: File? = null
    private var shellFixtures = false
    private val runId = "run-${UUID.randomUUID()}"
    private val privateReports = File(target.filesDir, "android-evaluation-receipts/$runId")
    private var consumeStagedInputs = false
    private val candidateUses = mutableMapOf<String, Int>()
    private val protectedInputs = mutableSetOf("manifest.json")
    private lateinit var root: File
    private lateinit var reports: File
    private lateinit var bridge: Class<*>

    private fun relative(name: String): String {
        require(name.isNotEmpty() && name.length <= 240 && !name.contains('\\') && !name.startsWith('/') && name.split('/').all { it.isNotEmpty() && it != "." && it != ".." })
        return name
    }

    private data class Consumable(
        val device: Long, val inode: Long, val size: Long, val creditedBytes: Long,
        var unlinked: Boolean = false, var descriptorClosed: Boolean = false
    )

    private fun externalFixture(name: String): File {
        val inputRoot = requireNotNull(externalFixtures)
        require(inputRoot.canonicalFile == inputRoot.absoluteFile && inputRoot.isDirectory) { "External fixture root must be a non-symlink directory" }
        var source = inputRoot
        for (component in relative(name).split('/')) {
            source = File(source, component)
            require(!OsConstants.S_ISLNK(Os.lstat(source.path).st_mode) && source.canonicalFile == source.absoluteFile) { "External fixture symlink or path escape rejected" }
        }
        return source
    }

    private fun requireIdentity(stat: android.system.StructStat, expected: Consumable) {
        require(OsConstants.S_ISREG(stat.st_mode) && stat.st_dev == expected.device && stat.st_ino == expected.inode &&
            stat.st_size == expected.size && stat.st_nlink == 1L) { "Disposable fixture identity, size or link count changed; input not consumed" }
    }

    // Test Lab officially supports --other-files under /data/local/tmp. The
    // shell bridge is test-APK-only: it streams those delivery files into
    // bounded private copies, never into the measured production workload.
    // Neither the app UID nor its production permissions are changed.
    private fun <T> withDeliveryCommand(operation: String, name: String = "", maximum: Long = 0, read: (DataInputStream) -> T): T {
        require(Build.VERSION.SDK_INT >= 24) { "Shell fixture delivery requires Android API 24+" }
        val testInfo = instrumentation.context.applicationInfo
        val targetInfo = target.applicationInfo
        // AGP excludes dependencies already supplied by the target from the
        // test APK (notably Kotlin stdlib). app_process does not inherit the
        // instrumentation class loader, so include both installed APK sets.
        // Merely using their dex does not load the production JNI library.
        val apks = (listOf(testInfo.sourceDir) + testInfo.splitSourceDirs.orEmpty().toList() +
            listOf(targetInfo.sourceDir) + targetInfo.splitSourceDirs.orEmpty().toList()).distinct()
        // UiAutomation executes an argv-tokenized command, not a shell script.
        // All tokens are fixed or encoded; installed paths cannot add argv.
        require(apks.all { it.matches(Regex("[A-Za-z0-9_./+=~\\-]+")) })
        val classpath = apks.joinToString(":")
        val encoded = Base64.encodeToString(name.toByteArray(Charsets.UTF_8), Base64.NO_WRAP or Base64.URL_SAFE)
        val command = "/system/bin/env CLASSPATH=$classpath /system/bin/app_process /system/bin org.kiwix.workbench.AndroidEvaluationFixtureDelivery $operation ${if (encoded.isEmpty()) "-" else encoded} $maximum"
        val stderrFile = if (Build.VERSION.SDK_INT in 31..33)
            "/data/local/tmp/kiwix-evaluation-reports/$runId-${UUID.randomUUID()}/stderr.txt" else null
        val stderrCapture = when {
            Build.VERSION.SDK_INT >= 34 -> "bounded separate stderr descriptor; remainder drained"
            Build.VERSION.SDK_INT >= 31 -> "bounded shell-owned stderr receipt; remainder drained"
            else -> "unavailable: API24-30 executeShellCommand exposes stdout only; no stderr or exit-status proof"
        }
        save("delivery-shell-command", JSONObject().put("operation", operation).put("file", name)
            .put("sdk", Build.VERSION.SDK_INT).put("stderrCapture", stderrCapture)
            .put("stderrReceiptPath", stderrFile ?: JSONObject.NULL)
            .put("stderrMaximumBytes", if (Build.VERSION.SDK_INT >= 31) 16384 else 0))
        val descriptors = if (Build.VERSION.SDK_INT >= 34) {
            instrumentation.uiAutomation.executeShellCommandRwe(command)
        } else if (Build.VERSION.SDK_INT >= 31) {
            // API 31–33 has no Rwe. Feed a fixed script through stdin instead
            // of relying on Runtime.exec's unsupported shell quoting. fd 3
            // carries ONLY binary stdout; the pipe carries ONLY stderr. Keep
            // at most four 4096-byte reads, then drain/discard the remainder
            // so verbose stderr cannot break the payload with SIGPIPE.
            val opened = instrumentation.uiAutomation.executeShellCommandRw("/system/bin/sh")
            val directory = requireNotNull(stderrFile).substringBeforeLast('/')
            val script = """
                umask 077
                base=/data/local/tmp/kiwix-evaluation-reports
                if [ -L "${'$'}base" ] || ! mkdir -p "${'$'}base" || [ ! -d "${'$'}base" ]; then
                    printf 'evaluation stderr receipt root unavailable\n'
                    exit 1
                fi
                if ! mkdir '$directory'; then
                    printf 'evaluation stderr receipt directory unavailable\n'
                    exit 1
                fi
                exec 3>&1
                ( $command 2>&1 1>&3 ) | ( /system/bin/toybox dd bs=4096 count=4 of='$stderrFile' 2>/dev/null; /system/bin/toybox cat >/dev/null )
            """.trimIndent() + "\n"
            try {
                ParcelFileDescriptor.AutoCloseOutputStream(opened[1]).use { it.write(script.toByteArray(Charsets.UTF_8)) }
            } catch (error: Throwable) {
                opened.forEach { it.close() }
                throw error
            }
            opened
        } else {
            // API24–30 supplies stdout only. The helper takes fixed argv and
            // needs no stdin; use the real supported API, without shell quoting,
            // privilege changes or a fabricated stderr/exit-status observation.
            arrayOf(instrumentation.uiAutomation.executeShellCommand(command))
        }
        val stderr = StringBuilder()
        if (Build.VERSION.SDK_INT >= 34) descriptors[1].close()
        val errorReader = if (Build.VERSION.SDK_INT >= 34) Thread({
            try {
                ParcelFileDescriptor.AutoCloseInputStream(descriptors[2]).use { input ->
                    val buffer = ByteArray(4096)
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        synchronized(stderr) {
                            val retained = minOf(count, 16384 - stderr.length)
                            if (retained > 0) stderr.append(String(buffer, 0, retained, Charsets.UTF_8))
                        }
                    }
                }
            } catch (error: Exception) {
                synchronized(stderr) { if (stderr.length < 16384) stderr.append(error.toString()) }
            }
        }, "evaluation-delivery-stderr") else null
        errorReader?.start()
        try {
            return DataInputStream(ParcelFileDescriptor.AutoCloseInputStream(descriptors[0])).use { input ->
                require(input.readInt() == AndroidEvaluationFixtureDelivery.MAGIC) { "Invalid shell delivery protocol" }
                read(input)
            }
        } catch (error: Throwable) {
            // Closing stdout unblocks a writer when the bounded consumer fails.
            descriptors[0].close()
            errorReader?.join(5000)
            val detail = when {
                Build.VERSION.SDK_INT < 31 -> "Unavailable: stdout-only API24-30 transport"
                stderrFile != null -> readShellStderr(stderrFile)
                else -> synchronized(stderr) { stderr.toString() }
            }
            save("delivery-failed", JSONObject().put("status", "failed").put("operation", operation).put("file", name)
                .put("reason", error.toString()).put("shellStderr", detail)
                .put("stderrReceiptPath", stderrFile ?: JSONObject.NULL).put("stderrCapture", stderrCapture))
            throw error
        } finally {
            descriptors[0].close()
            if (Build.VERSION.SDK_INT >= 34) descriptors[2].close()
            errorReader?.join(5000)
        }
    }

    private fun readShellStderr(path: String): String {
        // Only internally generated fixed-root paths reach here, never runner
        // arguments. This remains usable even if the Java helper cannot boot.
        require(path.startsWith("/data/local/tmp/kiwix-evaluation-reports/$runId-") &&
            path.matches(Regex("[A-Za-z0-9_./\\-]+")))
        return try {
            val descriptor = instrumentation.uiAutomation.executeShellCommand("/system/bin/toybox head -c 16384 $path")
            ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { input ->
                val bytes = ByteArray(16384)
                var count = 0
                while (count < bytes.size) {
                    val read = input.read(bytes, count, bytes.size - count)
                    if (read < 0) break
                    count += read
                }
                String(bytes, 0, count, Charsets.UTF_8)
            }
        } catch (error: Exception) { "Unable to retrieve shell stderr receipt: $error" }
    }

    private fun <T> withShellFixture(name: String, maximum: Long, read: (InputStream) -> T): T {
        val safeName = relative(name)
        save("delivery-current", JSONObject().put("status", "opening").put("file", safeName).put("maximumBytes", maximum))
        return withDeliveryCommand("read", safeName, maximum) { input ->
            val header = JSONObject(input.readUTF())
            require(header.optBoolean("ok")) { "Shell fixture open failed: $header" }
            require(header.getInt("uid") == 2000) { "Delivery must run as unprivileged Android shell, not root" }
            val size = header.getLong("size")
            require(size in 0L..maximum)
            val stat = header.getJSONObject("stat")
            require(OsConstants.S_ISREG(stat.getInt("mode")) && stat.getLong("size") == size)
            save("delivery-current", header.put("status", "copying").put("file", safeName))
            var remaining = size
            val bounded = object : InputStream() {
                override fun read(): Int {
                    if (remaining == 0L) return -1
                    val value = input.read()
                    check(value >= 0) { "Truncated shell fixture $safeName" }
                    remaining--
                    return value
                }
                override fun read(bytes: ByteArray, offset: Int, length: Int): Int {
                    if (length == 0) return 0
                    if (remaining == 0L) return -1
                    val count = input.read(bytes, offset, minOf(length.toLong(), remaining).toInt())
                    check(count > 0) { "Truncated shell fixture $safeName" }
                    remaining -= count
                    return count
                }
            }
            val result = read(bounded)
            require(remaining == 0L) { "Shell fixture was not fully consumed" }
            val receipt = JSONObject(input.readUTF())
            require(receipt.optBoolean("ok") && receipt.getString("identity") == header.getString("identity")) {
                "Shell fixture changed during delivery: $receipt"
            }
            require(input.read() == -1) { "Unexpected bytes after shell delivery receipt" }
            save("delivery-current", header.put("status", "copied").put("receipt", receipt))
            result
        }
    }

    private fun <T> withFixture(name: String, maximum: Long, consume: Consumable? = null, read: (InputStream) -> T): T {
        if (shellFixtures) {
            require(consume == null) { "Shell delivery inputs are read-only and never receive reclaim credit" }
            return withShellFixture(name, maximum, read)
        }
        if (externalFixtures == null) {
            require(consume == null)
            return assets.open("evaluation/${relative(name)}").use(read)
        }
        val source = externalFixture(name)
        val fd = Os.open(source.path, OsConstants.O_RDONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW, 0)
        var failure: Throwable? = null
        try {
            val stat = Os.fstat(fd)
            require(OsConstants.S_ISREG(stat.st_mode) && stat.st_size in 0L..maximum) { "External fixture must be a regular file within its declared size bound" }
            if (consume != null) requireIdentity(stat, consume)
            // Android's FileInputStream(FileDescriptor) is non-owning; close
            // the Os.open descriptor explicitly after closing the stream.
            val result = FileInputStream(fd).use(read)
            if (consume != null) {
                // Only exact, durably completed private copies reach here.
                // Re-resolve the fixed-root path, compare lstat to the still
                // owned no-follow fd, and never unlink shared/replaced inputs.
                require(consumeStagedInputs && candidateUses[name] == 1 && name.startsWith("candidates/") && name !in protectedInputs)
                requireIdentity(Os.fstat(fd), consume)
                requireIdentity(Os.lstat(externalFixture(name).path), consume)
                Os.remove(source.path)
                consume.unlinked = true
            }
            return result
        } catch (error: Throwable) {
            failure = error
            throw error
        } finally {
            try {
                Os.close(fd)
                if (consume != null) consume.descriptorClosed = true
            } catch (error: Throwable) {
                val original = failure
                if (original == null) throw error
                original.addSuppressed(error)
            }
        }
    }

    private fun copyFixture(name: String, maximum: Long = 32L * 1024 * 1024, exact: Boolean = false, consume: Consumable? = null) {
        val destination = File(root, "fixtures/${relative(name)}")
        destination.parentFile!!.mkdirs()
        require(consume == null || exact)
        withFixture(name, maximum, consume) { input ->
            destination.outputStream().use { output ->
                val buffer = ByteArray(128 * 1024)
                var copied = 0L
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    require(count.toLong() <= maximum - copied) { "Fixture $name exceeds declared byte bound $maximum" }
                    output.write(buffer, 0, count)
                    copied += count
                }
                require(!exact || copied == maximum) { "Fixture $name size $copied differs from declared $maximum" }
                if (consume != null || shellFixtures) output.fd.sync()
            }
        }
    }

    private data class StoragePlan(
        val sourceSizes: Map<String, Long>, val consumables: Map<String, Consumable>, val estimate: JSONObject
    )

    private fun storagePlan(case: JSONObject, recipe: JSONObject): StoragePlan {
        val expected = case.getJSONObject("expected").getLong("size")
        require(expected in 17L..(8L * 1024 * 1024 * 1024) && recipe.getLong("size") == expected)
        val block = StatFs(target.filesDir.absolutePath).blockSizeLong
        require(block > 0)
        fun allocated(bytes: Long): Long = Math.multiplyExact(Math.addExact(bytes, block - 1) / block, block)
        val sources = mutableListOf(recipe.getJSONObject("base"))
        val packs = recipe.getJSONObject("packs")
        val keys = packs.keys()
        while (keys.hasNext()) sources.add(packs.getJSONObject(keys.next()))
        val byName = mutableMapOf<String, Long>()
        val cacheChunks = mutableMapOf<String, Long>()
        val cacheBudget = 8L * 1024 * 1024
        val cacheEntryLimit = 4096
        var sourceBytes = 0L
        var largestChunk = 0L
        for (source in sources) {
            val size = source.getLong("size")
            val chunk = source.getLong("chunk")
            require(size > 0 && chunk in 1L..(16L * 1024 * 1024))
            sourceBytes = Math.addExact(sourceBytes, size)
            val name = source.getString("file")
            require(name == File(name).name && name.isNotEmpty())
            val previous = byName.put(name, size)
            require(previous == null || previous == size) { "Ambiguous source size for $name" }
            val hashes = source.getJSONArray("sha256")
            require(hashes.length().toLong() == (size - 1) / chunk + 1)
            for (index in 0 until hashes.length()) {
                val length = minOf(chunk, size - Math.multiplyExact(index.toLong(), chunk))
                val hash = hashes.getString(index)
                require(hash.matches(sha256Pattern))
                if (length <= cacheBudget) cacheChunks["$hash-$length"] = length
                largestChunk = maxOf(largestChunk, allocated(length))
            }
        }
        require(sourceBytes <= Math.addExact(Math.multiplyExact(expected, 2L), 32L * 1024 * 1024))
        val candidates = case.getJSONArray("localCandidates")
        require(candidates.length() in 1..16)
        val sourceSizes = mutableMapOf<String, Long>()
        var stagedBytes = 0L
        val consumables = mutableMapOf<String, Consumable>()
        var plannedReclaimed = 0L
        var maximumConsumedSource = 0L
        for (index in 0 until candidates.length()) {
            val name = relative(candidates.getString(index))
            val size = requireNotNull(byName[File(name).name]) { "Candidate $name has no declared source size" }
            require(sourceSizes.put(name, size) == null) { "Repeated candidate $name" }
            stagedBytes = Math.addExact(stagedBytes, allocated(size))
            if (consumeStagedInputs && candidateUses[name] == 1 && name.startsWith("candidates/") && name !in protectedInputs) {
                val stat = Os.lstat(externalFixture(name).path)
                // Hard links, unknown allocation and any non-regular source
                // receive no reclaim credit and are retained unchanged.
                if (OsConstants.S_ISREG(stat.st_mode) && stat.st_size == size && stat.st_ino > 0 && stat.st_nlink == 1L && stat.st_blocks > 0) {
                    val credit = minOf(allocated(size), Math.multiplyExact(stat.st_blocks, 512L))
                    consumables[name] = Consumable(stat.st_dev, stat.st_ino, size, credit)
                    plannedReclaimed = Math.addExact(plannedReclaimed, credit)
                    maximumConsumedSource = maxOf(maximumConsumedSource, allocated(size))
                }
            }
        }
        require(byName.keys.all { name -> sourceSizes.keys.any { File(it).name == name } }) { "A recipe source lacks a staged candidate" }
        val cachePayloadBytes = minOf(cacheBudget,
            cacheChunks.values.fold(0L) { total, length -> Math.addExact(total, length) })
        val cacheEntries = minOf(cacheEntryLimit, cacheChunks.size)
        // Eviction precedes temporary creation: cached payload plus the pending
        // temp share one budget, not a source-sized cache plus another chunk.
        // Allow one allocation-rounding block per retained leaf separately.
        val cacheBytes = Math.addExact(allocated(cachePayloadBytes),
            Math.multiplyExact(cacheEntries.toLong(), block))
        val outputBytes = allocated(expected)
        // Round file contents to filesystem blocks, allow four blocks per file
        // for metadata/directories, plus 64 MiB for journals/reports/OS churn.
        // Recipe/manifest and transport inputs already exist. Same-volume
        // inputs (APK or external files) are reflected in measured free space;
        // inputs on another volume do not consume this destination's capacity.
        val metadataBytes = Math.multiplyExact(
            Math.addExact(Math.addExact(cacheEntries.toLong(), sourceSizes.size.toLong()), 16L),
            Math.multiplyExact(block, 4L)
        )
        val reserveBytes = 64L * 1024 * 1024
        val netStaged = Math.subtractExact(stagedBytes, plannedReclaimed)
        // A consumed source cannot be reclaimed until its private copy is
        // complete. Credit its delivery allocation only after that copy;
        // netStaged + the largest such source bounds every staging prefix.
        val stagingPeak = Math.addExact(netStaged, maximumConsumedSource)
        val compositionPeak = listOf(netStaged, cacheBytes, outputBytes)
            .fold(0L) { total, bytes -> Math.addExact(total, bytes) }
        val required = Math.addExact(maxOf(stagingPeak, compositionPeak), Math.addExact(metadataBytes, reserveBytes))
        val compositionRequiredAfterStaging = listOf(cacheBytes, outputBytes, metadataBytes, reserveBytes)
            .fold(0L) { total, bytes -> Math.addExact(total, bytes) }
        return StoragePlan(sourceSizes, consumables, JSONObject().put("requiredBytesEstimate", required)
            .put("stagedSourceAllocatedBytes", stagedBytes).put("warmCacheAllocatedBytes", cacheBytes)
            .put("plannedReclaimedBytes", plannedReclaimed).put("plannedConsumedInputs", JSONArray(consumables.keys.toList()))
            .put("consumeStagedInputs", consumeStagedInputs).put("actualReclaimedBytes", 0L).put("actualConsumedInputBytes", 0L)
            .put("netStagedAllocatedBytes", netStaged).put("stagingPeakAdditionalBytes", stagingPeak)
            .put("compositionPeakAdditionalBytes", compositionPeak).put("compositionRequiredBytesAfterStaging", compositionRequiredAfterStaging)
            .put("outputAllocatedBytes", outputBytes).put("cacheReplacementAllocatedBytes", 0L)
            .put("cachePayloadBudgetBytes", cacheBudget).put("cacheEntryLimit", cacheEntryLimit)
            .put("maximumSourceChunkAllocationEstimateBytes", largestChunk)
            .put("workloadPolicy", "bounded-map-cache-raw-ranges-v2")
            .put("metadataAllowanceBytes", metadataBytes).put("operationalReserveBytes", reserveBytes)
            .put("filesystemBlockBytes", block).put("uniqueCacheChunks", cacheChunks.size)
            .put("basis", "Additional free space after recipe staging: max(net staged sources + largest consumed copy, net staged sources + bounded cache including its pending temporary + one renamed output), plus allocation rounding, metadata and reserve. Only disposable single-selected-case, single-link, identity-bound candidate delivery allocations receive planned reclaim credit; actual staging and composition capacity are checked separately."))
    }

    private fun recordStaging(plan: StoragePlan) {
        var unlinkedBytes = 0L
        var consumedBytes = 0L
        var closedAllocation = 0L
        for (source in plan.consumables.values) {
            if (!source.unlinked) continue
            unlinkedBytes = Math.addExact(unlinkedBytes, source.size)
            if (source.descriptorClosed) {
                consumedBytes = Math.addExact(consumedBytes, source.size)
                closedAllocation = Math.addExact(closedAllocation, source.creditedBytes)
            }
        }
        plan.estimate.put("actualUnlinkedInputBytes", unlinkedBytes).put("actualConsumedInputBytes", consumedBytes)
        try {
            val available = StatFs(target.filesDir.absolutePath).availableBytes
            var stagedAllocation = 0L
            for (name in plan.sourceSizes.keys) {
                val destination = File(root, "fixtures/$name")
                if (destination.exists()) {
                    stagedAllocation = Math.addExact(stagedAllocation, Math.multiplyExact(Os.lstat(destination.path).st_blocks, 512L))
                }
            }
            val observedRecovery = Math.addExact(
                Math.subtractExact(available, plan.estimate.getLong("availableBytesBeforeStaging")), stagedAllocation
            )
            plan.estimate.put("availableBytesAfterStaging", available).put("actualPrivateStagedAllocatedBytes", stagedAllocation)
                .put("observedRecoveryBytes", observedRecovery)
                .put("actualReclaimedBytes", minOf(closedAllocation, maxOf(0L, observedRecovery)))
                .put("reclamationCaveat", "actualConsumedInputBytes counts identity-bound delivery files unlinked and closed. actualReclaimedBytes is capped by that allocation and observed destination free-space recovery after accounting for private staged blocks; concurrent storage activity can affect this observation. Composition always rechecks actual free space.")
        } catch (error: Exception) {
            plan.estimate.put("stagingStorageObservationError", error.toString())
        }
    }

    private fun device(): JSONObject {
        val info = ActivityManager.MemoryInfo()
        (target.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).getMemoryInfo(info)
        val stat = StatFs(target.filesDir.absolutePath)
        return JSONObject().put("manufacturer", Build.MANUFACTURER).put("model", Build.MODEL)
            .put("device", Build.DEVICE).put("product", Build.PRODUCT).put("fingerprint", Build.FINGERPRINT)
            .put("sdk", Build.VERSION.SDK_INT).put("osRelease", Build.VERSION.RELEASE)
            .put("supportedAbis", JSONArray(Build.SUPPORTED_ABIS.toList())).put("process64Bit", Process.is64Bit())
            .put("processArch", System.getProperty("os.arch"))
            .put("hardware", Build.HARDWARE).put("socManufacturer", if (Build.VERSION.SDK_INT >= 31) Build.SOC_MANUFACTURER else JSONObject.NULL)
            .put("socModel", if (Build.VERSION.SDK_INT >= 31) Build.SOC_MODEL else JSONObject.NULL)
            .put("totalPhysicalRamBytes", info.totalMem).put("availableRamBytes", info.availMem)
            .put("lowRamDevice", (target.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).isLowRamDevice)
            .put("availableStorageBytes", stat.availableBytes).put("storageTotalBytes", stat.totalBytes)
    }

    // Constraints are requests, not observations. Only Android APIs above supply
    // measurements; no fixture, shell helper or caller-provided probe is trusted.
    private fun hardwareAdmission(): Boolean {
        val probe = arguments.getString("evaluationProbeOnly", "false")!!
        require(probe == "false" || probe == "true")
        val inventory = arguments.getString("evaluationInventoryOnly", "false")!!
        require(inventory == "false" || inventory == "true")
        require(probe != "true" || inventory != "true") { "Inventory and native admission are distinct modes" }
        val observed = device()
        val receipt = JSONObject().put("schema", 1).put("runId", runId)
            .put("mode", if (inventory == "true") "inventory-only" else if (probe == "true") "info-only" else "workload")
            .put("device", observed).put("measurementSource", "in-process Android platform APIs")
            .put("physicalDeviceStatus", "unverified: physical inventory evidence is required separately")
            .put("status", "measured").put("workloadExecuted", false)
        save("hardware-probe", receipt)
        try {
            require(observed.getLong("totalPhysicalRamBytes") > 0) { "Android did not report physical RAM" }
            fun ramBound(name: String): Long? = arguments.getString(name)?.let {
                val mib = it.toLong()
                require(mib in 1L..1048576L) { "RAM constraint must be positive MiB" }
                Math.multiplyExact(mib, 1024L * 1024L)
            }
            val minimum = ramBound("evaluationMinRamMiB")
            val maximum = ramBound("evaluationMaxRamMiB")
            require(minimum == null || maximum == null || minimum <= maximum) { "Inverted RAM constraints" }
            receipt.put("minimumRamBytesRequested", minimum ?: JSONObject.NULL)
                .put("maximumRamBytesRequested", maximum ?: JSONObject.NULL)
            val actual = observed.getLong("totalPhysicalRamBytes")
            require(minimum == null || actual >= minimum) { "Actual physical RAM is below requested range: $actual" }
            require(maximum == null || actual <= maximum) { "Actual physical RAM exceeds requested range: $actual" }
            val lowRam = arguments.getString("evaluationRequireLowRamDevice", "false")!!
            require(lowRam == "false" || lowRam == "true")
            receipt.put("lowRamDeviceRequired", lowRam == "true")
            require(lowRam != "true" || observed.getBoolean("lowRamDevice")) { "Android does not classify this as a low-RAM device" }
            arguments.getString("evaluationRequiredAbi")?.let { abi ->
                require(abi in setOf("armeabi-v7a", "arm64-v8a", "x86", "x86_64")) { "Unsupported requested ABI" }
                receipt.put("requiredAbi", abi)
                require(Build.SUPPORTED_ABIS.contains(abi)) { "Requested ABI is not supported by actual Android hardware/runtime: $abi" }
                if (inventory != "true") {
                    require(Process.is64Bit() == (abi == "arm64-v8a" || abi == "x86_64")) { "Installed APK process bitness does not match requested ABI: $abi" }
                    val arch = System.getProperty("os.arch", "")
                    require(if (abi.startsWith("arm")) arch.startsWith("arm") || arch == "aarch64" else arch in setOf("i686", "i386", "x86", "x86_64", "amd64")) {
                        "Installed process architecture does not match requested ABI: $arch"
                    }
                }
            }
            val apkHash = MessageDigest.getInstance("SHA-256")
            File(target.applicationInfo.sourceDir).inputStream().use { input ->
                val buffer = ByteArray(128 * 1024)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    apkHash.update(buffer, 0, count)
                }
            }
            val actualApkHash = apkHash.digest().joinToString("") { "%02x".format(it) }
            receipt.put("targetApkSha256", actualApkHash)
            arguments.getString("evaluationTargetApkSha256")?.let {
                require(it.matches(sha256Pattern) && it.equals(actualApkHash, ignoreCase = true)) { "Installed target APK differs from the requested build" }
            }
            if (inventory == "true") {
                // Java/platform inventory is usable without an SDK/native
                // library. It cannot authorize fixture preparation or compute.
                save("hardware-inventory", receipt.put("nativeLibraryLoaded", false))
                return true
            }
            // Prove the actual installed native bridge loads before any fixture
            // access. A probe never invokes evaluate() or composition.
            bridge = target.classLoader.loadClass("org.kiwix.workbench.AndroidEvaluationBridge")
            bridge.getMethod("phase").invoke(null)
            receipt.put("nativeLibraryLoaded", true).put("status", "admitted")
            save("hardware-admission", receipt)
            return probe == "true"
        } catch (error: Throwable) {
            save("hardware-admission", receipt.put("status", "rejected").put("reason", error.toString()))
            throw error
        }
    }

    private fun memory(): JSONObject {
        val info = Debug.MemoryInfo()
        Debug.getMemoryInfo(info)
        val runtime = Runtime.getRuntime()
        val status = try { File("/proc/self/status").readLines().filter { it.startsWith("VmRSS:") || it.startsWith("VmHWM:") }.associate { line -> line.substringBefore(':') to line.substringAfter(':').trim().split(Regex("\\s+"))[0].toLong() * 1024 } } catch (_: Exception) { emptyMap<String, Long>() }
        return JSONObject().put("elapsedRealtimeMs", SystemClock.elapsedRealtime())
            .put("rssBytes", status["VmRSS"] ?: JSONObject.NULL).put("processLifetimeRssHighWaterBytes", status["VmHWM"] ?: JSONObject.NULL).put("pssBytes", info.totalPss.toLong() * 1024)
            .put("nativePssBytes", info.nativePss.toLong() * 1024).put("dalvikPssBytes", info.dalvikPss.toLong() * 1024)
            .put("nativeHeapAllocatedBytes", Debug.getNativeHeapAllocatedSize())
            .put("jvmUsedHeapBytes", runtime.totalMemory() - runtime.freeMemory())
            .put("jvmCommittedHeapBytes", runtime.totalMemory()).put("availableStorageBytes", StatFs(target.filesDir.absolutePath).availableBytes)
    }

    private fun save(name: String, row: JSONObject) {
        if (!row.has("schema")) row.put("schema", 1)
        // Internal receipts and runner/logcat output survive an external report
        // directory failure, including failures before manifest parsing.
        try {
            require(privateReports.isDirectory || privateReports.mkdirs())
            File(privateReports, "$name.json").writeText(row.toString(2))
        } catch (error: Exception) { row.put("privateReceiptWriteError", error.toString()) }
        try {
            if (::reports.isInitialized) {
                require(reports.isDirectory || reports.mkdirs())
                File(reports, "$name.json").writeText(row.toString(2))
            }
        } catch (error: Exception) { row.put("externalReceiptWriteError", error.toString()) }
        Log.i("AndroidEvaluation", "$name $row")
        instrumentation.sendStatus(0, Bundle().apply { putString("stream", "ANDROID_EVALUATION $name ${row}\n") })
    }

    private fun bootDiagnostics(): JSONObject {
        val paths = mutableListOf(target.filesDir.path, AndroidEvaluationFixtureDelivery.INPUT_ROOT)
        try {
            target.getExternalFilesDir(null)?.let {
                paths.add(it.path)
                paths.add(File(it, "android-evaluation-inputs/manifest.json").path)
            }
        } catch (error: Exception) { paths.add("external-storage-query-failed:$error") }
        return JSONObject().put("schema", 1).put("runId", runId).put("uid", Os.getuid()).put("gid", Os.getgid())
            .put("pid", Process.myPid()).put("sdk", Build.VERSION.SDK_INT)
            .put("targetPackage", target.packageName).put("testPackage", instrumentation.context.packageName)
            .put("targetApplicationUid", target.applicationInfo.uid).put("testApplicationUid", instrumentation.context.applicationInfo.uid)
            .put("privateReportsDirectory", privateReports.path)
            .put("paths", JSONArray(paths.map { AndroidEvaluationFixtureDelivery.diagnosePath(it) }))
    }

    private fun execute(case: JSONObject, mode: String, repetition: Int, storage: JSONObject): Boolean {
        val baseline = memory()
        val running = AtomicBoolean(true)
        val byPhase = mutableMapOf<Int, JSONObject>()
        val phaseMethod = bridge.getMethod("phase")
        val sampler = Thread({
            while (running.get()) {
                try {
                    val phase = phaseMethod.invoke(null) as Int
                    val sample = memory()
                    synchronized(byPhase) {
                        val aggregate = byPhase.getOrPut(phase) { JSONObject().put("samples", 0).put("first", sample).put("peak", JSONObject()) }
                        aggregate.put("samples", aggregate.getInt("samples") + 1)
                        val peak = aggregate.getJSONObject("peak")
                        for (key in listOf("rssBytes", "pssBytes", "nativePssBytes", "dalvikPssBytes", "nativeHeapAllocatedBytes", "jvmUsedHeapBytes", "jvmCommittedHeapBytes")) {
                            if (!sample.isNull(key) && (!peak.has(key) || sample.getLong(key) > peak.getLong(key))) peak.put(key, sample.getLong(key))
                        }
                        val free = sample.getLong("availableStorageBytes")
                        if (!aggregate.has("minimumAvailableStorageBytes") || free < aggregate.getLong("minimumAvailableStorageBytes")) aggregate.put("minimumAvailableStorageBytes", free)
                    }
                } catch (error: Exception) {
                    synchronized(byPhase) { byPhase.getOrPut(-1) { JSONObject() }.put("samplingError", error.toString()) }
                }
                try { Thread.sleep(samplePeriodMs) } catch (_: InterruptedException) { break }
            }
        }, "android-evaluation-memory")
        val start = SystemClock.elapsedRealtimeNanos()
        sampler.start()
        val result = try {
            val method = bridge.getMethod("evaluate", Context::class.java, String::class.java, String::class.java, String::class.java)
            JSONObject(method.invoke(null, target, root.name, case.getString("id"), mode) as String)
        } catch (error: Throwable) { JSONObject().put("ok", false).put("error", error.toString()) }
        finally { running.set(false); sampler.interrupt(); sampler.join() }
        val phaseMemory = JSONObject()
        synchronized(byPhase) { byPhase.forEach { (key, value) -> phaseMemory.put(key.toString(), value) } }
        val row = JSONObject().put("schema", 1).put("caseId", case.getString("id")).put("tier", case.getString("tier"))
            .put("mode", mode).put("repetition", repetition).put("status", if (result.optBoolean("ok")) "passed" else "failed")
            .put("instrumentationElapsedUs", (SystemClock.elapsedRealtimeNanos() - start) / 1000)
            .put("device", device()).put("baseline", baseline).put("phaseMemory", phaseMemory).put("native", result)
            .put("requiredBytesEstimate", storage.getLong("requiredBytesEstimate")).put("storageEstimate", storage)
            .put("samplingPeriodMs", samplePeriodMs).put("phaseIds", JSONObject("{\"0\":\"idle\",\"1\":\"preparation\",\"2\":\"composition\",\"3\":\"identitySha256\",\"4\":\"retainedChecksum\",\"5\":\"structuralValidation\",\"6\":\"combinedIntegrity\",\"7\":\"contentAssertions\"}"))
            .put("memoryCaveat", "Sampled process peaks are observed lower bounds including JVM, instrumentation, sampler and native runtime; short phases may have zero samples. Phase is read before PSS collection and a boundary can overlap sampling. Process-lifetime VmHWM includes prior phases and is not attributed to this phase.")
        save("${case.getString("id")}-${mode}-${repetition}", row)
        return result.optBoolean("ok")
    }

    @Test
    fun productionComposerAndChecksumsOnDevice() {
        try {
            root = File(target.filesDir, "android-evaluation/$runId")
            reports = privateReports
            val external = target.getExternalFilesDir(null)
            if (external != null) reports = File(external, "android-evaluation/$runId")
            save("boot", bootDiagnostics().put("status", "starting"))
            require(root.mkdirs()) { "Private evaluation directory unavailable" }
            runEvaluation()
        } catch (error: Throwable) {
            save("boot-failed", bootDiagnostics().put("status", "failed").put("reason", error.toString())
                .put("stack", Log.getStackTraceString(error)))
            throw error
        }
    }

    private fun runEvaluation() {
        require(target.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) { "Evaluation requires a debuggable target APK" }
        if (hardwareAdmission()) return
        val externalMode = arguments.getString("evaluationExternalFixtures", "false")!!
        require(externalMode == "false" || externalMode == "true")
        if (externalMode == "true") {
            val transport = arguments.getString("evaluationFixtureTransport", "app-owned-external-files")
            require(transport == "app-owned-external-files" || transport == "shell-tmp")
            shellFixtures = transport == "shell-tmp"
            if (!shellFixtures) {
                val owned = requireNotNull(target.getExternalFilesDir(null)) { "App-owned fixture storage unavailable" }.canonicalFile
                externalFixtures = File(owned, "android-evaluation-inputs")
            }
        } else {
            require(arguments.getString("evaluationFixtureTransport") == null) { "A fixture transport requires evaluationExternalFixtures=true" }
        }
        val consumeMode = arguments.getString("evaluationConsumeStagedInputs", "false")!!
        require(consumeMode == "false" || consumeMode == "true")
        consumeStagedInputs = consumeMode == "true"
        require(!consumeStagedInputs || (!shellFixtures && externalFixtures != null)) {
            "Consuming requires app-owned external inputs; shell-tmp is read-only with zero reclaim credit"
        }
        if (shellFixtures) {
            val diagnostics = withDeliveryCommand("diagnose") { input -> JSONObject(input.readUTF()) }
            save("delivery-boot", diagnostics)
            require(diagnostics.optBoolean("ok") && diagnostics.getInt("uid") == 2000) { "Shell delivery preflight failed: $diagnostics" }
        }
        copyFixture("manifest.json")
        val manifestFile = File(root, "fixtures/manifest.json")
        require(manifestFile.length() in 1L..(32L * 1024 * 1024))
        val manifestBytes = ByteArray(manifestFile.length().toInt())
        java.io.DataInputStream(manifestFile.inputStream()).use { input -> input.readFully(manifestBytes); require(input.read() == -1) }
        val manifest = JSONObject(String(manifestBytes, Charsets.UTF_8))
        require(manifest.getInt("schema") == 1)
        val cases = manifest.getJSONArray("cases")
        require(cases.length() in 1..12)
        val selected = arguments.getString("evaluationCases")?.split(',')?.toSet()
        val repetitions = arguments.getString("evaluationRepetitions", "3")!!.toInt()
        require(repetitions in 1..3)
        val xlargeRepetitions = arguments.getString("evaluationXLargeRepetitions", repetitions.toString())!!.toInt()
        require(xlargeRepetitions in 1..repetitions)
        val defaultTiers = mutableListOf("smoke", "city", "large")
        if ((0 until cases.length()).any { cases.getJSONObject(it).getString("tier") == "xlarge" }) defaultTiers.add("xlarge")
        val tiers = arguments.getString("evaluationTiers", defaultTiers.joinToString(","))!!.split(',').toSet()
        if (consumeStagedInputs) {
            for (index in 0 until cases.length()) {
                val case = cases.getJSONObject(index)
                protectedInputs.add(relative(case.getString("recipe")))
                if (case.getString("tier") !in tiers || (selected != null && case.getString("id") !in selected)) continue
                val candidates = case.getJSONArray("localCandidates")
                require(candidates.length() in 1..16)
                val names = (0 until candidates.length()).map { relative(candidates.getString(it)) }.toSet()
                for (name in names) candidateUses[name] = (candidateUses[name] ?: 0) + 1
            }
        }
        val build = JSONObject().put("schema", 1).put("device", device()).put("repetitionsRequested", repetitions)
            .put("runId", root.name).put("reportsDirectory", reports.absolutePath)
            .put("fixtureTransport", if (shellFixtures) "shell-tmp" else if (externalFixtures == null) "embedded-assets" else "app-owned-external-files")
            .put("fixtureTransportMode", if (consumeStagedInputs) "consume-disposable-single-case-candidates" else "read-only")
            .put("consumeStagedInputs", consumeStagedInputs)
            .put("deliveryInputsRetained", shellFixtures)
            .put("deliveryStorageCaveat", if (shellFixtures) "All shell-owned delivery inputs remain allocated. No source unlink or planned/actual reclaim credit; full bounded private staging, warm cache and output headroom is required in addition to delivery files already reflected in device free space." else JSONObject.NULL)
            .put("manifestSha256", MessageDigest.getInstance("SHA-256").digest(manifestBytes).joinToString("") { "%02x".format(it) })
            .put("benchmarkBuildLabel", arguments.getString("evaluationBuildLabel", "UNSPECIFIED: do not infer release performance"))
            .put("nativeSdkManifestSha256", arguments.getString("evaluationNativeSdkManifestSha256", "UNREPORTED"))
            .put("debugAssertions", true).put("samplingPeriodMs", samplePeriodMs)
        build.put("xlargeRepetitionsRequested", xlargeRepetitions)
        save("run", build)
        var failures = 0
        var executed = 0
        var storageSkipped = 0
        caseLoop@ for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            val id = case.getString("id")
            require(id.length <= 80 && id.matches(Regex("[A-Za-z0-9_-]+")))
            if (case.getString("tier") !in tiers || (selected != null && id !in selected)) continue
            // Stage only the bounded recipe before calculating additional free
            // space; never copy a multi-GiB candidate just to discover its size.
            val staged = mutableListOf<File>()
            var storageEstimate: JSONObject? = null
            var casePlan: StoragePlan? = null
            try {
                val recipeName = relative(case.getString("recipe"))
                val recipeFile = File(root, "fixtures/$recipeName")
                staged.add(recipeFile)
                copyFixture(recipeName)
                val plan = storagePlan(case, JSONObject(recipeFile.readText(Charsets.UTF_8)))
                casePlan = plan
                storageEstimate = plan.estimate
                val required = plan.estimate.getLong("requiredBytesEstimate")
                val available = StatFs(target.filesDir.absolutePath).availableBytes
                plan.estimate.put("availableBytesBeforeStaging", available)
                if (available < required) {
                    storageSkipped++
                    save("${id}-storage-skipped", JSONObject().put("schema", 1).put("caseId", id).put("tier", case.getString("tier"))
                        .put("status", "skipped").put("reason", "insufficient natural app-files storage")
                        .put("requiredBytesEstimate", required).put("storageEstimate", plan.estimate).put("availableBytes", available))
                    continue
                }
                for ((name, size) in plan.sourceSizes) {
                    val block = plan.estimate.getLong("filesystemBlockBytes")
                    val copyAllocation = Math.multiplyExact(Math.addExact(size, block - 1) / block, block)
                    val headroom = Math.addExact(plan.estimate.getLong("metadataAllowanceBytes"), plan.estimate.getLong("operationalReserveBytes"))
                    val availableForCopy = StatFs(target.filesDir.absolutePath).availableBytes
                    if (availableForCopy < Math.addExact(copyAllocation, headroom)) {
                        storageSkipped++
                        recordStaging(plan)
                        save("${id}-storage-skipped", JSONObject().put("schema", 1).put("caseId", id).put("tier", case.getString("tier"))
                            .put("status", "skipped").put("reason", "insufficient observed storage for next bounded source copy")
                            .put("requiredBytesEstimate", required).put("storageEstimate", plan.estimate).put("availableBytes", availableForCopy))
                        continue@caseLoop
                    }
                    staged.add(File(root, "fixtures/$name"))
                    copyFixture(name, size, exact = true, consume = plan.consumables[name])
                }
                recordStaging(plan)
                val availableAfterStaging = StatFs(target.filesDir.absolutePath).availableBytes
                if (availableAfterStaging < plan.estimate.getLong("compositionRequiredBytesAfterStaging")) {
                    storageSkipped++
                    save("${id}-storage-skipped", JSONObject().put("schema", 1).put("caseId", id).put("tier", case.getString("tier"))
                        .put("status", "skipped").put("reason", "insufficient observed storage for production cache and output after staging")
                        .put("requiredBytesEstimate", required).put("storageEstimate", plan.estimate).put("availableBytes", availableAfterStaging))
                    continue@caseLoop
                }
                val caseRepetitions = if (case.getString("tier") == "xlarge") xlargeRepetitions else repetitions
                for (repetition in 1..caseRepetitions) {
                    for (mode in listOf("cold", "warm")) { executed++; if (!execute(case, mode, repetition, plan.estimate)) failures++ }
                }
                if (case.getString("tier") == "smoke") {
                    for (mode in listOf("collision", "corruption", "cancel", "boundaries")) { executed++; if (!execute(case, mode, 1, plan.estimate)) failures++ }
                }
            } catch (error: Exception) {
                failures++
                casePlan?.let { recordStaging(it) }
                save("${id}-staging-failed", JSONObject().put("schema", 1).put("caseId", id).put("status", "failed").put("reason", error.toString()).put("device", device())
                    .put("requiredBytesEstimate", storageEstimate?.getLong("requiredBytesEstimate") ?: JSONObject.NULL)
                    .put("storageEstimate", storageEstimate ?: JSONObject.NULL))
            } finally { staged.forEach { it.delete() } }
        }
        save("summary", JSONObject().put("schema", 1).put("executed", executed).put("failures", failures).put("storageSkippedCases", storageSkipped).put("device", device()))
        assertTrue("No evaluation workload executed", executed > 0)
        assertTrue("$failures evaluation operations failed; retrieve ${reports.absolutePath}", failures == 0)
    }
}

/**
 * Out-of-band test fixture reader, launched from the installed test APK by
 * UiAutomation as UID 2000. No manifest component, production JNI, permission
 * adoption, root, chmod/chown, source deletion, or arbitrary input root.
 *
 * Submission contract: preserve every frozen manifest/recipe/candidate byte;
 * map each old filesToPush regularFile.content GCS object to
 * /data/local/tmp/kiwix-evaluation-inputs/<same relative filename>. Set
 * evaluationExternalFixtures=true,evaluationFixtureTransport=shell-tmp,
 * evaluationConsumeStagedInputs=false. This transport retains ALL delivered
 * files, so disposable app-owned-source reclaim savings do not apply.
 *
 * Official path contract:
 * https://cloud.google.com/sdk/gcloud/reference/firebase/test/android/run#--other-files
 * Shell descriptor contracts (stdout API24–30; Rw API31–33; Rwe API34+).
 * Stdout-only transport cannot establish stderr contents or child exit status;
 * acceptance always requires complete framed bytes and the identity footer.
 * https://developer.android.com/reference/android/app/UiAutomation#executeShellCommand(java.lang.String)
 * https://developer.android.com/reference/android/app/UiAutomation#executeShellCommandRw(java.lang.String)
 * https://developer.android.com/reference/android/app/UiAutomation#executeShellCommandRwe(java.lang.String)
 * Runtime entry point (CLASSPATH, start class and tool-mode stdout):
 * https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/cmds/app_process/app_main.cpp
 *
 * A06 API 35 matrix-mkkw7glnjk62a denied child lstat before manifest parsing.
 * Its artifacts do not establish the denied child's owner/mode or why access
 * was denied. A target-owned directory existing is not proof its pushed
 * children are app-readable; the boot receipts now capture both views.
 */
object AndroidEvaluationFixtureDelivery {
    const val INPUT_ROOT = "/data/local/tmp/kiwix-evaluation-inputs"
    const val MAGIC = 0x4b455631

    private fun statJson(stat: android.system.StructStat): JSONObject =
        JSONObject().put("device", stat.st_dev).put("inode", stat.st_ino).put("size", stat.st_size)
            .put("mode", stat.st_mode).put("uid", stat.st_uid).put("gid", stat.st_gid)
            .put("links", stat.st_nlink).put("blocks", stat.st_blocks)
            .put("mtime", stat.st_mtime).put("ctime", stat.st_ctime)

    private fun identity(stat: android.system.StructStat): String =
        listOf(stat.st_dev, stat.st_ino, stat.st_size, stat.st_mode.toLong(), stat.st_uid.toLong(),
            stat.st_gid.toLong(), stat.st_nlink, stat.st_mtime, stat.st_ctime).joinToString(":")

    fun diagnosePath(path: String): JSONObject {
        val row = JSONObject().put("path", path)
        try { row.put("lstat", statJson(Os.lstat(path))) }
        catch (error: Exception) { row.put("lstatError", error.toString()) }
        try { row.put("canonicalPath", File(path).canonicalPath) }
        catch (error: Exception) { row.put("canonicalError", error.toString()) }
        return row
    }

    private fun diagnostics(): JSONObject {
        val paths = listOf("/data", "/data/local", "/data/local/tmp", INPUT_ROOT, "$INPUT_ROOT/manifest.json",
            "/sdcard/Android/data/org.kiwix.workbench/files",
            "/sdcard/Android/data/org.kiwix.workbench/files/android-evaluation-inputs",
            "/sdcard/Android/data/org.kiwix.workbench/files/android-evaluation-inputs/manifest.json")
        val row = JSONObject().put("ok", true).put("uid", Os.getuid()).put("gid", Os.getgid())
            .put("pid", Process.myPid()).put("paths", JSONArray(paths.map { diagnosePath(it) }))
        try { row.put("selinuxContext", File("/proc/self/attr/current").readText().trim()) }
        catch (error: Exception) { row.put("selinuxContextError", error.toString()) }
        return row
    }

    private fun checkedDirectories(source: File): List<Pair<File, android.system.StructStat>> {
        val directories = mutableListOf<Pair<File, android.system.StructStat>>()
        var current = source.parentFile
        while (current != null && current.path != "/") {
            val stat = Os.lstat(current.path)
            require(OsConstants.S_ISDIR(stat.st_mode) && !OsConstants.S_ISLNK(stat.st_mode)) {
                "Fixture parent must be a non-symlink directory: ${current.path}"
            }
            directories.add(current to stat)
            current = current.parentFile
        }
        require(source.canonicalFile == source.absoluteFile) { "Fixture canonical path escape rejected" }
        return directories
    }

    private fun requireDirectories(directories: List<Pair<File, android.system.StructStat>>) {
        for ((directory, before) in directories) {
            val after = Os.lstat(directory.path)
            require(OsConstants.S_ISDIR(after.st_mode) && after.st_dev == before.st_dev && after.st_ino == before.st_ino &&
                after.st_uid == before.st_uid && after.st_gid == before.st_gid && after.st_mode == before.st_mode) {
                "Fixture directory identity or permissions changed: ${directory.path}"
            }
        }
    }

    @JvmStatic
    fun main(args: Array<String>) {
        val output = DataOutputStream(FileOutputStream(FileDescriptor.out))
        output.writeInt(MAGIC)
        output.flush()
        try {
            require(args.size == 3 && (args[0] == "read" || args[0] == "diagnose"))
            if (args[0] == "diagnose") {
                output.writeUTF(diagnostics().toString())
                output.flush()
                return
            }
            require(Os.getuid() == 2000) { "Fixture delivery requires unprivileged shell UID 2000" }
            val name = String(Base64.decode(args[1], Base64.NO_WRAP or Base64.URL_SAFE), Charsets.UTF_8)
            require(name.isNotEmpty() && name.length <= 240 && !name.contains('\\') && !name.startsWith('/') &&
                name.split('/').all { it.isNotEmpty() && it != "." && it != ".." })
            val maximum = args[2].toLong()
            require(maximum in 0L..(8L * 1024 * 1024 * 1024 + 32L * 1024 * 1024))
            val source = File(INPUT_ROOT, name)
            val directories = checkedDirectories(source)
            val before = Os.lstat(source.path)
            require(OsConstants.S_ISREG(before.st_mode) && before.st_size in 0L..maximum) {
                "Fixture must be a regular file within declared bound: ${diagnosePath(source.path)}"
            }
            val descriptor = Os.open(source.path, OsConstants.O_RDONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW or OsConstants.O_NONBLOCK, 0)
            try {
                val opened = Os.fstat(descriptor)
                require(OsConstants.S_ISREG(opened.st_mode) && identity(opened) == identity(before)) {
                    "Fixture identity or size changed while opening"
                }
                requireDirectories(directories)
                output.writeUTF(JSONObject().put("ok", true).put("uid", Os.getuid()).put("file", name)
                    .put("size", opened.st_size).put("stat", statJson(opened)).put("identity", identity(opened)).toString())
                output.flush()
                FileInputStream(descriptor).use { input ->
                    val buffer = ByteArray(128 * 1024)
                    var remaining = opened.st_size
                    while (remaining > 0) {
                        val count = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                        require(count > 0) { "Fixture truncated during delivery" }
                        output.write(buffer, 0, count)
                        remaining -= count
                    }
                    require(input.read() == -1) { "Fixture grew during delivery" }
                }
                val after = Os.fstat(descriptor)
                require(identity(after) == identity(opened) && identity(Os.lstat(source.path)) == identity(opened)) {
                    "Fixture identity, size or metadata changed during delivery"
                }
                requireDirectories(directories)
                require(source.canonicalFile == source.absoluteFile)
                output.writeUTF(JSONObject().put("ok", true).put("identity", identity(after)).put("sourceRetained", true).toString())
                output.flush()
            } finally { Os.close(descriptor) }
        } catch (error: Throwable) {
            Log.e("AndroidEvaluationDelivery", "Fixture delivery failed", error)
            // If payload streaming already began, the consumer will reject a
            // truncated payload or failed footer; it can never accept success.
            output.writeUTF(JSONObject().put("ok", false).put("reason", error.toString())
                .put("stack", Log.getStackTraceString(error).take(12000)).put("uid", Os.getuid()).toString())
            output.flush()
        }
    }
}
