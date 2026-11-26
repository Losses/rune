package ci.not.rune.applemusic

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64

class HostMessagesTest {
    /** org.json on the JVM keeps no order of keys, so objects are compared as such. */
    private fun assertJson(expected: String, actual: String) {
        assertEquals(tree(JSONObject(expected)), tree(JSONObject(actual)))
    }

    private fun tree(value: Any?): Any? = when (value) {
        is JSONObject -> value.keys().asSequence().associateWith { tree(value.get(it)) }
        is JSONArray -> (0 until value.length()).map { tree(value.get(it)) }
        is Number -> value.toDouble()
        else -> value
    }

    @Test
    fun `a batch reads as the player writes it`() {
        // As in `a_batch_carries_jukebox_actions_in_camel_case` in playback/src/hosted.rs.
        val batch = HostMessages.parseBatch(
            """{"host":"appleMusic","revision":12,"actions":[{"action":"set","ids":["track-1"]},""" +
                """{"action":"skip","index":0,"offsetMs":1500},{"action":"setGain","gain":0.6},{"action":"start"},""" +
                """{"action":"add","ids":["1440"]},{"action":"remove","index":2},{"action":"clear"},""" +
                """{"action":"stop"},{"action":"shuffle"}]}""",
        )
        assertEquals(
            HostBatch(
                12,
                listOf(
                    HostAction.Set(listOf("track-1")),
                    HostAction.Skip(0, 1500),
                    HostAction.SetGain(0.6),
                    HostAction.Start,
                    HostAction.Add(listOf("1440")),
                    HostAction.Remove(2),
                    HostAction.Clear,
                    HostAction.Stop,
                    HostAction.Unknown,
                ),
            ),
            batch,
        )
        assertNull(HostMessages.parseBatch("not json"))
    }

    @Test
    fun `events carry their type, the host and the revision`() {
        assertJson(
            """{"type":"jukeboxStatus","host":"appleMusic","revision":3,"currentIndex":0,"currentId":"track-1","playing":true,"positionMs":1500}""",
            HostMessages.encode(HostEvent.Status(3, 0, "track-1", true, 1500)),
        )
        assertJson(
            """{"type":"jukeboxStatus","host":"appleMusic","revision":3,"currentIndex":0,"currentId":"track-1","playing":true,"positionMs":0,"buffering":true}""",
            HostMessages.encode(HostEvent.Status(3, 0, "track-1", true, 0, buffering = true)),
        )
        val idle = JSONObject(HostMessages.encode(HostEvent.Status(3, null, null, false, 0)))
        assertTrue(idle.isNull("currentIndex") && idle.has("currentIndex"))
        assertTrue(idle.isNull("currentId") && idle.has("currentId"))
        assertJson(
            """{"type":"jukeboxItemFailed","host":"appleMusic","revision":3,"index":1,"reason":"notAuthorized"}""",
            HostMessages.encode(HostEvent.ItemFailed(3, 1, Failure.NOT_AUTHORIZED)),
        )
        assertJson(
            """{"type":"jukeboxEnded","host":"appleMusic","revision":4,"index":2}""",
            HostMessages.encode(HostEvent.Ended(4, 2)),
        )
        assertFalse(JSONObject(HostMessages.encode(HostEvent.Ended(4, null))).has("index"))
        assertJson(
            """{"type":"hostReady","host":"appleMusic","authorized":true,"canPlay":false,"showsInMediaControls":false}""",
            HostMessages.ready(authorized = true, canPlay = false),
        )
    }

    @Test
    fun `offline, the network state names no stored songs`() {
        assertJson(
            """{"type":"networkState","host":"appleMusic","online":false,"downloaded":[]}""",
            HostMessages.networkState(false),
        )
        assertJson(
            """{"type":"networkState","host":"appleMusic","online":true}""",
            HostMessages.networkState(true),
        )
    }

    private fun jwt(payload: String): String {
        val encoder = Base64.getUrlEncoder().withoutPadding()
        return listOf("""{"alg":"ES256"}""", payload, "signature")
            .joinToString(".") { encoder.encodeToString(it.toByteArray()) }
    }

    @Test
    fun `a developer token holds until its expiry`() {
        val token = jwt("""{"iss":"team","iat":1000,"exp":2000}""")
        assertTrue(DeveloperToken.holds(token, 1999))
        assertFalse(DeveloperToken.holds(token, 2000))
        assertFalse(DeveloperToken.holds(jwt("""{"iss":"team"}"""), 0))
        assertFalse(DeveloperToken.holds("", 0))
        assertFalse(DeveloperToken.holds("a.b", 0))
        assertFalse(DeveloperToken.holds("a.%%%.c", 0))
    }

    @Test
    fun `base64url decodes as the JDK does`() {
        val encoder = Base64.getUrlEncoder().withoutPadding()
        for (length in 0..40) {
            val bytes = ByteArray(length) { (it * 37 + 251).toByte() }
            assertEquals(bytes.toList(), DeveloperToken.decodeBase64Url(encoder.encodeToString(bytes))?.toList())
        }
        assertNull(DeveloperToken.decodeBase64Url("a"))
    }
}
