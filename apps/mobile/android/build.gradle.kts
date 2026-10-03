plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

val irohRepository = rootProject.projectDir.resolve("../../../iroh-kmp")
val mistRepository = rootProject.projectDir.resolve("../..")
val nativeOutput = layout.buildDirectory.dir("native/android")
val androidSdk = providers.environmentVariable("ANDROID_HOME")
    .orElse("${System.getProperty("user.home")}/Android/Sdk")
val androidNdk = androidSdk.map { "$it/ndk/30.0.16248370" }

val buildIrohNative by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds the Rust Iroh runtime for Android arm64"
    workingDir(irohRepository)
    environment("ANDROID_HOME", androidSdk.get())
    environment("ANDROID_NDK_HOME", androidNdk.get())
    commandLine(
        "cargo", "ndk",
        "--target", "arm64-v8a",
        "--platform", "26",
        "--output-dir", nativeOutput.get().asFile,
        "build", "--release", "--package", "iroh-kmp-native",
    )
    inputs.files(
        irohRepository.resolve("Cargo.toml"),
        irohRepository.resolve("Cargo.lock"),
    )
    inputs.dir(irohRepository.resolve("iroh-runtime/native/src"))
    outputs.file(nativeOutput.map { it.file("arm64-v8a/libiroh_kmp_native.so") })
}

val buildMistNative by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds the Rust Mist runtime for Android arm64"
    workingDir(mistRepository)
    environment("ANDROID_HOME", androidSdk.get())
    environment("ANDROID_NDK_HOME", androidNdk.get())
    commandLine(
        "cargo", "ndk",
        "--target", "arm64-v8a",
        "--platform", "26",
        "--output-dir", nativeOutput.get().asFile,
        "build", "--release", "--package", "mist-core",
    )
    inputs.files(
        mistRepository.resolve("Cargo.toml"),
        mistRepository.resolve("Cargo.lock"),
        mistRepository.resolve("core/Cargo.toml"),
        mistRepository.resolve("core/build.rs"),
    )
    inputs.dir(mistRepository.resolve("core/proto"))
    inputs.dir(mistRepository.resolve("core/src"))
    outputs.file(nativeOutput.map { it.file("arm64-v8a/libmist_core.so") })
}

android {
    namespace = "io.github.vafu.mist.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.vafu.mist"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes.getByName("release") {
        proguardFiles(
            getDefaultProguardFile("proguard-android-optimize.txt"),
            "proguard-rules.pro",
        )
    }

    buildFeatures.compose = true
    sourceSets.getByName("main").jniLibs.directories.add(nativeOutput.get().asFile.absolutePath)
}

tasks.named("preBuild").configure { dependsOn(buildIrohNative, buildMistNative) }

dependencies {
    implementation(project(":"))
    implementation(libs.androidx.activity.compose)
}
