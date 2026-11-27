package ci.not.rune.applemusic

/**
 * What the host needs of the platform player: a queue of songs that it plays in order, without a
 * gap from one to the next. [MusicKitPlayer] is MusicKit's; the JVM tests use a fake.
 *
 * Every call returns at once, and the player answers later, through its [listener], on the thread
 * the host runs on. It answers every queue change once, in the order they were made: with
 * [Listener.onItemsAdded] once the songs show in [current] and [upcoming], which takes from a
 * second to several, or with a [PlayerError.MISSING] or [PlayerError.QUEUE] error. It leaves out a
 * song it cannot find without saying so, unless that song was all it was given.
 */
internal interface QueuePlayer {
    interface Listener {
        /** Songs were added to the queue or left it. */
        fun onQueueChanged()

        /** The songs of the oldest queue change that has yet to be answered were added. */
        fun onItemsAdded()

        /** The queue item that plays changed, to none when the queue ran out or was stopped. */
        fun onCurrentItemChanged(item: QueueItem?)

        /**
         * The queue item left off at [positionMs]: at its end, or where a stop, a skip or a new
         * queue left it, -1 for the last.
         */
        fun onItemEnded(item: QueueItem, positionMs: Long)

        fun onStateChanged(state: PlayerState)

        /** The current item started or stopped buffering. */
        fun onBufferingChanged()

        fun onError(error: PlayerError)
    }

    var listener: Listener?

    val state: PlayerState

    /** Where the current queue item stands, or a negative number when the player does not say. */
    val positionMs: Long

    /**
     * How far the current queue item is buffered, 0 until its first data came. MusicKit cannot
     * seek in an item before that: the seek fails as if the song had no audio.
     */
    val bufferedMs: Long

    /** The queue item that plays or would play next. */
    val current: QueueItem?

    /** The queue items after [current], in order. */
    val upcoming: List<QueueItem>

    /**
     * Replaces the queue with the song of [id] and waits there, paused. When the song cannot be
     * found, the queue stays as it was and an error says so.
     */
    fun replace(id: String)

    /** Adds the songs of [ids], which are all catalogue songs or all library songs, at the tail. */
    fun append(ids: List<String>)

    fun remove(queueId: Long)

    fun play()

    fun pause()

    /** Stops the player and lets the audio go. Its queue stays. */
    fun stop()

    fun seek(positionMs: Long)

    fun skipToNext()
}

/**
 * A song in the platform player's queue. MusicKit makes [queueId] of a generation that each queue
 * change counts up, in the upper 32 bits, and the place within it in the lower ones, and keeps it
 * for as long as the item stays.
 */
internal data class QueueItem(val queueId: Long, val storeId: String, val durationMs: Long) {
    val generation: Long get() = queueId ushr 32
    val slot: Long get() = queueId and 0xffffffffL
}

internal enum class PlayerState { STOPPED, PAUSED, PLAYING }

/** What went wrong in the platform player, as far as the host tells it apart. */
internal enum class PlayerError {
    /** The oldest queue change that has yet to be answered failed: the service has no such song. */
    MISSING,

    /** The oldest queue change that has yet to be answered failed for another reason. */
    QUEUE,

    /** The user's token is no longer accepted. */
    TOKEN,

    /** The account has no subscription any more. */
    SUBSCRIPTION,

    /** The account may not play the song, as an explicit one. */
    RESTRICTED,

    /** A network or read error, which the player tries again by itself. */
    IO,

    /**
     * The song's license or lease failed, as after a long pause, when the key it was loaded with
     * ran out. A new queue gets a fresh one.
     */
    LICENSE,

    /** Anything else, such as the account playing on another device. */
    OTHER,
}

/** The prefix of a catalogue song's id. Every other id is a library song's, `i.` and more. */
private const val CATALOG_PREFIX = "catalog:"

/** Whether [id] names a song of the catalogue rather than of the user's library. */
internal fun isCatalogId(id: String) = id.startsWith(CATALOG_PREFIX)

/** The id the platform player knows the song of [id] by. */
internal fun storeId(id: String) = id.removePrefix(CATALOG_PREFIX)
