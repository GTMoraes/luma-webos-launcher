import java.io.File

plugins {
    id("com.android.application")
}

// One version for webOS and Android: appinfo.json at the repository root.
val appinfo = rootProject.file("../appinfo.json").readText()
val appVersion = Regex("\"version\"\\s*:\\s*\"([0-9.]+)\"").find(appinfo)?.groupValues?.get(1) ?: "0.0.0"
// Each build must have a higher versionCode to install over the last one.
val buildNumber = (System.getenv("GITHUB_RUN_NUMBER") ?: "1").toInt()

// Release key from the repository secrets (android/README.md). Without it the
// runner's debug key signs, which changes on every run.
val key = System.getenv("MYS_KEYSTORE")?.let { File(it) }?.takeIf { it.exists() }

android {
    namespace = "com.frx9.luma"
    compileSdk = 35
    // The NDK that built the native libraries (android/build-native.sh), if set.
    System.getenv("LUMA_NDK_VERSION")?.let { ndkVersion = it }

    defaultConfig {
        applicationId = "com.frx9.luma"
        // The Chromecast with Google TV runs Android TV 14 on 32-bit ARM.
        minSdk = 26
        targetSdk = 35
        versionCode = buildNumber
        versionName = appVersion
        ndk {
            abiFilters += listOf("armeabi-v7a")
        }
    }

    signingConfigs {
        if (key != null) {
            create("release-key") {
                storeFile = key
                storePassword = System.getenv("MYS_KEYSTORE_PASS")
                keyAlias = System.getenv("MYS_KEY_ALIAS")
                keyPassword = System.getenv("MYS_KEY_PASS")
            }
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = if (key != null) signingConfigs.getByName("release-key")
                            else signingConfigs.getByName("debug")
        }
    }

    // SDLActivity opens libluma.so by its path in the app's native library folder.
    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        checkReleaseBuilds = false
        abortOnError = false
    }
}
