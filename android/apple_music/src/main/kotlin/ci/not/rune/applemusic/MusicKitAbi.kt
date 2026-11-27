package ci.not.rune.applemusic

/** The processes MusicKit for Android can run in: its native libraries are built for ARM only. */
internal object MusicKitAbi {
    /** Whether a process whose `os.arch` is [arch] can load MusicKit's native libraries. */
    fun runs(arch: String?): Boolean =
        arch != null && (arch.startsWith("aarch64") || arch.startsWith("arm"))
}
