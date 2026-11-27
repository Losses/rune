package ci.not.rune.applemusic

/**
 * A platform player that behaves as MusicKit for Android was measured to: queue changes show
 * only once [answer] lets the service answer, in order, each with an items-added callback, each
 * change gets the next generation of queue item ids, the service leaves out songs it lacks, and a
 * change of only such songs fails with an error and leaves the queue as it was. A seek in a new
 * queue's item before its first data fails, and one between that and the start of its audio
 * leaves the item [silent]. Callbacks wait until [flush], as MusicKit posts them.
 */
internal class FakePlayer : QueuePlayer {
    private class Pending(val generation: Long, val replaces: Boolean, val ids: List<String>)

    override var listener: QueuePlayer.Listener? = null
    override var state = PlayerState.STOPPED
        private set
    override var positionMs = -1L
        private set

    /** Store ids of songs the service does not have. */
    val missing = mutableSetOf<String>()

    /** While buffering, the position stands still though the player says it plays. */
    override var buffering = false

    /** Whether a song given a queue of its own gets its first data only once [dataArrives]. */
    var slowData = false

    /**
     * Whether the current item's audio started since its queue was made: its position moved
     * while it played.
     */
    private var started = false

    /**
     * Whether the current item stands still for good while the player says that it plays it, as
     * MusicKit leaves an item that was moved between its first data and the start of its audio.
     * A new queue clears it.
     */
    var silent = false

    /** How many of the next calls to [play] MusicKit ignores, as while a song still loads. */
    var ignorePlays = 0

    /** How far the current item is buffered, 0 until its first data came. */
    private var bufferedMs = 0L

    /** What the host called, in order. */
    val calls = mutableListOf<String>()

    private var generation = 0L
    private val queue = mutableListOf<QueueItem>()
    private var index = -1
    private val pending = ArrayDeque<Pending>()
    private val callbacks = ArrayDeque<(QueuePlayer.Listener) -> Unit>()

    override val current: QueueItem? get() = queue.getOrNull(index)
    override val upcoming: List<QueueItem> get() = if (index < 0) emptyList() else queue.drop(index + 1)

    /** Every queue item there is, current or not. */
    val items: List<QueueItem> get() = queue.toList()

    private fun post(callback: (QueuePlayer.Listener) -> Unit) {
        callbacks.addLast(callback)
    }

    /** Delivers the callbacks that wait, in order. */
    fun flush() {
        while (callbacks.isNotEmpty()) listener?.let(callbacks.removeFirst())
    }

    override fun replace(id: String) {
        calls += "replace $id"
        pending.addLast(Pending(++generation, replaces = true, ids = listOf(id)))
    }

    override fun append(ids: List<String>) {
        calls += "append ${ids.joinToString(",")}"
        pending.addLast(Pending(++generation, replaces = false, ids = ids))
    }

    /** Whether a queue change waits for the service. */
    val answering: Boolean get() = pending.isNotEmpty()

    /** The service answers the oldest queue change. */
    fun answer() {
        val change = pending.removeFirst()
        val items = change.ids
            .filter { storeId(it) !in missing }
            .mapIndexed { slot, id -> QueueItem((change.generation shl 32) or slot.toLong(), storeId(id), DURATION) }
        if (items.isEmpty()) {
            post { it.onError(PlayerError.MISSING) }
            return
        }
        if (!change.replaces) {
            queue += items
            post {
                it.onItemsAdded()
                it.onQueueChanged()
            }
            return
        }
        val old = current
        val before = state
        queue.clear()
        queue += items
        index = 0
        positionMs = 0
        bufferedMs = if (slowData) 0 else 5_000
        started = false
        silent = false
        state = PlayerState.PAUSED
        post { listener ->
            listener.onItemsAdded()
            old?.let { listener.onItemEnded(it, 0) }
            listener.onCurrentItemChanged(null)
            if (before != PlayerState.PAUSED) listener.onStateChanged(PlayerState.PAUSED)
            listener.onCurrentItemChanged(items.first())
            listener.onQueueChanged()
        }
    }

    /** The first data of the current song came. */
    fun dataArrives() {
        bufferedMs = 5_000
        post { it.onBufferingChanged() }
    }

    /** The oldest queue change fails for [error], as one that a new queue overtook can. */
    fun refuse(error: PlayerError) {
        pending.removeFirst()
        post { it.onError(error) }
    }

    override fun remove(queueId: Long) {
        calls += "remove $queueId"
        val at = queue.indexOfFirst { it.queueId == queueId }
        if (at < 0 || at == index) return
        queue.removeAt(at)
        if (at < index) index--
        post { it.onQueueChanged() }
    }

    override fun play() {
        calls += "play"
        if (ignorePlays > 0) {
            ignorePlays--
            return
        }
        if (current == null || state == PlayerState.PLAYING) return
        state = PlayerState.PLAYING
        if (positionMs < 0) positionMs = 0
        post { it.onStateChanged(PlayerState.PLAYING) }
    }

    override fun pause() {
        calls += "pause"
        if (state != PlayerState.PLAYING) return
        state = PlayerState.PAUSED
        post { it.onStateChanged(PlayerState.PAUSED) }
    }

    override fun stop() {
        calls += "stop"
        val item = current ?: return
        val at = positionMs
        state = PlayerState.STOPPED
        positionMs = -1
        post {
            it.onItemEnded(item, at)
            it.onCurrentItemChanged(null)
            it.onStateChanged(PlayerState.STOPPED)
        }
    }

    override fun seek(positionMs: Long) {
        calls += "seek $positionMs"
        // MusicKit fails a seek in a song it has no data for yet.
        check(bufferedMs > 0) { "A seek before the song's first data" }
        if (current == null) return
        this.positionMs = positionMs
        if (!started) silent = true
    }

    override fun skipToNext() {
        calls += "skip"
        val item = current ?: return
        if (index + 1 >= queue.size) return
        val at = positionMs
        index++
        positionMs = 0
        val next = queue[index]
        post {
            it.onItemEnded(item, at)
            it.onCurrentItemChanged(next)
        }
    }

    /**
     * [ms] of the current item play, unless the player buffers, has no data, or the item is
     * [silent]. The first time the audio starts, the listener hears of it.
     */
    fun advance(ms: Long) {
        if (state != PlayerState.PLAYING || buffering || bufferedMs <= 0 || silent) return
        positionMs = minOf(positionMs + ms, current?.durationMs ?: 0)
        if (started) return
        started = true
        post { it.onAudioStarted() }
    }

    /**
     * The current item plays to its end. With an item after it the player moves on without a gap;
     * at the end of the queue it stops, then waits paused on the first item, its queue kept.
     */
    fun playOut() {
        val item = current ?: error("Nothing plays")
        positionMs = item.durationMs
        post { it.onItemEnded(item, item.durationMs) }
        if (index + 1 < queue.size) {
            index++
            positionMs = 0
            val next = queue[index]
            post { it.onCurrentItemChanged(next) }
            return
        }
        index = 0
        positionMs = 0
        state = PlayerState.PAUSED
        val first = queue.first()
        post {
            it.onStateChanged(PlayerState.STOPPED)
            it.onCurrentItemChanged(null)
            it.onStateChanged(PlayerState.PAUSED)
            it.onCurrentItemChanged(first)
        }
    }

    /**
     * The player gives up on the current item short of its end, as on a song it fails to load,
     * and moves on to the next item, or stops without one.
     */
    fun giveUp() {
        val item = current ?: error("Nothing plays")
        val at = positionMs
        post { it.onItemEnded(item, at) }
        if (index + 1 < queue.size) {
            index++
            positionMs = 0
            val next = queue[index]
            post { it.onCurrentItemChanged(next) }
            return
        }
        state = PlayerState.STOPPED
        positionMs = -1
        post {
            it.onStateChanged(PlayerState.STOPPED)
            it.onCurrentItemChanged(null)
        }
    }

    companion object {
        const val DURATION = 200_000L
    }
}
