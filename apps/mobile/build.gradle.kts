import org.jetbrains.kotlin.gradle.tasks.KotlinCompilationTask

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kotlin.multiplatform.library)
    alias(libs.plugins.compose.multiplatform)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.kotlin.atomicfu)
}

allprojects {
    group = "io.github.vafu.mist"
    version = "0.1.0-SNAPSHOT"
}

val repositoryRoot = layout.projectDirectory.dir("../..")
val coreDirectory = repositoryRoot.dir("core")
val uniffiConfig = layout.projectDirectory.file("uniffi.toml")
val bindingsOutput = layout.buildDirectory.dir("generated/mist-uniffi")
val bindgenRoot = layout.buildDirectory.dir("tools/gobley-uniffi-bindgen")
val bindgen = bindgenRoot.map { it.file("bin/gobley-uniffi-bindgen") }
val hostLibraryName = when {
    System.getProperty("os.name").startsWith("Mac", ignoreCase = true) -> "libmist_core.dylib"
    System.getProperty("os.name").startsWith("Windows", ignoreCase = true) -> "mist_core.dll"
    else -> "libmist_core.so"
}
val hostLibrary = repositoryRoot.file("target/debug/$hostLibraryName")

val installGobleyBindgen by tasks.registering(Exec::class) {
    group = "uniffi"
    description = "Installs the pinned Gobley UniFFI generator"
    commandLine(
        "cargo", "install", "--locked",
        "--version", libs.versions.gobley.get(),
        "--root", bindgenRoot.get().asFile.absolutePath,
        "gobley-uniffi-bindgen",
    )
    outputs.file(bindgen)
}

val buildMistHostLibrary by tasks.registering(Exec::class) {
    group = "uniffi"
    description = "Builds Mist metadata for binding generation"
    workingDir(repositoryRoot)
    commandLine("cargo", "build", "--package", "mist-core")
    inputs.files(
        repositoryRoot.file("Cargo.toml"),
        repositoryRoot.file("Cargo.lock"),
        coreDirectory.file("Cargo.toml"),
        coreDirectory.file("build.rs"),
        uniffiConfig,
    )
    inputs.dir(coreDirectory.dir("proto"))
    inputs.dir(coreDirectory.dir("src"))
    outputs.file(hostLibrary)
}

val generateMistBindings by tasks.registering(Exec::class) {
    group = "uniffi"
    description = "Generates Kotlin Multiplatform bindings for Mist"
    dependsOn(installGobleyBindgen, buildMistHostLibrary)
    workingDir(repositoryRoot)
    commandLine(
        bindgen.get().asFile.absolutePath,
        "--library",
        "--crate", "mist_core",
        "--config", uniffiConfig.asFile.absolutePath,
        "--out-dir", bindingsOutput.get().asFile.absolutePath,
        hostLibrary.asFile.absolutePath,
    )
    inputs.file(bindgen)
    inputs.file(hostLibrary)
    inputs.file(uniffiConfig)
    outputs.dir(bindingsOutput)
}

tasks.withType<KotlinCompilationTask<*>>().configureEach {
    dependsOn(generateMistBindings)
}

kotlin {
    explicitApi()

    android {
        namespace = "io.github.vafu.mist.mobile"
        compileSdk = 37
        minSdk = 26

        withHostTest {}
    }

    sourceSets {
        commonMain {
            kotlin.srcDir(bindingsOutput.map { it.dir("commonMain/kotlin") })
        }
        commonMain.dependencies {
            implementation(libs.compose.foundation)
            implementation(libs.compose.material3)
            implementation(libs.compose.runtime)
            implementation(libs.compose.ui)
            implementation(libs.kotlinx.coroutines.core)
            implementation(libs.kotlinx.atomicfu)
        }
        androidMain {
            kotlin.srcDir(bindingsOutput.map { it.dir("androidMain/kotlin") })
        }
        androidMain.dependencies {
            implementation(libs.google.code.scanner)
            implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
        }
    }
}
