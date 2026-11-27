package ci.not.rune.applemusic

/**
 * A value that takes long to make, and that its users must not wait for: what needs it before it
 * exists is kept and runs, in the order it was asked, once it does. Everything here runs on one
 * thread. [start] begins the making, off that thread, and has [finished] called on this one with
 * the value, or with null when it could not be made. A later call that creates tries again.
 */
internal class LateValue<T : Any>(private val start: (finished: (T?) -> Unit) -> Unit) {
    private class Step<T>(val work: (T) -> Unit, val abandoned: () -> Unit)

    private val waiting = ArrayDeque<Step<T>>()
    private var making = false

    /** The value, once it was made. */
    var value: T? = null
        private set

    /**
     * Runs [work] with the value: now when it exists, else once it was made. Without the value
     * and without [creates] it does nothing, unless the value is being made already. [creates]
     * starts the making. [abandoned] runs instead of [work] when the making failed.
     */
    fun call(creates: Boolean = false, abandoned: () -> Unit = {}, work: (T) -> Unit) {
        value?.let { return work(it) }
        if (!making && !creates) return
        waiting.addLast(Step(work, abandoned))
        make()
    }

    /** Starts the making unless it is under way. */
    fun make() {
        if (value != null || making) return
        making = true
        start(::finished)
    }

    private fun finished(made: T?) {
        if (made == null) {
            making = false
            val dropped = waiting.toList()
            waiting.clear()
            dropped.forEach { it.abandoned() }
            return
        }
        // What runs meanwhile and asks for the value is kept behind what waits.
        while (waiting.isNotEmpty()) waiting.removeFirst().work(made)
        value = made
        making = false
    }
}
