package ci.not.rune.applemusic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class JukeboxTest {
    private var now = 0L
    private var online = true
    private var unready: Failure? = null
    private var accessLost = 0
    private val player = FakePlayer()
    private val events = mutableListOf<HostEvent>()
    private val jukebox = Jukebox(
        player = player,
        clock = { now },
        online = { online },
        unready = { unready },
        send = { events += it },
        accessLost = { accessLost++ },
        log = {},
    )
    private var revision = 0L

    private fun batch(vararg actions: HostAction) {
        jukebox.apply(HostBatch(++revision, actions.toList()))
    }

    private fun handOver(vararg ids: String, offsetMs: Long = 0) = batch(
        HostAction.Set(ids.toList()),
        HostAction.Skip(0, offsetMs),
        HostAction.SetGain(0.5),
        HostAction.Start,
    )

    /** The service answers every queue change, one after another, as they come. */
    private fun answerAll() {
        while (player.answering) {
            player.answer()
            player.flush()
        }
    }

    /** Lets [ms] pass, in ticks, while the current item plays. */
    private fun advance(ms: Long) {
        repeat((ms / Jukebox.TICK_MS).toInt()) {
            now += Jukebox.TICK_MS
            player.advance(Jukebox.TICK_MS)
            jukebox.tick()
            player.flush()
        }
    }

    private val statuses get() = events.filterIsInstance<HostEvent.Status>()
    private val failures get() = events.filterIsInstance<HostEvent.ItemFailed>()
    private val ends get() = events.filterIsInstance<HostEvent.Ended>()

    private fun queueIdOf(storeId: String) = player.items.single { it.storeId == storeId }.queueId

    @Test
    fun `a hand-over plays the first entry alone and then queues the rest in order, a run at a time`() {
        handOver("catalog:1", "catalog:2", "catalog:3", "i.lib4", "catalog:5")
        assertEquals(
            HostEvent.Status(1, currentIndex = 0, currentId = "catalog:1", playing = true, positionMs = 0),
            statuses.last(),
        )
        answerAll()

        assertEquals(
            listOf("replace catalog:1", "play", "append catalog:2,catalog:3", "append i.lib4", "append catalog:5"),
            player.calls,
        )
        assertEquals(listOf("1", "2", "3", "i.lib4", "5"), player.items.map { it.storeId })
        assertTrue(failures.isEmpty())
    }

    @Test
    fun `moving on by itself reports the next index at once, and the indexes hold after it`() {
        handOver("catalog:a", "catalog:b", "catalog:c")
        answerAll()
        advance(5_000)
        events.clear()

        player.playOut()
        player.flush()
        assertEquals(
            HostEvent.Status(1, currentIndex = 1, currentId = "catalog:b", playing = true, positionMs = 0),
            statuses.single(),
        )

        // Next, after the host moved on to that entry by itself, starts it again from 0.
        batch(HostAction.Skip(1, 0), HostAction.Start)
        assertFalse(player.calls.contains("skip"))
        assertEquals("seek 0", player.calls.last { it.startsWith("seek") })
        assertEquals(1, statuses.last().currentIndex)

        // The tail goes by its index as before.
        val c = queueIdOf("c")
        batch(HostAction.Remove(2))
        assertEquals("remove $c", player.calls.last())
        assertEquals(listOf("a", "b"), player.items.map { it.storeId })
        assertEquals(1, statuses.last().currentIndex)
    }

    @Test
    fun `a song the service leaves out fails its entry, and the host does not play past it`() {
        player.missing += "x"
        handOver("catalog:a", "catalog:b", "catalog:x", "catalog:d")
        answerAll()

        assertEquals(listOf(HostEvent.ItemFailed(1, index = 2, reason = Failure.UNPLAYABLE)), failures)
        // The song after the one left out does not stay in the platform queue.
        assertEquals(listOf("a", "b"), player.items.map { it.storeId })

        player.playOut()
        player.flush()
        assertEquals(1, statuses.last().currentIndex)
        events.clear()
        player.playOut()
        player.flush()

        assertEquals(listOf(HostEvent.Ended(1, index = 1)), ends)
        assertEquals(1, statuses.last().currentIndex)
        assertFalse(statuses.last().playing)
        assertTrue(failures.isEmpty())
    }

    @Test
    fun `offline, a song left out counts as out of reach`() {
        player.missing += "x"
        online = false
        handOver("catalog:a", "catalog:x")
        answerAll()
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 1, reason = Failure.NETWORK)), failures)
    }

    @Test
    fun `the end of the queue is reported once, and the first entry is not played again`() {
        handOver("catalog:a", "catalog:b")
        answerAll()
        player.playOut()
        player.flush()
        val plays = player.calls.count { it == "play" }

        player.playOut()
        player.flush()
        advance(10_000)

        assertEquals(listOf(HostEvent.Ended(1, index = 1)), ends)
        assertEquals(
            HostEvent.Status(1, currentIndex = 1, currentId = "catalog:b", playing = false, positionMs = FakePlayer.DURATION),
            statuses.last(),
        )
        assertEquals(PlayerState.PAUSED, player.state)

        // A batch after the end is answered with the end again, and starts nothing.
        batch(HostAction.Start)
        assertEquals(HostEvent.Ended(2, index = 1), ends.last())
        assertEquals(2, ends.size)
        assertEquals(plays, player.calls.count { it == "play" })
        assertFalse(statuses.last().playing)
    }

    @Test
    fun `the answers of a replaced queue do not touch the new one`() {
        handOver("catalog:a", "catalog:b")
        answerAll()
        val oldA = player.current!!
        val oldB = player.upcoming.single()
        advance(5_000)

        handOver("catalog:c")
        events.clear()
        // What the old queue still says: a natural end, a move on and a stop.
        jukebox.onItemEnded(oldA, oldA.durationMs)
        jukebox.onCurrentItemChanged(oldB)
        jukebox.onStateChanged(PlayerState.STOPPED)
        answerAll()
        jukebox.onCurrentItemChanged(oldB)
        jukebox.onItemEnded(oldB, oldB.durationMs)
        jukebox.onStateChanged(PlayerState.STOPPED)
        advance(5_000)

        assertTrue(failures.isEmpty())
        assertTrue(ends.isEmpty())
        assertTrue(statuses.all { it.currentIndex == 0 && it.currentId == "catalog:c" })
        assertTrue(statuses.last().playing)
    }

    @Test
    fun `the failure of a change that a new queue overtook is not the new queue's`() {
        handOver("catalog:a", "catalog:b")
        player.answer()
        player.flush()
        assertEquals("append catalog:b", player.calls.last())

        // A new queue while that append is still unanswered, which then fails.
        handOver("catalog:c")
        player.refuse(PlayerError.QUEUE)
        player.flush()
        assertTrue(failures.isEmpty())

        answerAll()
        advance(5_000)
        assertTrue(failures.isEmpty())
        val last = statuses.last()
        assertEquals(HostEvent.Status(2, 0, "catalog:c", true, last.positionMs), last)
        assertTrue(last.positionMs > 0)
    }

    @Test
    fun `a queue change that fails for another reason fails its entry as out of reach`() {
        handOver("catalog:a")
        player.refuse(PlayerError.QUEUE)
        player.flush()
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NETWORK)), failures)
        assertFalse(player.calls.contains("play"))
    }

    @Test
    fun `an item left on a skip is not an end, and the next entry plays from the skip`() {
        handOver("catalog:a", "catalog:b")
        answerAll()
        advance(5_000)

        batch(HostAction.Skip(1, 0))
        player.flush()
        assertEquals("skip", player.calls.last { it == "skip" || it.startsWith("replace") })
        advance(5_000)
        assertTrue(ends.isEmpty())
        assertTrue(failures.isEmpty())
        assertEquals(1, statuses.last().currentIndex)
        assertTrue(statuses.last().positionMs in 1..5_000)

        // The last entry left short of its end, without the host asking, is not an end either.
        val b = player.current!!
        jukebox.onItemEnded(b, 12_000)
        jukebox.onStateChanged(PlayerState.STOPPED)
        assertTrue(ends.isEmpty())
        assertEquals("replace catalog:b", player.calls.last { it.startsWith("replace") })
    }

    @Test
    fun `an entry the platform player gives up on plays on from where it was left, once`() {
        handOver("catalog:a", "catalog:b")
        answerAll()
        advance(5_000)
        events.clear()

        player.giveUp()
        player.flush()
        assertEquals("replace catalog:a", player.calls.last { it.startsWith("replace") })
        answerAll()
        assertTrue(player.calls.containsAll(listOf("seek 5000", "play")))
        assertTrue(statuses.all { it.currentIndex == 0 && it.playing })
        assertTrue(failures.isEmpty())
        assertEquals(listOf("a", "b"), player.items.map { it.storeId })

        // A second time, the entry fails rather than the host moving past it.
        advance(3_000)
        player.giveUp()
        player.flush()
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NETWORK)), failures)
        assertEquals(0, statuses.last().currentIndex)
        assertFalse(statuses.last().playing)
    }

    @Test
    fun `the last entry given up on with nothing after it plays on too`() {
        handOver("catalog:a")
        answerAll()
        advance(5_000)
        player.giveUp()
        player.flush()
        assertTrue(ends.isEmpty())
        assertEquals("replace catalog:a", player.calls.last { it.startsWith("replace") })
    }

    @Test
    fun `a hand-over at an offset seeks once the song's first data came, and only then plays`() {
        player.slowData = true
        handOver("catalog:a", offsetMs = 30_000)
        answerAll()
        advance(2_000)
        assertFalse(player.calls.any { it.startsWith("seek") || it == "play" })
        assertEquals(30_000L, statuses.last().positionMs)
        assertTrue(statuses.last().playing)

        player.dataArrives()
        player.flush()
        assertEquals(listOf("seek 30000", "play"), player.calls.takeLast(2))
        advance(2_000)
        assertTrue(statuses.last().positionMs > 30_000)
    }

    @Test
    fun `a seek before the current song's first data waits for it, paused`() {
        player.slowData = true
        handOver("catalog:a")
        answerAll()
        assertEquals("play", player.calls.last())

        batch(HostAction.Skip(0, 50_000))
        assertEquals("pause", player.calls.last())
        assertEquals(HostEvent.Status(2, 0, "catalog:a", true, 50_000), statuses.last())

        player.dataArrives()
        player.flush()
        assertEquals(listOf("seek 50000", "play"), player.calls.takeLast(2))
        assertTrue(failures.isEmpty())
    }

    @Test
    fun `a seek once the data came overtakes the one that waited for it`() {
        player.slowData = true
        handOver("catalog:a")
        answerAll()
        batch(HostAction.Skip(0, 50_000))
        player.dataArrives()
        batch(HostAction.Skip(0, 70_000))
        player.flush()
        advance(1_000)
        assertEquals(listOf("seek 70000", "play"), player.calls.filter { it.startsWith("seek") || it == "play" }.takeLast(2))
        assertFalse(player.calls.contains("seek 50000"))
    }

    @Test
    fun `a buffering entry reports playing where it starts, and fails once it made no progress for too long`() {
        player.buffering = true
        handOver("catalog:a", offsetMs = 30_000)
        answerAll()
        assertTrue(player.calls.containsAll(listOf("seek 30000", "play")))
        events.clear()

        advance(Jukebox.PROGRESS_LIMIT_MS - Jukebox.TICK_MS)
        assertTrue(statuses.size >= (Jukebox.PROGRESS_LIMIT_MS / Jukebox.REPORT_EVERY_MS - 1).toInt())
        assertTrue(statuses.all { it.playing && it.positionMs == 30_000L })
        assertTrue(failures.isEmpty())

        advance(Jukebox.TICK_MS)
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NETWORK)), failures)
        assertFalse(statuses.last().playing)
    }

    @Test
    fun `an entry that starts after buffering plays on`() {
        player.buffering = true
        handOver("catalog:a")
        answerAll()
        advance(10_000)
        player.buffering = false
        advance(Jukebox.PROGRESS_LIMIT_MS)
        assertTrue(failures.isEmpty())
        assertTrue(statuses.last().playing)
        assertTrue(statuses.last().positionMs > 0)
    }

    @Test
    fun `a time the process did not run does not count against the progress limit`() {
        player.buffering = true
        handOver("catalog:a")
        answerAll()
        advance(10_000)
        now += 60_000
        advance(Jukebox.TICK_MS * 2)
        assertTrue(failures.isEmpty())
    }

    @Test
    fun `a current song the service lacks fails once the player says so`() {
        player.missing += "x"
        handOver("catalog:x")
        assertTrue(statuses.last().playing)
        answerAll()
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.UNPLAYABLE)), failures)
        assertFalse(statuses.last().playing)
        assertFalse(player.calls.contains("play"))
    }

    @Test
    fun `a queue change that never shows fails its entry as out of reach`() {
        handOver("catalog:a")
        advance(Jukebox.CHANGE_TIMEOUT_MS)
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NETWORK)), failures)
    }

    @Test
    fun `the next entry that has yet to reach the platform queue is moved to with a queue of its own`() {
        handOver("catalog:a", "catalog:b")
        player.answer()
        player.flush()
        assertEquals("append catalog:b", player.calls.last())
        events.clear()

        // The current entry plays out before the service answered for the next one.
        player.playOut()
        player.flush()

        assertTrue(ends.isEmpty())
        assertEquals(1, statuses.first().currentIndex)
        assertTrue(statuses.first().playing)
        assertEquals("replace catalog:b", player.calls.last())
        answerAll()
        assertEquals(listOf("b"), player.items.map { it.storeId })
        assertEquals("play", player.calls.last())
    }

    @Test
    fun `a refused token fails the current entry and loses the access once`() {
        handOver("catalog:a")
        answerAll()
        jukebox.onError(PlayerError.TOKEN)
        jukebox.onError(PlayerError.TOKEN)
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NOT_AUTHORIZED)), failures)
        assertEquals(1, accessLost)
    }

    @Test
    fun `a hand-over to a host that cannot play fails its first entry and leaves the player alone`() {
        unready = Failure.NOT_AUTHORIZED
        handOver("catalog:a", "catalog:b")
        assertEquals(listOf(HostEvent.ItemFailed(1, index = 0, reason = Failure.NOT_AUTHORIZED)), failures)
        assertTrue(player.calls.none { it.startsWith("replace") || it.startsWith("append") })
        assertFalse(statuses.last().playing)
    }

    @Test
    fun `a remove aimed at the playing entry is ignored`() {
        handOver("catalog:a", "catalog:b")
        answerAll()
        player.playOut()
        player.flush()
        batch(HostAction.Remove(1))
        assertEquals(listOf("a", "b"), player.items.map { it.storeId })
        assertEquals(HostEvent.Status(3, 1, "catalog:b", true, 0), statuses.last().copy(revision = 3))
    }

    @Test
    fun `a stop pauses where the entry stands, and a seek moves within it`() {
        handOver("catalog:a")
        answerAll()
        advance(4_000)
        batch(HostAction.Stop)
        assertEquals("pause", player.calls.last())
        assertFalse(statuses.last().playing)
        assertEquals(4_000L, statuses.last().positionMs)

        batch(HostAction.Skip(0, 90_000))
        assertEquals("seek 90000", player.calls.last())
        assertEquals(90_000L, statuses.last().positionMs)

        batch(HostAction.Start)
        assertEquals("play", player.calls.last())
        assertTrue(statuses.last().playing)
    }

    @Test
    fun `clear stops the platform player and holds nothing`() {
        handOver("catalog:a")
        answerAll()
        batch(HostAction.Stop, HostAction.Clear)
        assertEquals("stop", player.calls.last())
        assertEquals(HostEvent.Status(2, null, null, false, 0), statuses.last())
        assertFalse(jukebox.wantsTicks)
    }

    @Test
    fun `an entry added after the end goes nowhere until the next set`() {
        handOver("catalog:a")
        answerAll()
        player.playOut()
        player.flush()
        val calls = player.calls.size
        batch(HostAction.Add(listOf("catalog:b")))
        assertEquals(calls, player.calls.size)
        assertEquals(HostEvent.Ended(2, index = 0), ends.last())
    }
}
