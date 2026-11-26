package ci.not.rune.applemusic

/**
 * The hosted player's contract (the doc comment of `playback/src/hosted.rs`) on top of a
 * [QueuePlayer].
 *
 * The host's queue holds the entries the player gave since its latest `set`, and indexes count
 * from 0 there. The platform player's queue gets them in order: the entry that becomes current
 * in a queue of its own, which tells at once whether the service has its song, and the entries
 * after it by queue changes at the tail, one run of catalogue or library songs at a time, each
 * once the one before has shown in the queue. An entry knows its platform queue item by the item's
 * id. The platform player leaves out a song that it cannot find, so the host compares what came
 * with what it asked for: an entry that did not come fails, and nothing after it stays in the
 * platform queue, so that the platform player never moves on past an entry that cannot be played.
 *
 * The platform queue item that plays changing to a later entry's is the host moving on, at once
 * reported. An entry that plays to its end with no queue item after it is the end of the queue,
 * or, when the next entry has not reached the platform queue yet, the host moving on by giving it
 * a queue of its own. Entries that played stay in the platform queue: the platform player has no
 * controls of its own that could take it back to them, and once the queue ran out it waits on its
 * first item, which the host never plays again.
 *
 * The platform player gives up on a song it fails to load: it leaves it short of its end and
 * moves on without a word. The host does not follow it past an entry that did not play through:
 * the entry plays on from where it was left in a queue of its own, once, and fails the second
 * time. A seek waits for the first data of the song, as the platform player cannot seek before.
 *
 * Everything happens on one thread: batches, the platform player's answers, and [tick], which the
 * host calls every [TICK_MS] while [wantsTicks]. Nothing here waits. Every event carries the
 * revision of the latest batch.
 */
internal class Jukebox(
    private val player: QueuePlayer,
    private val clock: () -> Long,
    /** Whether the device is online, which tells a song the service lacks from one out of reach. */
    private val online: () -> Boolean,
    /** Why nothing can be played now, or null when the host can play. */
    private val unready: () -> Failure?,
    private val send: (HostEvent) -> Unit,
    /** The user's access was refused while playing. Called last, after the failure is sent. */
    private val accessLost: () -> Unit,
    private val log: (String) -> Unit,
) : QueuePlayer.Listener {
    companion object {
        /** How often [tick] wants to run. */
        const val TICK_MS = 500L

        /** How often a status goes out while an entry plays. */
        const val REPORT_EVERY_MS = 2_000L

        /**
         * How long a queue change may go unanswered before its entries count as not taken. The
         * platform player takes up to 10 s to say that it cannot find a song.
         */
        const val CHANGE_TIMEOUT_MS = 20_000L

        /** How many unanswered queue changes are remembered at most. */
        private const val UNANSWERED_LIMIT = 32

        /**
         * How long an entry told to play may make no progress before it fails. The player reports
         * a song as playing while it buffers, which takes seconds on a slow network, and the
         * host keeps reporting it as playing meanwhile, and as buffering.
         */
        const val PROGRESS_LIMIT_MS = 30_000L

        /** How far the position has to move for an entry to count as heard playing. */
        const val PROGRESS_STEP_MS = 100L

        /** How long an entry that plays may stand still before it counts as buffering again. */
        const val STALL_AFTER_MS = 2_000L

        /** How short of its end an entry that ends counts as played out. */
        const val END_TOLERANCE_MS = 1_000L

        /** How long an entry that played out waits for the platform player to move on or stop. */
        const val PLAYED_OUT_WAIT_MS = 3_000L

        /**
         * A tick that comes this much later than it should means that the process did not run
         * meanwhile, and no deadline counts that time. The player takes the same threshold.
         */
        const val FROZEN_GAP_MS = 5_000L
    }

    private class Entry(val id: String) {
        /** The platform queue item that plays the entry, once it showed in the queue. */
        var queueId: Long? = null
        var durationMs = 0L

        /** Whether the entry went to the platform player, which may not have answered yet. */
        var sent = false
        var failure: Failure? = null

        /** Whether the entry played on once after the platform player left it short of its end. */
        var resumed = false
    }

    /** A queue change that has yet to show: a new queue for [entries]' only one, or an append. */
    private class Change(val entries: List<Entry>, val replaces: Boolean, var deadline: Long)

    private var revision = 0L
    private val entries = mutableListOf<Entry>()
    private var current: Int? = null

    /** Whether the last entry finished with nothing playable after it. */
    private var ended = false

    /** Whether Rune wants the current entry to play. */
    private var wantPlaying = false

    /** The current entry is in a new platform queue and is not current there yet. */
    private var loading = false

    /** Where the current entry starts once it is loaded. */
    private var pendingOffset = 0L

    /**
     * A seek in the current entry that waits for its first data, before which the platform player
     * cannot seek. The entry plays once the seek is made, so that it never starts from 0 first.
     */
    private var waitingSeek: Long? = null

    private var change: Change? = null

    /**
     * The queue changes made that the platform player has yet to answer, oldest first. It answers
     * each once and in order, so an answer belongs to the oldest of them: one of a queue that was
     * replaced since, or the change under way.
     */
    private val unanswered = ArrayDeque<Change>()

    /** The queue generations that came before the change that is under way. */
    private val knownGenerations = mutableSetOf<Long>()

    /**
     * The generation of the latest new queue. Generations count up in the order of the calls
     * that made them, so one below it is left from a change that this queue replaced.
     */
    private var floor = 0L

    /** The current entry played to its end, and the platform player moves on or stops. */
    private var playedOut = false
    private var playedOutAt = 0L

    /**
     * Where the platform player left the current entry short of its end without being told to,
     * as it does with a song it fails to load, and moves on or stops.
     */
    private var leftEarlyAt: Long? = null
    private var leftEarlyWhen = 0L

    /** Whether the current entry was heard playing since it was last told to play or moved. */
    private var heard = true

    /** When the current entry last moved on, or was told to play or to move. */
    private var progressAt = 0L
    private var lastPosition = 0L

    /** When Rune last paused, as the platform player may still say that it plays just after. */
    private var stoppedAt = 0L
    private var lastTick: Long? = null
    private var accessLostReported = false

    private var reportedSignature = ""
    private var reportedAt = 0L

    init {
        player.listener = this
    }

    /** Whether [tick] should run: while the host holds entries. */
    val wantsTicks: Boolean get() = entries.isNotEmpty()

    private val currentEntry: Entry? get() = current?.let { entries.getOrNull(it) }

    private fun indexOf(entry: Entry) = entries.indexOfFirst { it === entry }

    // Batches

    /** Applies a batch in order, and reports a status once it is applied. */
    fun apply(batch: HostBatch) {
        revision = batch.revision
        val endedBefore = ended
        var picked = false
        for (action in batch.actions) {
            when (action) {
                is HostAction.Set -> {
                    picked = true
                    set(action.ids)
                }
                is HostAction.Add -> add(action.ids)
                is HostAction.Remove -> remove(action.index)
                HostAction.Clear -> {
                    picked = true
                    drop()
                }
                is HostAction.Skip -> skip(action.index, action.offsetMs)
                HostAction.Start -> start()
                HostAction.Stop -> stop()
                // MusicKit for Android has no volume of its own: the system volume applies.
                is HostAction.SetGain, HostAction.Unknown -> Unit
            }
        }
        // A batch that comes after the queue ended still needs its answer at this revision.
        if (endedBefore && ended && !picked) sendEnded()
        sendStatus()
    }

    /** Stops the platform player and forgets every entry, as a host that starts holds nothing. */
    fun drop() {
        val held = entries.isNotEmpty()
        entries.clear()
        current = null
        resetPlayback()
        // A host that held nothing leaves the platform player alone, which may not even exist.
        if (held) player.stop()
    }

    private fun resetPlayback() {
        ended = false
        wantPlaying = false
        loading = false
        pendingOffset = 0
        waitingSeek = null
        change = null
        playedOut = false
        leftEarlyAt = null
        heard = true
        lastPosition = 0
        lastTick = null
        accessLostReported = false
    }

    private fun set(ids: List<String>) {
        entries.clear()
        entries += ids.map(::Entry)
        resetPlayback()
        current = if (entries.isEmpty()) null else 0
        val first = entries.firstOrNull() ?: return player.stop()
        unready()?.let { return fail(first, it) }
        replaceFrom(0, 0)
    }

    /** Appends entries. After the end nothing goes to the platform player until the next `set`. */
    private fun add(ids: List<String>) {
        entries += ids.map(::Entry)
        feed()
    }

    /** Removes an entry after the current one. One aimed at the current entry is ignored. */
    private fun remove(index: Int) {
        val current = current ?: return
        if (index <= current || index !in entries.indices) return
        val entry = entries.removeAt(index)
        entry.queueId?.let(player::remove)
        // An entry that failed and held the others back may be the one that left.
        feed()
    }

    /** Within the current entry this is a seek. To the next one it moves on, as before. */
    private fun skip(index: Int, offsetMs: Long) {
        val current = current ?: return
        if (ended || index !in entries.indices) return
        when (index) {
            current -> seek(offsetMs)
            current + 1 -> moveOn(index, offsetMs)
            else -> log("Ignoring a skip from entry $current to $index")
        }
    }

    private fun seek(offsetMs: Long) {
        val entry = currentEntry ?: return
        if (entry.failure != null) return
        when {
            loading -> pendingOffset = offsetMs
            // An entry that played out goes back to the platform player first.
            playedOut -> return replaceFrom(current!!, offsetMs)
            hasData(entry) -> {
                player.seek(offsetMs)
                // A seek that waited is overtaken, and what it held back plays.
                if (waitingSeek != null && wantPlaying) player.play()
                waitingSeek = null
            }
            else -> {
                if (player.state == PlayerState.PLAYING) player.pause()
                waitingSeek = offsetMs
            }
        }
        lastPosition = offsetMs
        watchProgress()
    }

    /** Whether the platform player has the first data of [entry], which plays now. */
    private fun hasData(entry: Entry) =
        player.current?.queueId == entry.queueId && player.bufferedMs > 0

    /** Makes the seek that waits for the current entry's first data, once that came. */
    private fun seekOnceBuffered() {
        val offset = waitingSeek ?: return
        val entry = currentEntry ?: return
        if (loading || entry.failure != null || !hasData(entry)) return
        waitingSeek = null
        player.seek(offset)
        lastPosition = offset
        watchProgress()
        if (wantPlaying) player.play()
    }

    private fun moveOn(index: Int, offsetMs: Long) {
        val playing = currentEntry ?: return
        val next = entries[index]
        val queued = next.queueId != null && playing.queueId != null && !loading && !playedOut &&
            player.current?.queueId == playing.queueId
        becomeCurrent(index, offsetMs)
        next.failure?.let {
            // The player moved on to an entry that cannot be played: nothing plays, and the
            // player hears why at this revision.
            player.pause()
            send(HostEvent.ItemFailed(revision, index, it))
            return
        }
        if (!queued) return replaceFrom(index, offsetMs)
        player.skipToNext()
        // A seek right after the skip applies to the new item.
        if (offsetMs > 0) player.seek(offsetMs)
        if (wantPlaying) player.play() else player.pause()
    }

    private fun start() {
        wantPlaying = true
        val entry = currentEntry ?: return
        // An entry that played out is not started again: the host moves on from it.
        if (ended || playedOut || entry.failure != null) return
        watchProgress()
        if (!loading && waitingSeek == null && player.state != PlayerState.PLAYING) player.play()
    }

    private fun stop() {
        wantPlaying = false
        stoppedAt = clock()
        val entry = currentEntry ?: return
        if (!loading && !ended && entry.failure == null) player.pause()
    }

    /** Makes the entry at [index] current, from [offsetMs]. */
    private fun becomeCurrent(index: Int, offsetMs: Long) {
        current = index
        playedOut = false
        leftEarlyAt = null
        waitingSeek = null
        lastPosition = offsetMs
        watchProgress()
    }

    /** Restarts the wait for the current entry to be heard playing. */
    private fun watchProgress() {
        heard = !wantPlaying
        progressAt = clock()
    }

    // The platform queue

    /**
     * Gives the platform player a new queue with the entry at [index] alone, paused. What still
     * plays stops at once rather than once the new queue is there.
     */
    private fun replaceFrom(index: Int, offsetMs: Long) {
        entries.forEachIndexed { at, entry ->
            entry.queueId = null
            if (at >= index) entry.sent = false
        }
        val entry = entries[index]
        entry.sent = true
        knownGenerations += visibleGenerations()
        loading = true
        playedOut = false
        leftEarlyAt = null
        waitingSeek = null
        pendingOffset = offsetMs
        lastPosition = offsetMs
        if (player.state == PlayerState.PLAYING) player.pause()
        player.replace(entry.id)
        // The first call waits for the platform player to be made, which takes no time off the
        // deadlines.
        issue(Change(listOf(entry), replaces = true, deadline = clock() + CHANGE_TIMEOUT_MS))
        watchProgress()
    }

    /**
     * Sends the next run of entries after the current one to the platform player, once the queue
     * holds the current entry and no other change is under way. Nothing goes past an entry that
     * failed, nor after the end.
     */
    private fun feed() {
        val current = current ?: return
        val playing = entries[current]
        if (change != null || ended || playing.queueId == null || playing.failure != null) return
        for (index in current + 1 until entries.size) {
            val entry = entries[index]
            if (entry.failure != null) return
            if (entry.sent) continue
            val catalog = isCatalogId(entry.id)
            val run = entries.subList(index, entries.size)
                .takeWhile { !it.sent && it.failure == null && isCatalogId(it.id) == catalog }
                .toList()
            run.forEach { it.sent = true }
            knownGenerations += visibleGenerations()
            player.append(run.map { it.id })
            return issue(Change(run, replaces = false, deadline = clock() + CHANGE_TIMEOUT_MS))
        }
    }

    private fun issue(change: Change) {
        this.change = change
        unanswered.addLast(change)
        if (unanswered.size > UNANSWERED_LIMIT) unanswered.removeFirst()
    }

    private fun visibleItems(): List<QueueItem> = listOfNotNull(player.current) + player.upcoming

    private fun visibleGenerations() = visibleItems().map { it.generation }

    /** Takes the change under way as done once its items show, or once it ran out of time. */
    private fun checkChange() {
        val change = change ?: return
        if (change.replaces) {
            // A new queue shows as a current item of a new generation.
            val item = player.current
            if (item != null && item.generation !in knownGenerations && item.generation > floor) {
                floor = item.generation
                // Every generation below the floor counts as an earlier queue's anyway.
                knownGenerations.removeAll { it < floor }
                return complete(change, visibleItems().filter { it.generation == item.generation })
            }
        } else {
            val fresh = visibleItems().filter { it.generation !in knownGenerations }
            // Songs of a change that the current queue replaced, which came after all the same.
            val stale = fresh.filter { it.generation < floor }
            if (stale.isNotEmpty()) {
                log("Taking ${stale.size} queue items of an earlier queue out of the queue")
                knownGenerations += stale.map { it.generation }
                stale.forEach { player.remove(it.queueId) }
            }
            val later = fresh - stale.toSet()
            if (later.isNotEmpty()) {
                val generation = later.minOf { it.generation }
                val came = later.filter { it.generation == generation }.sortedBy { it.slot }
                knownGenerations += generation
                if (came.size == change.entries.size || match(change.entries, came).isNotEmpty()) {
                    return complete(change, came)
                }
                // Songs that none of the entries asked for.
                log("Taking ${came.size} queue items that map to no entry out of the queue")
                came.forEach { player.remove(it.queueId) }
            }
        }
        if (clock() >= change.deadline) {
            log("A queue change was not answered in time")
            complete(change, emptyList(), Failure.NETWORK)
        }
    }

    /**
     * Pairs the entries of a change with the queue items that came, in order. When some did not
     * come, the songs tell which: the player names a song by the id it was asked for.
     */
    private fun match(sent: List<Entry>, came: List<QueueItem>): Map<Entry, QueueItem> {
        if (came.size == sent.size) return sent.zip(came).toMap()
        val matched = mutableMapOf<Entry, QueueItem>()
        var next = 0
        for (item in came) {
            val at = (next until sent.size).firstOrNull { storeId(sent[it].id) == item.storeId }
                ?: continue
            matched[sent[at]] = item
            next = at + 1
        }
        return matched
    }

    /**
     * Takes the change as done with what [came]. An entry that did not come fails, for [failure]
     * when the platform player said why.
     */
    private fun complete(change: Change, came: List<QueueItem>, failure: Failure? = null) {
        this.change = null
        knownGenerations += visibleGenerations()
        val matched = match(change.entries, came)
        for (item in came) {
            if (item !in matched.values) player.remove(item.queueId)
        }
        var blocked = false
        for (entry in change.entries) {
            val item = matched[entry]
            // An entry that left meanwhile, or that comes after one that did not come, leaves the
            // platform queue too.
            if (indexOf(entry) < 0 || blocked) {
                item?.let { player.remove(it.queueId) }
                entry.sent = false
                continue
            }
            if (item == null) {
                blocked = true
                // A song left out of what came is one the service lacks.
                val reason = failure ?: if (online()) Failure.UNPLAYABLE else Failure.NETWORK
                log("The platform player did not take entry ${indexOf(entry)}")
                fail(entry, reason)
                continue
            }
            entry.queueId = item.queueId
            entry.durationMs = item.durationMs
        }
        settle()
        feed()
    }

    /** The current entry became current in its new queue: it starts where it should. */
    private fun settle() {
        if (!loading) return
        val entry = currentEntry ?: return
        val queueId = entry.queueId ?: return
        if (player.current?.queueId != queueId) return
        loading = false
        lastPosition = pendingOffset
        if (pendingOffset > 0) {
            waitingSeek = pendingOffset
            watchProgress()
            seekOnceBuffered()
        } else if (wantPlaying) {
            watchProgress()
            player.play()
        }
        pendingOffset = 0
        feed()
    }

    // Answers of the platform player

    override fun onQueueChanged() {
        checkChange()
        settle()
        report(periodic = false)
    }

    override fun onBufferingChanged() {
        seekOnceBuffered()
        report(periodic = false)
    }

    override fun onItemsAdded() {
        unanswered.removeFirstOrNull()
        checkChange()
        settle()
        report(periodic = false)
    }

    override fun onCurrentItemChanged(item: QueueItem?) {
        checkChange()
        settle()
        follow(item)
        report(periodic = false)
    }

    override fun onItemEnded(item: QueueItem, positionMs: Long) {
        val entry = currentEntry ?: return
        if (loading || ended || entry.queueId != item.queueId) return
        if (entry.failure != null || playedOut) return
        val duration = if (item.durationMs > 0) item.durationMs else entry.durationMs
        if (duration > 0 && positionMs >= duration - END_TOLERANCE_MS) {
            playedOut = true
            playedOutAt = clock()
            lastPosition = maxOf(positionMs, duration)
            return
        }
        // The host's own skips and new queues make another entry current first, and a stop
        // drops the entries, so this item was left without the host telling it to.
        leftEarlyAt = maxOf(positionMs, 0)
        leftEarlyWhen = clock()
    }

    override fun onStateChanged(state: PlayerState) {
        // An item that played out with nothing after it stops the player.
        if (state == PlayerState.STOPPED && playedOut) endOrMoveOn()
        if (state == PlayerState.STOPPED && leftEarlyAt != null) playOnAfterEarlyLeave()
        report(periodic = false)
    }

    override fun onError(error: PlayerError) {
        log("The platform player failed: $error")
        failFor(error)
        report(periodic = false)
    }

    private fun failFor(error: PlayerError) {
        when (error) {
            PlayerError.MISSING, PlayerError.QUEUE -> {
                // The answer to the oldest change that has yet to be answered. A change of a queue
                // replaced since, or one that showed already, is over.
                val answered = unanswered.removeFirstOrNull()
                checkChange()
                val change = change
                if (answered == null || answered !== change) return
                val missing = error == PlayerError.MISSING && online()
                complete(change, emptyList(), if (missing) Failure.UNPLAYABLE else Failure.NETWORK)
            }
            PlayerError.TOKEN, PlayerError.SUBSCRIPTION -> {
                if (accessLostReported) return
                accessLostReported = true
                currentEntry?.let { fail(it, Failure.NOT_AUTHORIZED) }
                accessLost()
            }
            PlayerError.RESTRICTED -> failPlaying(Failure.NOT_AUTHORIZED)
            // The platform player tries again by itself while it still plays.
            PlayerError.IO -> if (player.state != PlayerState.PLAYING) failPlaying(Failure.NETWORK)
            PlayerError.OTHER -> failPlaying(Failure.NETWORK)
        }
    }

    /**
     * Fails the current entry for an error of what plays. Until the entry is loaded, the error
     * is about what played before.
     */
    private fun failPlaying(reason: Failure) {
        if (loading) return
        currentEntry?.let { fail(it, reason) }
    }

    /** Follows the platform player when the item that plays changed. */
    private fun follow(item: QueueItem?) {
        val current = current ?: return
        val entry = entries[current]
        if (ended || loading || entry.failure != null) return
        // The platform player gave up on the entry and moved on: the entry is not played through.
        if (leftEarlyAt != null && item?.queueId != entry.queueId) return playOnAfterEarlyLeave()
        if (item == null) {
            if (playedOut) endOrMoveOn()
            return
        }
        val index = entries.indexOfFirst { it.queueId == item.queueId }
        when {
            index < 0 || index == current -> Unit
            // The host moved on by itself, at the end of an entry.
            index > current -> {
                becomeCurrent(index, 0)
                sendStatus()
            }
            // The platform queue ran out and went back to its start.
            playedOut || lastPosition >= entry.durationMs - END_TOLERANCE_MS -> endOrMoveOn()
        }
    }

    /**
     * The current entry played out and the platform player did not move on: the queue ended, or
     * the next entry has yet to reach the platform queue, and the host moves on by itself.
     */
    private fun endOrMoveOn() {
        val current = current ?: return
        if (ended) return
        val next = entries.getOrNull(current + 1)
        if (next == null || next.failure != null) return finish()
        becomeCurrent(current + 1, 0)
        replaceFrom(current + 1, 0)
        sendStatus()
    }

    /**
     * The platform player left the current entry short of its end without being told to, and
     * moved on or stopped. The entry plays on from there in a queue of its own, once; when that
     * happens again, it fails.
     */
    private fun playOnAfterEarlyLeave() {
        val current = current ?: return
        val entry = entries[current]
        val at = leftEarlyAt ?: return
        leftEarlyAt = null
        if (entry.resumed) {
            log("The platform player left entry $current short of its end again")
            return fail(entry, Failure.NETWORK)
        }
        log("The platform player left entry $current at $at ms, short of its end; it plays on")
        entry.resumed = true
        replaceFrom(current, at)
        report(periodic = false)
    }

    /** The last entry finished, and nothing playable follows it. */
    private fun finish() {
        ended = true
        playedOut = false
        lastPosition = maxOf(lastPosition, currentEntry?.durationMs ?: 0)
        sendEnded()
        sendStatus()
    }

    // Failures and reports

    /**
     * Reports that an entry cannot be played, once. Nothing after it stays in the platform queue.
     */
    private fun fail(entry: Entry, reason: Failure) {
        val index = indexOf(entry)
        if (entry.failure != null || index < 0) return
        entry.failure = reason
        send(HostEvent.ItemFailed(revision, index, reason))
        val current = current
        if (index == current) {
            loading = false
            playedOut = false
            leftEarlyAt = null
            waitingSeek = null
            player.pause()
        }
        for (later in entries.subList(index, entries.size)) {
            if (indexOf(later) == current) continue
            later.queueId?.let(player::remove)
            later.queueId = null
        }
    }

    /** Runs the deadlines and the periodic status. */
    fun tick() {
        noteFreeze()
        checkChange()
        settle()
        seekOnceBuffered()
        observe()
        val entry = currentEntry
        if (entry != null && playedOut && clock() - playedOutAt >= PLAYED_OUT_WAIT_MS) {
            log("The platform player neither moved on nor stopped after an entry played out")
            endOrMoveOn()
        }
        if (entry != null && leftEarlyAt != null && clock() - leftEarlyWhen >= PLAYED_OUT_WAIT_MS) {
            playOnAfterEarlyLeave()
        }
        if (entry != null && wantPlaying && !ended && !playedOut && entry.failure == null &&
            (loading || !heard || player.state == PlayerState.PLAYING) &&
            clock() - progressAt >= PROGRESS_LIMIT_MS
        ) {
            log("Entry $current made no progress in time")
            fail(entry, Failure.NETWORK)
        }
        report(periodic = true)
    }

    /** A time that the process did not run moves every deadline on. */
    private fun noteFreeze() {
        val now = clock()
        val last = lastTick
        lastTick = now
        val gap = if (last == null) 0 else now - last - TICK_MS
        if (gap <= FROZEN_GAP_MS) return
        log("The host did not run for $gap ms")
        progressAt += gap
        playedOutAt += gap
        leftEarlyWhen += gap
        change?.let { it.deadline += gap }
    }

    /** Notes the current entry's progress, and a resume from outside Rune. */
    private fun observe() {
        val entry = currentEntry ?: return
        if (loading || waitingSeek != null || ended || playedOut || entry.failure != null) return
        if (player.current?.queueId != entry.queueId) return
        val position = player.positionMs
        if (position < 0) return
        if (player.state == PlayerState.PLAYING) {
            if (!wantPlaying && clock() - stoppedAt > 1_000) wantPlaying = true
            if (position >= lastPosition + PROGRESS_STEP_MS) {
                heard = true
                progressAt = clock()
            }
        }
        lastPosition = position
    }

    /** Whether the current entry plays, as Rune should see it: one that starts or moves on does. */
    private fun reportsPlaying(): Boolean {
        val entry = currentEntry ?: return false
        if (ended || entry.failure != null || !wantPlaying) return false
        return loading || waitingSeek != null || !heard || playedOut ||
            player.state == PlayerState.PLAYING
    }

    /**
     * Whether the current entry is told to play but does not move on yet, as while the platform
     * player loads its audio. Rune then keeps the entry where the status puts it.
     */
    private fun buffers(): Boolean {
        if (!reportsPlaying() || playedOut) return false
        return loading || waitingSeek != null || !heard || clock() - progressAt >= STALL_AFTER_MS
    }

    private fun position(): Long {
        val entry = currentEntry ?: return 0
        if (entry.failure != null) return 0
        if (loading) return pendingOffset
        waitingSeek?.let { return it }
        if (ended || playedOut) return maxOf(lastPosition, entry.durationMs)
        if (player.current?.queueId == entry.queueId) {
            val position = player.positionMs
            if (position >= 0) return position
        }
        return lastPosition
    }

    private fun signature() =
        "$current|$ended|${reportsPlaying()}|${buffers()}|${currentEntry?.failure}"

    /** Reports a change that Rune did not cause, and, while an entry plays, every few seconds. */
    private fun report(periodic: Boolean) {
        if (current == null) return
        val due = periodic && reportsPlaying() && clock() - reportedAt >= REPORT_EVERY_MS
        if (due || signature() != reportedSignature) sendStatus()
    }

    private fun sendStatus() {
        reportedSignature = signature()
        reportedAt = clock()
        val status = HostEvent.Status(
            revision,
            current,
            currentEntry?.id,
            reportsPlaying(),
            position(),
            buffers(),
        )
        send(status)
    }

    /** Reports the end at the entry that ended. */
    private fun sendEnded() {
        send(HostEvent.Ended(revision, current))
    }
}
