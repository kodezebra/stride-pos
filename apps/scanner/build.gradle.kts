// The Android companion. It is a separate build from the Rust workspace on
// purpose: a phone app has its own toolchain, and the till does not need Gradle
// to build or run.
plugins {
    id("com.android.application") version "8.13.1" apply false
    id("org.jetbrains.kotlin.android") version "2.4.0" apply false
}
