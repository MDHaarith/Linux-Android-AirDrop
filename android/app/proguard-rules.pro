# kotlinx.serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**
-keepclassmembers class kotlinx.serialization.json.** { *** Companion; }
-keep,includedescriptorclasses class io.github.mdhaarith.airbridge.**$$serializer { *; }
-keepclassmembers class io.github.mdhaarith.airbridge.** { *** Companion; }
-keepclasseswithmembers class io.github.mdhaarith.airbridge.** { kotlinx.serialization.KSerializer serializer(...); }

# BouncyCastle is only used for certificate generation.
-dontwarn org.bouncycastle.**
-keep class org.bouncycastle.jcajce.provider.** { *; }
# OkHttp
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.openjsse.**
