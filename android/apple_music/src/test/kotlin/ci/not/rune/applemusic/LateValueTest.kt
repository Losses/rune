package ci.not.rune.applemusic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LateValueTest {
    private var finish: ((String?) -> Unit)? = null
    private var starts = 0
    private val late = LateValue<String> {
        starts++
        finish = it
    }
    private val ran = mutableListOf<String>()

    @Test
    fun `calls that need the value before it exists wait for it and keep their order`() {
        late.call(creates = true) { ran += "first $it" }
        late.call { ran += "second $it" }
        late.call(creates = true) { ran += "third $it" }
        assertEquals(1, starts)
        assertEquals(emptyList<String>(), ran)
        assertTrue(late.making)

        finish!!("made")
        assertFalse(late.making)
        assertEquals(listOf("first made", "second made", "third made"), ran)
        late.call { ran += "later $it" }
        assertEquals("later made", ran.last())
        assertEquals(1, starts)
    }

    @Test
    fun `a call that does not create does nothing while there is no value and none is being made`() {
        late.call { ran += "lost" }
        assertEquals(0, starts)
        assertEquals(emptyList<String>(), ran)
    }

    @Test
    fun `a value that could not be made abandons what waited, and the next creating call tries again`() {
        late.call(creates = true, abandoned = { ran += "abandoned" }) { ran += "ran" }
        late.call { ran += "also" }
        finish!!(null)
        assertEquals(listOf("abandoned"), ran)
        assertNull(late.value)

        late.call(creates = true) { ran += "again $it" }
        assertEquals(2, starts)
        finish!!("made")
        assertEquals("again made", ran.last())
    }
}
