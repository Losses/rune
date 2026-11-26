# MusicKit for Android ships minified already, with rules of its own in the AAR. Its native code
# reaches its classes through JNI and JavaCPP by name, and Gson fills its models by reflection,
# none of which R8 can see, so its classes are kept as they are.
-keep class com.apple.android.music.** { *; }
-keep class com.apple.android.sdk.authentication.** { *; }
-keep class org.bytedeco.javacpp.** { *; }

# Classes the SDK refers to but never loads on Android: JavaCPP's build tools and its JMX bean,
# and the data binding mapper of a library with no layouts.
-dontwarn org.apache.maven.**
-dontwarn org.slf4j.**
-dontwarn org.osgi.**
-dontwarn java.lang.management.**
-dontwarn javax.management.**
-dontwarn androidx.databinding.**
