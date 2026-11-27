package ci.not.rune.applemusic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64

class TokenPolicyTest {
    private val day = 24 * 60 * 60L
    private val now = 1_000_000L

    private fun jwt(expiry: Long): String {
        val encoder = Base64.getUrlEncoder().withoutPadding()
        return listOf("""{"alg":"ES256"}""", """{"iss":"team","exp":$expiry}""", "signature")
            .joinToString(".") { encoder.encodeToString(it.toByteArray()) }
    }

    @Test
    fun `the expiry is read from a JWT only`() {
        assertEquals(1234L, DeveloperToken.expiry(jwt(1234)))
        assertNull(DeveloperToken.expiry("a.b.c"))
        assertNull(DeveloperToken.expiry(""))
    }

    @Test
    fun `the token that expires last and has not expired wins`() {
        val soon = jwt(now + day)
        val late = jwt(now + 30 * day)
        val gone = jwt(now - 1)
        assertEquals(late, TokenPolicy.best(listOf(soon, late, gone), now))
        assertEquals(soon, TokenPolicy.best(listOf(gone, soon, ""), now))
        assertEquals("", TokenPolicy.best(listOf(gone, "", "junk"), now))
    }

    @Test
    fun `a refresh is due without a token and when the best expires within a week`() {
        assertTrue(TokenPolicy.refreshDue("", now))
        assertTrue(TokenPolicy.refreshDue(jwt(now + 6 * day), now))
        assertTrue(TokenPolicy.refreshDue(jwt(now + 7 * day), now))
        assertFalse(TokenPolicy.refreshDue(jwt(now + 8 * day), now))
    }

    @Test
    fun `attempts are spaced by an hour, forced ones by five minutes`() {
        assertTrue(TokenPolicy.attemptAllowed(null, false))
        assertFalse(TokenPolicy.attemptAllowed(59 * 60_000L, false))
        assertTrue(TokenPolicy.attemptAllowed(60 * 60_000L, false))
        assertFalse(TokenPolicy.attemptAllowed(4 * 60_000L, true))
        assertTrue(TokenPolicy.attemptAllowed(5 * 60_000L, true))
    }

    @Test
    fun `a fetched text is accepted only as a newer token that holds`() {
        val best = jwt(now + 3 * day)
        val newer = jwt(now + 40 * day)
        assertEquals(newer, TokenPolicy.accept("  $newer\n", best, now))
        assertEquals(newer, TokenPolicy.accept(newer, "", now))
        assertNull(TokenPolicy.accept(best, best, now))
        assertNull(TokenPolicy.accept(jwt(now + day), best, now))
        assertNull(TokenPolicy.accept(jwt(now - 1), "", now))
        assertNull(TokenPolicy.accept("<html>not found</html>", "", now))
        assertNull(TokenPolicy.accept("", "", now))
    }
}
