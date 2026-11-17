import com.android.build.api.artifact.SingleArtifact
import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val inventoryOnlyProperty = providers.gradleProperty("kiwixEvaluationInventoryOnly").orElse("false").get()
require(inventoryOnlyProperty == "true" || inventoryOnlyProperty == "false")
val inventoryOnlyBuild = inventoryOnlyProperty == "true"
val bundledKokoroDir = providers.gradleProperty("kiwixBundledKokoroDir").orNull
if (!inventoryOnlyBuild && bundledKokoroDir != null) {
    val bundled = file(bundledKokoroDir)
    val required = listOf(
        "model.fp16.onnx", "voices.bin", "tokens.txt",
        "lexicon-us-en.txt", "lexicon-gb-en.txt",
    )
    required.forEach { name ->
        require(bundled.resolve("kokoro-en-fp16-v1_0/$name").isFile) {
            "kiwixBundledKokoroDir is missing kokoro-en-fp16-v1_0/$name: $bundled"
        }
    }
    require(bundled.resolve("kokoro-en-fp16-v1_0/espeak-ng-data").isDirectory) {
        "kiwixBundledKokoroDir is missing kokoro-en-fp16-v1_0/espeak-ng-data: $bundled"
    }
}
// Do not even register Rust/native tasks for the independent inventory APK.
if (!inventoryOnlyBuild) {
    apply(plugin = "rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    namespace = "org.kiwix.workbench"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = if (inventoryOnlyBuild) "org.kiwix.workbench.inventory" else "org.kiwix.workbench"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = !inventoryOnlyBuild
    }
    if (!inventoryOnlyBuild && bundledKokoroDir != null) {
        sourceSets.getByName("main").assets.srcDir(file(bundledKokoroDir))
    }
    // A probe/external-delivery test APK must not embed multi-GiB fixtures.
    val embedEvaluationFixtures = providers.gradleProperty("kiwixEmbedEvaluationFixtures").orElse("true").get()
    require(embedEvaluationFixtures == "true" || embedEvaluationFixtures == "false")
    if (!inventoryOnlyBuild && embedEvaluationFixtures == "true") {
        sourceSets.getByName("androidTest").assets.srcDir(
            rootProject.projectDir.resolve("../../../.native/evaluation/android-assets")
        )
    }
    if (inventoryOnlyBuild) {
        sourceSets.getByName("main").manifest.srcFile("src/inventory/AndroidManifest.xml")
        for (name in listOf("main", "debug", "release")) {
            sourceSets.getByName(name).apply {
                java.setSrcDirs(emptyList<String>())
                res.setSrcDirs(emptyList<String>())
                assets.setSrcDirs(emptyList<String>())
                jniLibs.setSrcDirs(emptyList<String>())
            }
        }
        // Instrumentation requires executable target code, but never the reader.
        sourceSets.getByName("main").java.srcDir("src/inventory/java")
        sourceSets.getByName("androidTest").assets.setSrcDirs(emptyList<String>())
        // Also reject transitive native payloads in inventory packaging.
        packaging.jniLibs.excludes.add("**/*.so")
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

if (!inventoryOnlyBuild) {
    extensions.configure<Config>("rust") {
        rootDirRel = "../../../"
    }
}

dependencies {
    if (!inventoryOnlyBuild) {
        implementation("androidx.webkit:webkit:1.14.0")
        implementation("androidx.appcompat:appcompat:1.7.1")
        implementation("androidx.activity:activity-ktx:1.10.1")
        implementation("com.google.android.material:material:1.12.0")
        implementation("androidx.lifecycle:lifecycle-process:2.10.0")
        // Required by src-tauri/src/android_tls.rs; version-locked to Cargo.lock.
        implementation(project(":rustls-platform-verifier"))
        testImplementation("junit:junit:4.13.2")
    }
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    if (!inventoryOnlyBuild) androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

if (!inventoryOnlyBuild) {
    apply(from = file("tauri.build.gradle.kts"))
}

// ---------------------------------------------------------------------------
// 16 KB page-size gate
//
// Android 15+ devices can run with 16 KiB memory pages. Native libraries must
// align every PT_LOAD segment to 16 KiB and the APK must zip-align its
// uncompressed .so entries to 16 KiB. NDK r28 and the Rust linker already emit
// that alignment because the mobile SDK manifest's
// `-Wl,-z,max-page-size=16384` reaches the final link; this gate fails the
// build if a toolchain, SDK or packaging change ever regresses it.

androidComponents {
    onVariants { variant ->
        val capitalized = variant.name.replaceFirstChar { it.uppercaseChar() }
        val apkDirectory = variant.artifacts.get(SingleArtifact.APK)
        val sdkDirectory = sdkComponents.sdkDirectory
        val verify = tasks.register<Verify16KbPageAlignmentTask>("verify${capitalized}16KbPageAlignment") {
            group = "verification"
            description = "Fails when ${variant.name} native libraries or APK are not 16 KB page compatible"
            this.apkDirectory.set(apkDirectory)
            this.sdkDirectory.set(sdkDirectory)
        }
        // AGP registers assemble tasks after the variant callbacks run, so match
        // lazily instead of resolving the task name here.
        tasks.configureEach {
            if (name == "assemble$capitalized") finalizedBy(verify)
        }
    }
}
