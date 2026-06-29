plugins {
    kotlin("jvm") version "2.0.21"
    `maven-publish`
}

group = "sh.haven"
version = "0.1.0"

repositories {
    mavenCentral()
    google()
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.1")
    // Keep version in sync with [versions.jna] in gradle/libs.versions.toml
    compileOnly("net.java.dev.jna:jna:5.14.0")

    testImplementation("junit:junit:4.13.2")
}

// Generated Kotlin sources live under kotlin/ (see tools/build-android.sh)
sourceSets {
    main {
        kotlin.srcDir("kotlin")
    }
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// Build the SPICE native library from Rust source via cargo-ndk.
// Prerequisites: rustup, cargo-ndk, aarch64-linux-android + x86_64-linux-android targets.
// The prebuilt .so files under jniLibs/ ARE committed (CI consumes them without
// the Rust toolchain); re-run tools/build-android.sh after changing the Rust.
val buildSpiceNative by tasks.registering(Exec::class) {
    val rustDir = file("rust")
    val jniDir = file("jniLibs")

    inputs.dir(rustDir.resolve("src"))
    // The decoder lives in the vendored crate; without this a decoder change
    // leaves the task UP-TO-DATE and ships a stale .so (#286).
    inputs.dir(rustDir.resolve("vendor"))
    inputs.file(rustDir.resolve("Cargo.toml"))
    inputs.file(rustDir.resolve("Cargo.lock"))
    outputs.dir(jniDir)

    workingDir = rustDir

    // Detect NDK from ANDROID_NDK_HOME or ANDROID_SDK_ROOT
    val ndkHome = System.getenv("ANDROID_NDK_HOME")
        ?: System.getenv("ANDROID_SDK_ROOT")?.let { sdk ->
            file("$sdk/ndk").listFiles()?.maxByOrNull { it.name }?.absolutePath
        }
    if (ndkHome != null) {
        environment("ANDROID_NDK_HOME", ndkHome)
    }

    commandLine("cargo", "ndk",
        "-o", jniDir.absolutePath,
        "-t", "arm64-v8a",
        "-t", "x86_64",
        "build", "--release")
}

// No publishing block needed — consumed via includeBuild() in settings.gradle.kts
