plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
    // Renders Compose screens to PNG on the JVM (./gradlew recordPaparazziDebug).
    id("app.cash.paparazzi")
}

android {
    namespace = "io.github.mdhaarith.airbridge"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.mdhaarith.airbridge"
        minSdk = 29
        targetSdk = 35
        versionCode = 1005
        versionName = "0.1.5"
    }

    // A stable release key keeps updates installable and lets Play Protect recognise the app.
    // Locally, without AIRBRIDGE_KEYSTORE set, release builds fall back to the debug key.
    val keystore = System.getenv("AIRBRIDGE_KEYSTORE")?.let { file(it) }
    signingConfigs {
        if (keystore != null) create("release") {
            storeFile = keystore
            storePassword = System.getenv("AIRBRIDGE_KEYSTORE_PASSWORD")
            keyAlias = "airbridge"
            keyPassword = System.getenv("AIRBRIDGE_KEYSTORE_PASSWORD")
        }
    }

    sourceSets["main"].assets.srcDir("../../linux/ui")

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName(if (keystore != null) "release" else "debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    testOptions {
        // android.util.Log & co. become no-ops, so the networking core runs on a plain JVM.
        unitTests.isReturnDefaultValues = true
        unitTests.all { test ->
            test.testLogging { events("passed", "failed", "skipped"); showStandardStreams = false; exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL }
        }
    }
    packaging {
        resources.excludes += setOf("META-INF/versions/9/OSGI-INF/MANIFEST.MF", "META-INF/{AL2.0,LGPL2.1}")
    }
}

dependencies {
    implementation("com.google.zxing:core:3.5.3")
    // Liquid glass (backdrop blur + lens refraction) for floating surfaces.
    implementation("io.github.kyant0:backdrop:1.0.0")
    val composeBom = platform("androidx.compose:compose-bom:2025.10.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")

    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-service:2.8.7")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.bouncycastle:bcpkix-jdk18on:1.79")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
}
