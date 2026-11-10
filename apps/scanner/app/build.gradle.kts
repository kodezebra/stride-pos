import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release builds carry the version the till release was tagged with. Gradle
// supplies the defaults for a local build, so `assembleDebug` works with no
// environment at all.
val appVersion = System.getenv("STRIDE_SCAN_VERSION")?.removePrefix("v") ?: "0.1.0"
val appVersionCode = System.getenv("STRIDE_SCAN_VERSION_CODE")?.toIntOrNull() ?: 1

// A release that can be sideloaded without a secret store. CI replaces this
// with a real keystore when the shop has one; until then the debug key keeps
// the APK installable, and the README says so. Empty environment variables are
// treated as absent: GitHub Actions sets a blank value for an unset secret.
val releaseKeystore = System.getenv("STRIDE_SCAN_KEYSTORE")?.takeIf { it.isNotBlank() }
val releaseStorePassword = System.getenv("STRIDE_SCAN_STORE_PASSWORD")?.takeIf { it.isNotBlank() }
val releaseKeyAlias = System.getenv("STRIDE_SCAN_KEY_ALIAS")?.takeIf { it.isNotBlank() }
val releaseKeyPassword = System.getenv("STRIDE_SCAN_KEY_PASSWORD")?.takeIf { it.isNotBlank() }
val hasReleaseSigning =
    listOf(releaseKeystore, releaseStorePassword, releaseKeyAlias, releaseKeyPassword)
        .all { it != null }

android {
    namespace = "com.stride.scan"
    compileSdk = 36
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "com.stride.scan"
        minSdk = 24
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersion
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(releaseKeystore!!)
                storePassword = releaseStorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            isShrinkResources = false
            signingConfig =
                signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        viewBinding = false
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.activity:activity-ktx:1.8.1")

    // ML Kit pulls in play-services-basement, which drags fragment 1.1.0 along.
    // registerForActivityResult() needs fragment 1.3.0 or newer (lint fails the
    // build otherwise), so the version is pinned here instead of being left to
    // a 2019 transitive.
    implementation("androidx.fragment:fragment:1.7.1")

    // CameraX preview and analysis. The version is pinned: a phone app that
    // builds differently every day is not something a shop can support.
    implementation("androidx.camera:camera-core:1.3.3")
    implementation("androidx.camera:camera-camera2:1.3.3")
    implementation("androidx.camera:camera-lifecycle:1.3.3")
    implementation("androidx.camera:camera-view:1.3.3")

    // On-device barcode decoding. Nothing leaves the phone; the model ships in
    // the APK.
    implementation("com.google.mlkit:barcode-scanning:17.2.0")

    // The money and payload logic is plain Kotlin on purpose; these tests run
    // on the JVM, with no emulator and no Android stub in the way.
    testImplementation("junit:junit:4.13.2")
}
