plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

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

    buildTypes {
        release {
            isMinifyEnabled = false
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
