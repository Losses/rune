package ci.not.rune.applemusic

import org.json.JSONException
import org.json.JSONObject

/** The developer token, a JWT that Apple signed for the app, and whether it still holds. */
internal object DeveloperToken {
    /**
     * Whether [token] is a JWT whose `exp` lies after [nowSeconds]. An empty token, one that does
     * not read as a JWT, and one that has expired do not hold. Its text is never logged.
     */
    fun holds(token: String, nowSeconds: Long): Boolean {
        val expiry = expiry(token) ?: return false
        return expiry > nowSeconds
    }

    /**
     * The `exp` of [token] in seconds, or null when it is not a JWT with one. Its text is never
     * logged.
     */
    fun expiry(token: String): Long? {
        val parts = token.split('.')
        if (parts.size != 3 || parts.any { it.isEmpty() }) return null
        val payload = decodeBase64Url(parts[1]) ?: return null
        val expiry = try {
            JSONObject(String(payload, Charsets.UTF_8)).optLong("exp", 0)
        } catch (error: JSONException) {
            return null
        }
        return if (expiry > 0) expiry else null
    }

    private const val ALPHABET =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"

    /**
     * Decodes unpadded base64url, as JWTs use, or returns null for text that is not. Written out
     * here as `java.util.Base64` needs Android 8 and `android.util.Base64` is not there on the JVM.
     */
    fun decodeBase64Url(text: String): ByteArray? {
        val trimmed = text.trimEnd('=')
        if (trimmed.length % 4 == 1) return null
        val out = java.io.ByteArrayOutputStream(trimmed.length * 3 / 4)
        var buffer = 0
        var bits = 0
        for (char in trimmed) {
            val value = ALPHABET.indexOf(char)
            if (value < 0) return null
            // At most 13 bits wait to be written.
            buffer = ((buffer shl 6) or value) and 0xffff
            bits += 6
            if (bits >= 8) {
                bits -= 8
                out.write((buffer shr bits) and 0xff)
            }
        }
        return out.toByteArray()
    }
}
