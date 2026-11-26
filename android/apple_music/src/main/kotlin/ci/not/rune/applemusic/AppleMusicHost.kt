package ci.not.rune.applemusic

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import com.apple.android.sdk.authentication.AuthenticationFactory
import com.apple.android.sdk.authentication.TokenError

/**
 * Plays Apple Music for the hub's hosted player with MusicKit for Android, as the macOS and iOS
 * runner's `AppleMusicHost.swift` does with MusicKit.
 *
 * Dart only forwards. Each batch of commands arrives as JSON text ([batch]), and every event goes
 * back as JSON text to the [connect]ed sink. [authorize] and [status] are separate calls: Dart
 * attaches with [status], whose answer is the host's `hostReady`.
 *
 * There is one host per process, made on first use and kept for as long as the process lives,
 * since MusicKit's player must be: Flutter engines connect and disconnect, and the host plays on
 * in between. Everything happens on the main thread, where MusicKit calls back too.
 *
 * The host can play once this build has a developer token that holds and MusicKit's native
 * libraries load, and the user signed in through the Apple Music app. It requests no audio focus
 * and makes no service, session or notification: MusicKit takes the audio focus by itself, and
 * Rune's playback service shows every track.
 */
class AppleMusicHost private constructor(context: Context) {
    companion object {
        private const val TAG = "AppleMusicHost"

        /** The request code of the sign-in, whose result comes back to the activity. */
        const val SIGN_IN_REQUEST = 0x414d

        private const val APPLE_MUSIC_PACKAGE = "com.apple.android.music"

        /** How long a change of the network settles before it is reported. */
        private const val REACH_DEBOUNCE_MS = 1_000L

        @Volatile
        private var instance: AppleMusicHost? = null

        /** The host of this process, made on first use. */
        fun shared(context: Context): AppleMusicHost = instance ?: synchronized(this) {
            instance ?: AppleMusicHost(context.applicationContext).also { instance = it }
        }
    }

    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val tokens = TokenStore(this.context)
    private val player = MusicKitPlayer(this.context, tokens)
    private val network = NetworkWatch(this.context, ::reachChanged)
    private val jukebox = Jukebox(
        player = player,
        clock = SystemClock::elapsedRealtime,
        online = { network.online },
        unready = ::unready,
        send = { emit(HostMessages.encode(it)) },
        accessLost = ::accessLost,
        log = { Log.i(TAG, it) },
    )
    private val authentication by lazy {
        AuthenticationFactory.createAuthenticationManager(this.context)
    }

    private var sink: ((String) -> Unit)? = null

    /** What the latest `hostReady` said about the user's access, so that only a change restarts. */
    private var lastAuthorized: Boolean? = null

    /** The answers that wait for the sign-in under way. */
    private val signIns = mutableListOf<(Map<String, Any>?) -> Unit>()
    private var ticking = false
    private var lastReach: String? = null

    private val tick = Runnable {
        ticking = false
        jukebox.tick()
        keepTicking()
    }

    private val reach = Runnable { reportReach(onlyIfChanged = true) }

    /** Sends every event to [sink] from now on, as a Flutter engine attaches. */
    fun connect(sink: (String) -> Unit) {
        this.sink = sink
        network.start()
    }

    /** Stops sending events to [sink], unless another sink replaced it already. */
    fun disconnect(sink: (String) -> Unit) {
        if (this.sink === sink) this.sink = null
    }

    /** Answers with the `hostReady` of a host that just started, as Dart attaches. */
    fun status(reply: (String) -> Unit) {
        val answer: () -> Unit = {
            // The player takes the answer for a host that just started, so whatever is still
            // held from before goes in the same turn as the answer.
            jukebox.drop()
            reply(readyEvent())
            // The hub learns the network state of a host that just attached.
            main.post { reportReach(onlyIfChanged = false) }
        }
        if (natives.isInitialized() || !developerTokenHolds()) return answer()
        // Loading MusicKit's native libraries takes a moment, and making its player takes
        // seconds, which the main thread does not wait for.
        inBackground {
            natives.value
            main.post(answer)
            if (tokens.userToken != null) warmUp()
        }
    }

    /** Applies one batch, given as JSON text. */
    fun batch(json: String) {
        val batch = HostMessages.parseBatch(json)
        if (batch == null) {
            Log.w(TAG, "Ignoring a batch that does not parse")
            return
        }
        jukebox.apply(batch)
        keepTicking()
    }

    /**
     * Authorizes Rune to play the user's Apple Music, and answers with the tokens the hub signs
     * in with, or with why not, as the Swift host's reply: `refused` without access, `unsupported`
     * without the Apple Music app, `account` when the account has no subscription, and `noToken`
     * when the sign-in gave none otherwise. Null means that this build cannot play at all.
     *
     * Without [interactive] it answers with the token a sign-in stored, if any. With it, it signs
     * in through the Apple Music app from [activity], whose result comes back through
     * [onActivityResult], and stores the token.
     */
    fun authorize(activity: Activity?, interactive: Boolean, reply: (Map<String, Any>?) -> Unit) {
        if (!usable()) return reply(null)
        if (!interactive) return reply(tokens.userToken?.let(::authorization) ?: refused)
        if (!appleMusicInstalled()) return reply(unsupported)
        if (activity == null) return reply(null)
        signIns += reply
        // One sign-in at a time: whoever asks meanwhile gets its answer.
        if (signIns.size > 1) return
        try {
            val intent = authentication.createIntentBuilder(BuildConfig.DEVELOPER_TOKEN)
                .setHideStartScreen(false)
                .build()
            activity.startActivityForResult(intent, SIGN_IN_REQUEST)
        } catch (error: RuntimeException) {
            Log.w(TAG, "The sign-in could not start: ${error.javaClass.simpleName}")
            answerSignIns(null)
        }
    }

    /** Takes the result of the sign-in. False for a result of anything else. */
    fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean {
        if (requestCode != SIGN_IN_REQUEST) return false
        val result = authentication.handleTokenResult(data)
        val token = result.musicUserToken
        val reply = if (!result.isError && !token.isNullOrEmpty()) {
            tokens.store(token, result.storefrontId, result.appleMusicStorefrontId)
            player.tokensChanged()
            Log.i(TAG, "Signed in to Apple Music")
            inBackground(::warmUp)
            authorization(token)
        } else {
            Log.w(TAG, "The sign-in gave no token: ${result.error}, result $resultCode")
            signInFailure(result.error, resultCode)
        }
        // A change of access restarts the host. Without one the host keeps what it holds: a
        // `hostReady` would make the player hand its track over again.
        if ((tokens.userToken != null) != lastAuthorized) restart()
        answerSignIns(reply)
        return true
    }

    private fun answerSignIns(reply: Map<String, Any>?) {
        val waiting = signIns.toList()
        signIns.clear()
        waiting.forEach { it(reply) }
    }

    private val refused: Map<String, Any> = mapOf("authorized" to false, "reason" to "refused")

    /** As the Swift host answers on a system too old for Apple Music: Android lacks its app. */
    private val unsupported: Map<String, Any> =
        mapOf("authorized" to false, "reason" to "unsupported")

    private fun authorization(token: String): Map<String, Any> = mapOf(
        "authorized" to true,
        "token" to token,
        "developerToken" to BuildConfig.DEVELOPER_TOKEN,
    )

    /**
     * Why a sign-in gave no token, as Dart reads it. Dart only reads the reason of a reply without
     * a token that says access was given.
     */
    private fun signInFailure(error: TokenError?, resultCode: Int): Map<String, Any> = when {
        // Leaving Apple's screens without an answer comes back as unknown.
        error == TokenError.USER_CANCELLED ||
            (error == TokenError.UNKNOWN && resultCode == Activity.RESULT_CANCELED) -> refused
        error == TokenError.NO_SUBSCRIPTION || error == TokenError.SUBSCRIPTION_EXPIRED ->
            mapOf("authorized" to true, "reason" to "account")
        else -> mapOf("authorized" to true, "reason" to "noToken")
    }

    @Suppress("DEPRECATION")
    private fun appleMusicInstalled() = try {
        context.packageManager.getPackageInfo(APPLE_MUSIC_PACKAGE, 0)
        true
    } catch (error: PackageManager.NameNotFoundException) {
        false
    }

    /** Whether this build can play: its developer token holds and MusicKit's libraries load. */
    private fun usable() = developerTokenHolds() && natives.value

    private fun developerTokenHolds() =
        DeveloperToken.holds(BuildConfig.DEVELOPER_TOKEN, System.currentTimeMillis() / 1000)

    /** Whether MusicKit's native libraries, which it never loads by itself, loaded, once. */
    private val natives = lazy {
        try {
            // As Apple's sample does: JavaCPP, which the SDK uses, then keeps no limits of its own.
            System.setProperty("org.bytedeco.javacpp.maxphysicalbytes", "0")
            System.setProperty("org.bytedeco.javacpp.maxbytes", "0")
            System.loadLibrary("c++_shared")
            System.loadLibrary("appleMusicSDK")
            Log.i(TAG, "MusicKit's native libraries loaded")
            true
        } catch (error: LinkageError) {
            Log.w(TAG, "MusicKit's native libraries did not load: ${error.javaClass.simpleName}")
            false
        }
    }

    /** Makes MusicKit's player ahead of the first track, off the main thread, if it can play. */
    private fun warmUp() {
        if (usable()) player.makeController()
    }

    private fun inBackground(work: () -> Unit) {
        Thread(work, "AppleMusicHost").start()
    }

    /** Why nothing can be played now, or null when the host can play. */
    private fun unready(): Failure? = when {
        !usable() -> Failure.NETWORK
        tokens.userToken == null -> Failure.NOT_AUTHORIZED
        else -> null
    }

    /** The `hostReady` event for what the host can do now, whose access it records. */
    private fun readyEvent(): String {
        val usable = usable()
        val authorized = tokens.userToken != null
        lastAuthorized = authorized
        // A build that cannot play has nothing to sign in to, so Rune is not asked to.
        return HostMessages.ready(
            authorized = authorized || !usable,
            canPlay = authorized && usable,
        )
    }

    /**
     * Reports the host as started again. It drops its queue and says so in the same turn, so that
     * no batch comes in between: the player then hands the current track over again.
     */
    private fun restart() {
        jukebox.drop()
        emit(readyEvent())
    }

    /**
     * Apple Music refused the user's token while playing. The token is forgotten, so that the next
     * sign-in goes through the Apple Music app again, and the host starts again without access.
     * A developer token that expired is refused the same way, and the user's token is kept then.
     */
    private fun accessLost() {
        if (developerTokenHolds()) {
            Log.w(TAG, "Apple Music refused the user's token, which is forgotten")
            tokens.forget()
            player.tokensChanged()
        }
        restart()
    }

    private fun keepTicking() {
        if (ticking || !jukebox.wantsTicks) return
        ticking = true
        main.postDelayed(tick, Jukebox.TICK_MS)
    }

    private fun reachChanged() {
        main.removeCallbacks(reach)
        main.postDelayed(reach, REACH_DEBOUNCE_MS)
    }

    private fun reportReach(onlyIfChanged: Boolean) {
        val json = HostMessages.networkState(network.online)
        if (onlyIfChanged && json == lastReach) return
        lastReach = json
        emit(json)
    }

    private fun emit(json: String) {
        sink?.invoke(json)
    }
}
