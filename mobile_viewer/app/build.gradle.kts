import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release imzosi: `mobile_viewer/keystore.properties` (gitga qo'shilmaydi; `make mobileKeystore` yaratadi)
// yoki muhit o'zgaruvchilari (CI uchun): RD_KEYSTORE, RD_KEYSTORE_PASSWORD, RD_KEY_ALIAS, RD_KEY_PASSWORD.
val keystoreProps = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

fun signing(key: String, env: String): String? = keystoreProps.getProperty(key) ?: System.getenv(env)

val releaseStore = signing("storeFile", "RD_KEYSTORE")

android {
    namespace = "com.remotedesktop.mobile"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.remotedesktop.mobile"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    signingConfigs {
        if (releaseStore != null) {
            create("release") {
                storeFile = rootProject.file(releaseStore)
                storePassword = signing("storePassword", "RD_KEYSTORE_PASSWORD")
                keyAlias = signing("keyAlias", "RD_KEY_ALIAS")
                keyPassword = signing("keyPassword", "RD_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Kalit bo'lmasa imzosiz APK chiqadi (`app-release-unsigned.apk`); `make mobileRelease` buni oldindan tekshiradi
            signingConfig = signingConfigs.findByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // Rust .so fayllari `cargo ndk -o app/src/main/jniLibs ...` bilan shu yerga yoziladi
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
}
