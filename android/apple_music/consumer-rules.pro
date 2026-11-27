# MusicKit for Android ships minified already, with rules of its own in the AAR. Its native code
# reaches its classes through JNI and JavaCPP by name, and Gson fills its models by reflection,
# none of which R8 can see, so its classes are kept as they are.
-keep class com.apple.android.music.** { *; }
-keep class com.apple.android.sdk.authentication.** { *; }
-keep class org.bytedeco.javacpp.** { *; }

# Gson reads what a `TypeToken` subclass stands for from its generic signature, and the names of
# the SDK's model fields from their annotations. R8 drops both unless told to keep them, and the
# first song then fails as its metadata is read.
-keepattributes Signature, *Annotation*, InnerClasses, EnclosingMethod
-keep,allowobfuscation,allowshrinking class com.google.gson.reflect.TypeToken
-keep,allowobfuscation,allowshrinking class * extends com.google.gson.reflect.TypeToken
-keep class * extends com.google.gson.TypeAdapter
-keep class * implements com.google.gson.TypeAdapterFactory
-keep class * implements com.google.gson.JsonSerializer
-keep class * implements com.google.gson.JsonDeserializer

# Classes the SDK refers to but never loads on Android: JavaCPP's build tools and its JMX bean,
# and the data binding mapper of a library with no layouts.
-dontwarn org.apache.maven.**
-dontwarn org.slf4j.**
-dontwarn org.osgi.**
-dontwarn java.lang.management.**
-dontwarn javax.management.**
-dontwarn androidx.databinding.**
