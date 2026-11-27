package ci.not.rune.applemusic

import android.content.Context
import android.util.Log
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import javax.net.ssl.HttpsURLConnection

/**
 * The decisions about which developer token to use and when to look for a newer one. They depend
 * on neither Android nor the network. A token's text is never logged.
 */
internal object TokenPolicy {
    /** A token that expires within this many seconds is replaced if a newer one can be had. */
    const val RENEW_WITHIN_SECONDS = 7L * 24 * 60 * 60

    /** The least time between two attempts to fetch a token, and between two forced ones. */
    const val ATTEMPT_INTERVAL_MS = 60L * 60 * 1000
    const val FORCED_INTERVAL_MS = 5L * 60 * 1000

    /** The least time after an attempt that got no answer, as when the device was offline. */
    const val FAILED_INTERVAL_MS = 60L * 1000

    /** The longest life Apple allows a developer token, and a day for clocks that differ. */
    const val MAX_LIFETIME_SECONDS = 15_777_000L
    const val CLOCK_SLACK_SECONDS = 24L * 60 * 60

    /** The most that is read of an answer. A token is far shorter. */
    const val MAX_BYTES = 4096

    /** Of the [candidates] that still hold, the one that expires last, or empty if none does. */
    fun best(candidates: List<String>, nowSeconds: Long): String = candidates
        .filter { DeveloperToken.holds(it, nowSeconds) }
        .maxByOrNull { DeveloperToken.expiry(it) ?: 0L }
        ?: ""

    /** Whether the best token is missing or about to expire, so that a newer one is wanted. */
    fun refreshDue(best: String, nowSeconds: Long): Boolean {
        val expiry = DeveloperToken.expiry(best)
        return expiry == null || expiry - nowSeconds <= RENEW_WITHIN_SECONDS
    }

    /**
     * Whether an attempt may be made [sinceLastMs] after the last one, which is null if there was
     * none yet. A forced attempt waits less, and so does one after an attempt that got no answer
     * ([lastFailed]), which is worth trying again soon.
     */
    fun attemptAllowed(sinceLastMs: Long?, forced: Boolean, lastFailed: Boolean = false): Boolean {
        if (sinceLastMs == null) return true
        var wait = if (forced) FORCED_INTERVAL_MS else ATTEMPT_INTERVAL_MS
        if (lastFailed) wait = minOf(wait, FAILED_INTERVAL_MS)
        return sinceLastMs >= wait
    }

    /**
     * The token in a fetched [text], or null when it is not one worth keeping: it must be a JWT
     * that holds, expires later than the [best] one, and does not live longer than Apple allows.
     */
    fun accept(text: String, best: String, nowSeconds: Long): String? {
        val token = text.trim()
        if (!DeveloperToken.holds(token, nowSeconds)) return null
        val expiry = DeveloperToken.expiry(token) ?: return null
        if (expiry > nowSeconds + MAX_LIFETIME_SECONDS + CLOCK_SLACK_SECONDS) return null
        val bestExpiry = DeveloperToken.expiry(best) ?: 0L
        return if (expiry > bestExpiry) token else null
    }
}

/**
 * Where the developer token comes from: the one compiled into this build, and a newer one fetched
 * from the build's token URL and kept in the app's private preferences. Whichever expires last,
 * among those that have not expired, is the current one. An empty URL never fetches.
 *
 * MusicKit keeps its own copy of the tokens and takes new ones when told that they changed, which
 * the host does through `MusicKitPlayer.tokensChanged` after [refresh] stored one.
 */
internal class DeveloperTokens(context: Context) {
    private companion object {
        const val TAG = "DeveloperTokens"
        const val FETCHED = "fetchedToken"
        const val TIMEOUT_MS = 15_000
    }

    private val compiled = BuildConfig.DEVELOPER_TOKEN
    private val url = BuildConfig.DEVELOPER_TOKEN_URL
    private val preferences =
        context.getSharedPreferences("apple_music_developer_token", Context.MODE_PRIVATE)

    private val lock = Any()
    private var lastAttemptMs: Long? = null
    private var fetching = false

    /** Whether the last attempt got no answer from the token URL. */
    private var lastFailed = false

    /** The developer token to use now, or empty when none holds. */
    val current: String
        get() = TokenPolicy.best(candidates(), nowSeconds())

    private fun candidates() = listOf(compiled, preferences.getString(FETCHED, null) ?: "")

    private fun nowSeconds() = System.currentTimeMillis() / 1000

    /**
     * Fetches a newer token in the background if one is wanted: none holds or the best expires
     * soon, and the last attempt is long enough ago, sooner when it got no answer. A [forced]
     * attempt skips the first condition, for a token that holds by its `exp` but that Apple Music
     * refuses. [stored] runs on the fetching thread after a new token was kept.
     */
    fun refresh(forced: Boolean, stored: () -> Unit) {
        if (url.isEmpty()) return
        synchronized(lock) {
            if (fetching) return
            val now = System.currentTimeMillis()
            val since = lastAttemptMs?.let { now - it }
            if (!TokenPolicy.attemptAllowed(since, forced, lastFailed)) return
            if (!forced && !TokenPolicy.refreshDue(current, nowSeconds())) return
            fetching = true
            lastAttemptMs = now
        }
        Thread({
            try {
                if (fetchAndStore()) stored()
            } finally {
                synchronized(lock) { fetching = false }
            }
        }, "DeveloperTokens").start()
    }

    private fun fetchAndStore(): Boolean {
        val text = try {
            fetch()
        } catch (error: Exception) {
            Log.w(TAG, "The developer token could not be fetched: ${error.javaClass.simpleName}")
            synchronized(lock) { lastFailed = true }
            return false
        }
        synchronized(lock) { lastFailed = false }
        val token = TokenPolicy.accept(text, current, nowSeconds())
        if (token == null) {
            Log.w(TAG, "The fetched developer token was not newer than the one in use, or not valid")
            return false
        }
        preferences.edit().putString(FETCHED, token).apply()
        Log.i(TAG, "A newer developer token was stored")
        return true
    }

    private fun fetch(): String {
        val connection = URL(url).openConnection() as HttpsURLConnection
        try {
            connection.requestMethod = "GET"
            connection.connectTimeout = TIMEOUT_MS
            connection.readTimeout = TIMEOUT_MS
            connection.instanceFollowRedirects = false
            if (connection.responseCode != HttpURLConnection.HTTP_OK) {
                throw IOException("status ${connection.responseCode}")
            }
            val buffer = ByteArray(TokenPolicy.MAX_BYTES)
            var size = 0
            connection.inputStream.use { input ->
                while (size < buffer.size) {
                    val read = input.read(buffer, size, buffer.size - size)
                    if (read < 0) break
                    size += read
                }
            }
            return String(buffer, 0, size, Charsets.UTF_8).trim()
        } finally {
            connection.disconnect()
        }
    }
}
