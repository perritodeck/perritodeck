plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "dev.perritodeck.client"
    compileSdk = 36
    // AGP 8.11's default, written out so CI, F-Droid and local builds use the same one.
    // Raise it when AGP is upgraded.
    buildToolsVersion = "35.0.0"

    defaultConfig {
        // Never change after the first public release.
        applicationId = "dev.perritodeck.client"
        minSdk = 22
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // Release signing comes from the environment (CI secrets or a local keystore). Without
    // PERRITO_KEYSTORE (unset or blank), release builds are unsigned, which is what F-Droid
    // expects, since it signs builds itself. With it, the other three must be set as well: a
    // half-set group stops the build here instead of producing a wrongly signed APK.
    fun env(name: String): String? = System.getenv(name)?.takeIf { it.isNotBlank() }
    val keystorePath = env("PERRITO_KEYSTORE")
    if (keystorePath != null) {
        val missing = listOf("PERRITO_KEYSTORE_PASSWORD", "PERRITO_KEY_ALIAS", "PERRITO_KEY_PASSWORD")
            .filter { env(it) == null }
        if (missing.isNotEmpty()) {
            val names = if (missing.size == 1) missing[0] else missing.dropLast(1).joinToString() + " and " + missing.last()
            throw GradleException(
                "PERRITO_KEYSTORE is set, but $names ${if (missing.size == 1) "is" else "are"} not. " +
                    "Set all four to sign a release, or clear PERRITO_KEYSTORE to build it unsigned.",
            )
        }
        val keystore = file(keystorePath)
        if (!keystore.isFile) {
            throw GradleException("PERRITO_KEYSTORE points at $keystore, which doesn't exist.")
        }
        signingConfigs {
            create("release") {
                storeFile = keystore
                storePassword = env("PERRITO_KEYSTORE_PASSWORD")
                keyAlias = env("PERRITO_KEY_ALIAS")
                keyPassword = env("PERRITO_KEY_PASSWORD")
                // v1 (JAR signing) is what Android 5.1 and 6 check, v2 is what 7 and newer check.
                // v3 (key rotation) stays off until it's decided; see docs/releasing.md.
                enableV1Signing = true
                enableV2Signing = true
                enableV3Signing = false
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // AGP would write the git commit into release APKs
            // (META-INF/version-control-info.textproto), so a build from a source tarball, or
            // one without .git, wouldn't match the published APK byte for byte.
            vcsInfo { include = false }
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    // F-Droid flags the encrypted dependency-info block Google adds to APKs.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    val fixturesDir = rootProject.file("../protocol/fixtures").absolutePath
    testOptions {
        unitTests.all {
            it.systemProperty("perritodeck.fixtures", fixturesDir)
        }
    }

    lint {
        abortOnError = true
        warningsAsErrors = false
    }
}

// Release APKs carry the prebuilt AndroidX native libraries (graphics-path and CameraX) exactly
// as they come. AGP strips them only when an NDK is installed: GitHub's runners have one and
// F-Droid's build server doesn't, so stripping would make the two builds differ. Debug builds
// are left as they were.
androidComponents {
    onVariants(selector().withBuildType("release")) { variant ->
        variant.packaging.jniLibs.keepDebugSymbols.add("**/*.so")
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.okhttp)
    // QR scanning: CameraX + ZXing (both Apache-2.0, no Google Play Services).
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.core)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.okhttp.mockwebserver)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    debugImplementation(libs.compose.ui.test.manifest)
}
