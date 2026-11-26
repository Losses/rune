# R8 cannot see classes reached only from Rust through JNI FindClass:
# ndk-saf looks up these by their original binary name, so shrinking or
# renaming them makes file access die with ClassNotFoundException.
# Keep the whole (tiny) package, names and members included.
-keep class androidx.documentfile.** { *; }

# The hub calls PlaybackService.update through JNI, by name, which R8 cannot
# see either.
-keep class ci.not.rune.PlaybackService {
    public static void update(...);
}
