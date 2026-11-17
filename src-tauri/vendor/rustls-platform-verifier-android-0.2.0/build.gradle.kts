import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// JVM half of the `rustls-platform-verifier` crate (Android release-support
// crate `rustls-platform-verifier-android` 0.2.0), built from pinned upstream
// source instead of the upstream GitHub-hosted Maven archive. The version must
// equal Cargo.lock's `rustls-platform-verifier-android`; see
// upstream-provenance.json and scripts/test-android-platform-verifier.mjs,
// which enforces it.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "org.rustls.platformverifier"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        // Upstream compiles its mock-root test hooks out of non-test builds.
        buildConfigField("boolean", "TEST", "false")
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}
