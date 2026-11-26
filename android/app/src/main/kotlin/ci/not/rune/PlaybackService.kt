package ci.not.rune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import kotlin.concurrent.thread
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * The media session that the lock screen, quick settings, headset and Bluetooth buttons and other
 * apps control, with its MediaStyle notification, the audio focus, and headphones that go away.
 *
 * The hub's playback (`playback/src/android_media_session.rs`) decides all of it: it calls
 * [update] from any thread with what the session shows and whether the service runs, and every
 * call is posted to the main thread, where everything here happens, in the order it was made.
 * What the user does with the controls, and what audio focus does, goes back to it.
 *
 * The service starts when playback starts. It runs in the foreground while the player plays, or
 * waits to play again once audio focus comes back. Paused, it runs in the background and keeps
 * its notification. It stops when playback stops, when the app's task is removed, and when the
 * system stops it in the background.
 */
class PlaybackService : Service() {
    companion object {
        private const val TAG = "PlaybackService"

        // What the session shows, `State` in the hub.
        private const val STATE_STOPPED = 0
        private const val STATE_PAUSED = 1
        private const val STATE_PLAYING = 2

        // Whether the service runs, and in the foreground, `Service` in the hub.
        private const val SERVICE_STOPPED = 0
        private const val SERVICE_BACKGROUND = 1
        private const val SERVICE_FOREGROUND = 2

        // The commands the hub takes, `COMMAND_*` in the hub.
        private const val COMMAND_PLAY = 0
        private const val COMMAND_PAUSE = 1
        private const val COMMAND_NEXT = 2
        private const val COMMAND_PREVIOUS = 3
        private const val COMMAND_STOP = 4
        private const val COMMAND_SEEK_TO = 5

        // The notification's buttons, which start the service with these actions.
        private const val ACTION_PLAY = "ci.not.rune.action.PLAY"
        private const val ACTION_PAUSE = "ci.not.rune.action.PAUSE"
        private const val ACTION_NEXT = "ci.not.rune.action.NEXT"
        private const val ACTION_PREVIOUS = "ci.not.rune.action.PREVIOUS"

        private const val CHANNEL_ID = "playback"
        private const val NOTIFICATION_ID = 1

        /** The longer side of the cover, in pixels, small enough for a binder transaction. */
        private const val ARTWORK_SIZE = 320

        private const val ACTIONS = PlaybackState.ACTION_PLAY or
            PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_PLAY_PAUSE or
            PlaybackState.ACTION_SKIP_TO_NEXT or
            PlaybackState.ACTION_SKIP_TO_PREVIOUS or
            PlaybackState.ACTION_SEEK_TO or
            PlaybackState.ACTION_STOP

        private val main = Handler(Looper.getMainLooper())

        // Only the main thread touches these: the last write, and the service while it runs.
        private var latest: Shown? = null
        private var running: PlaybackService? = null

        /**
         * Shows what the hub sends, from any thread. A duration of -1 is none, and the position is
         * the one at the moment of the call, which the session moves on at [speed].
         */
        @JvmStatic
        fun update(
            context: Context,
            title: String?,
            artist: String?,
            album: String?,
            durationMs: Long,
            artwork: String?,
            state: Int,
            service: Int,
            positionMs: Long,
            speed: Float,
        ) {
            val shown = Shown(
                title, artist, album, durationMs, artwork, state, service, positionMs, speed,
                SystemClock.elapsedRealtime(),
            )
            main.post { show(context.applicationContext, shown) }
        }

        private fun show(context: Context, shown: Shown) {
            latest = shown
            val service = running
            if (service != null) {
                service.show(shown)
            } else if (shown.service == SERVICE_FOREGROUND) {
                start(context)
            }
        }

        private fun start(context: Context) {
            val intent = Intent(context, PlaybackService::class.java)
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
            } catch (e: IllegalStateException) {
                // Android 12 and later refuse a foreground service that starts in the background
                // without an exemption, and Android 8 and later any service that does.
                Log.w(TAG, "Could not start the playback service: $e")
            }
        }
    }

    /** One write of the hub. */
    private class Shown(
        val title: String?,
        val artist: String?,
        val album: String?,
        val durationMs: Long,
        val artwork: String?,
        val state: Int,
        val service: Int,
        val positionMs: Long,
        val speed: Float,
        /** When the position was, by [SystemClock.elapsedRealtime]. */
        val at: Long,
    )

    private lateinit var session: MediaSession
    private lateinit var notifications: NotificationManager
    private lateinit var audio: AudioManager
    private var focusRequest: AudioFocusRequest? = null
    private var focusHeld = false
    private var noisyRegistered = false
    private var foreground = false
    private var released = false
    private var artworkPath: String? = null
    private var artwork: Bitmap? = null

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = nativeCommand(COMMAND_PLAY, 0)
        override fun onPause() = nativeCommand(COMMAND_PAUSE, 0)
        override fun onSkipToNext() = nativeCommand(COMMAND_NEXT, 0)
        override fun onSkipToPrevious() = nativeCommand(COMMAND_PREVIOUS, 0)
        override fun onStop() = nativeCommand(COMMAND_STOP, 0)
        override fun onSeekTo(pos: Long) = nativeCommand(COMMAND_SEEK_TO, pos)
    }

    private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
        // A loss for good ends the request, so the next playback asks again.
        if (change == AudioManager.AUDIOFOCUS_LOSS) abandonFocus()
        nativeAudioFocusChange(change)
    }

    /** Headphones that are unplugged or turned off pause, before the audio moves to the speaker. */
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                nativeCommand(COMMAND_PAUSE, 0)
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        notifications = getSystemService(NotificationManager::class.java)
        audio = getSystemService(AudioManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.playback_channel),
                NotificationManager.IMPORTANCE_LOW,
            )
            channel.setShowBadge(false)
            notifications.createNotificationChannel(channel)
        }
        session = MediaSession(this, TAG)
        session.setCallback(callback, main)
        session.setSessionActivity(openApp())
        // Android 8 and later always give the session these.
        @Suppress("DEPRECATION")
        session.setFlags(
            MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS
        )
        running = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val shown = latest
        if (shown == null) {
            // A button of a notification that an earlier process left: nothing here plays.
            release()
            return START_NOT_STICKY
        }
        when (intent?.action) {
            ACTION_PLAY -> nativeCommand(COMMAND_PLAY, 0)
            ACTION_PAUSE -> nativeCommand(COMMAND_PAUSE, 0)
            ACTION_NEXT -> nativeCommand(COMMAND_NEXT, 0)
            ACTION_PREVIOUS -> nativeCommand(COMMAND_PREVIOUS, 0)
            else -> {
                // Started by `start`: the service goes to the foreground at once, as
                // `startForegroundService` asks, even if playback paused meanwhile.
                enterForeground(notification(shown))
                show(shown)
            }
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Playback ends with the app's task.
        nativeCommand(COMMAND_PAUSE, 0)
        release()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        release()
        super.onDestroy()
    }

    private fun show(shown: Shown) {
        if (released) return
        if (shown.service == SERVICE_STOPPED) {
            release()
            return
        }
        loadArtwork(shown.artwork)
        session.setMetadata(metadata(shown))
        session.setPlaybackState(playbackState(shown))
        session.isActive = true
        val notification = notification(shown)
        when (shown.service) {
            SERVICE_FOREGROUND -> enterForeground(notification)
            SERVICE_BACKGROUND -> {
                // The notification stays, and can be dismissed.
                if (foreground) {
                    stopForeground(STOP_FOREGROUND_DETACH)
                    foreground = false
                }
                notifications.notify(NOTIFICATION_ID, notification)
            }
        }
        if (shown.state == STATE_PLAYING) {
            registerNoisy()
            // After the service is in the foreground: Android 15 and later refuse audio focus
            // to an app in the background without one.
            requestFocus()
        } else {
            unregisterNoisy()
        }
    }

    private fun enterForeground(notification: Notification) {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
                )
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            foreground = true
        } catch (e: IllegalStateException) {
            // Android 12 and later refuse the foreground to a service in the background without
            // an exemption.
            Log.w(TAG, "Could not run the playback service in the foreground: $e")
        }
    }

    /** Ends the session and stops the service. Once released, the service shows nothing more. */
    private fun release() {
        if (released) return
        released = true
        if (running === this) running = null
        abandonFocus()
        unregisterNoisy()
        if (::session.isInitialized) {
            session.isActive = false
            session.release()
        }
        stopForeground(STOP_FOREGROUND_REMOVE)
        foreground = false
        notifications.cancel(NOTIFICATION_ID)
        stopSelf()
    }

    private fun requestFocus() {
        if (focusHeld) return
        val result = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val request = focusRequest ?: AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                )
                .setOnAudioFocusChangeListener(focusListener, main)
                .build()
                .also { focusRequest = it }
            audio.requestAudioFocus(request)
        } else {
            @Suppress("DEPRECATION")
            audio.requestAudioFocus(
                focusListener,
                AudioManager.STREAM_MUSIC,
                AudioManager.AUDIOFOCUS_GAIN,
            )
        }
        focusHeld = result == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        // Refused, as during a call: the player pauses, as for a loss for good.
        if (!focusHeld) nativeAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS)
    }

    private fun abandonFocus() {
        if (!focusHeld) return
        focusHeld = false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            focusRequest?.let { audio.abandonAudioFocusRequest(it) }
        } else {
            @Suppress("DEPRECATION")
            audio.abandonAudioFocus(focusListener)
        }
    }

    private fun registerNoisy() {
        if (noisyRegistered) return
        val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(noisy, filter, RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(noisy, filter)
        }
        noisyRegistered = true
    }

    private fun unregisterNoisy() {
        if (!noisyRegistered) return
        unregisterReceiver(noisy)
        noisyRegistered = false
    }

    private fun metadata(shown: Shown): MediaMetadata {
        val builder = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, shown.title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, shown.artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, shown.album)
        if (shown.durationMs >= 0) builder.putLong(MediaMetadata.METADATA_KEY_DURATION, shown.durationMs)
        artwork?.let { builder.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) }
        return builder.build()
    }

    private fun playbackState(shown: Shown): PlaybackState {
        val state = when (shown.state) {
            STATE_PLAYING -> PlaybackState.STATE_PLAYING
            STATE_PAUSED -> PlaybackState.STATE_PAUSED
            else -> PlaybackState.STATE_STOPPED
        }
        return PlaybackState.Builder()
            .setActions(ACTIONS)
            .setState(state, shown.positionMs, shown.speed, shown.at)
            .build()
    }

    private fun notification(shown: Shown): Notification {
        val playing = shown.state == STATE_PLAYING
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        return builder
            .setSmallIcon(R.drawable.ic_launcher_monochrome)
            .setLargeIcon(artwork)
            .setContentTitle(shown.title)
            .setContentText(shown.artist)
            .setContentIntent(openApp())
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setShowWhen(false)
            .setOnlyAlertOnce(true)
            .setOngoing(playing)
            .addAction(action(android.R.drawable.ic_media_previous, R.string.previous, ACTION_PREVIOUS))
            .addAction(
                if (playing) {
                    action(android.R.drawable.ic_media_pause, R.string.pause, ACTION_PAUSE)
                } else {
                    action(android.R.drawable.ic_media_play, R.string.play, ACTION_PLAY)
                }
            )
            .addAction(action(android.R.drawable.ic_media_next, R.string.next, ACTION_NEXT))
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(session.sessionToken)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            .build()
    }

    private fun action(icon: Int, title: Int, action: String): Notification.Action {
        val intent = Intent(this, PlaybackService::class.java).setAction(action)
        val pending = PendingIntent.getService(this, 0, intent, PendingIntent.FLAG_IMMUTABLE)
        return Notification.Action.Builder(Icon.createWithResource(this, icon), getString(title), pending)
            .build()
    }

    private fun openApp(): PendingIntent {
        val intent = Intent(this, MainActivity::class.java)
            .setAction(Intent.ACTION_MAIN)
            .addCategory(Intent.CATEGORY_LAUNCHER)
        return PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE)
    }

    /** Loads the cover at [path] on a thread of its own, and shows it if it is still the one. */
    private fun loadArtwork(path: String?) {
        if (path == artworkPath) return
        artworkPath = path
        artwork = null
        if (path == null) return
        thread(name = "playback-artwork") {
            val bitmap = decodeArtwork(path)
            main.post {
                val shown = latest
                if (bitmap != null && !released && artworkPath == path && shown != null) {
                    artwork = bitmap
                    show(shown)
                }
            }
        }
    }

    private fun decodeArtwork(path: String): Bitmap? {
        return try {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(path, bounds)
            var sample = 1
            while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= ARTWORK_SIZE) sample *= 2
            val options = BitmapFactory.Options().apply { inSampleSize = sample }
            val bitmap = BitmapFactory.decodeFile(path, options) ?: return null
            val scale = ARTWORK_SIZE.toFloat() / max(bitmap.width, bitmap.height)
            if (scale >= 1f) {
                bitmap
            } else {
                Bitmap.createScaledBitmap(
                    bitmap,
                    max(1, (bitmap.width * scale).roundToInt()),
                    max(1, (bitmap.height * scale).roundToInt()),
                    true,
                )
            }
        } catch (e: Exception) {
            Log.w(TAG, "Could not load the cover $path: $e")
            null
        }
    }

    private external fun nativeCommand(command: Int, positionMs: Long)

    private external fun nativeAudioFocusChange(change: Int)
}
