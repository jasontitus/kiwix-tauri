import org.jetbrains.kotlin.gradle.dsl.JvmTarget
plugins { id("com.android.application"); id("org.jetbrains.kotlin.android") }

// The app's own network security config, so revocation fetches obey the same
// cleartext policy as the shipping app.
val appNetworkConfig = rootDir.resolve("../../../src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml")
val probeRes = layout.buildDirectory.dir("generated/probe-res").get().asFile
val copyNetworkConfig = tasks.register<Copy>("copyAppNetworkConfig") {
    from(appNetworkConfig); into(probeRes.resolve("xml"))
}
tasks.named("preBuild") { dependsOn(copyNetworkConfig) }

android {
    namespace = "org.kiwix.probe"
    compileSdk = 36
    defaultConfig { applicationId = "org.kiwix.probe"; minSdk = 24; targetSdk = 36; versionCode = 1; versionName = "1" }
    sourceSets.getByName("main").res.srcDir(probeRes)
    buildTypes {
        // Same shrinking mode as the app's release build; debug-signed for install.
        getByName("release") { optimization { enable = true }; signingConfig = signingConfigs.getByName("debug") }
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_1_8; targetCompatibility = JavaVersion.VERSION_1_8 }
}
kotlin { compilerOptions { jvmTarget = JvmTarget.JVM_1_8 } }
dependencies {
    // -PprobeWithoutVerifier=true builds a deliberately broken package to check
    // the initialization failure path.
    if (!providers.gradleProperty("probeWithoutVerifier").isPresent) {
        implementation(project(":rustls-platform-verifier"))
    }
}
