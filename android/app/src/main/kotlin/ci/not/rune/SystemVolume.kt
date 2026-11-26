package ci.not.rune

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/**
 * The system's music volume, for Rune's volume control while a track plays on a platform host
 * that has no volume of its own.
 *
 * Dart reads it with `volume`, whose answer is `{volume, settable}`, sets it with `setVolume`
 * (without the system's volume panel: Rune's slider is the control) and gets `changed` calls with
 * the same map. The volume is the music stream's index scaled over its range, so that its
 * minimum, which is not 0 on some devices, reads as 0.
 *
 * Changes come from the `VOLUME_CHANGED_ACTION` broadcast, which the audio service sends the
 * moment a stream's index changes, whether a hardware key, another app or Rune did it. A
 * `ContentObserver` on the system settings only fires once the value is stored, which some
 * devices do late or per output, and misses a change that is not stored. An output change can
 * move the volume without that broadcast, so an `AudioDeviceCallback` reports it too.
 */
class SystemVolume(private val context: Context) {
    private val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val handler = Handler(Looper.getMainLooper())
    private var channel: MethodChannel? = null

    /** What `changed` last told Dart, so that only a change is reported. */
    private var reported: Map<String, Any>? = null

    // Not in the public SDK, but the broadcast every volume UI and app follows.
    private val volumeAction = "android.media.VOLUME_CHANGED_ACTION"
    private val streamExtra = "android.media.EXTRA_VOLUME_STREAM_TYPE"

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.getIntExtra(streamExtra, -1) == AudioManager.STREAM_MUSIC) report()
        }
    }

    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = report()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = report()
    }

    /** Creates the channel on the engine's messenger and starts following the system volume. */
    fun attach(messenger: BinaryMessenger) {
        val channel = MethodChannel(messenger, "ci.not.rune/system_volume")
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "volume" -> result.success(state())
                "setVolume" -> {
                    val volume = (call.arguments as? Number)?.toDouble()
                    if (volume == null) {
                        result.error("bad_arguments", "A volume is needed.", null)
                    } else if (setVolume(volume.coerceIn(0.0, 1.0))) {
                        result.success(null)
                    } else {
                        result.error("not_settable", "The output's volume cannot be set.", null)
                    }
                }
                else -> result.notImplemented()
            }
        }
        this.channel = channel
        reported = null
        val filter = IntentFilter(volumeAction)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(receiver, filter)
        }
        audio.registerAudioDeviceCallback(deviceCallback, handler)
    }

    /** Stops following the system volume and closes the channel. */
    fun detach() {
        context.unregisterReceiver(receiver)
        audio.unregisterAudioDeviceCallback(deviceCallback)
        channel?.setMethodCallHandler(null)
        channel = null
    }

    private fun minIndex() =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            audio.getStreamMinVolume(AudioManager.STREAM_MUSIC)
        } else {
            0
        }

    private fun state(): Map<String, Any> {
        val min = minIndex()
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        val index = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
        val volume = if (max > min) (index - min).toDouble() / (max - min) else 1.0
        return mapOf("volume" to volume.coerceIn(0.0, 1.0), "settable" to !audio.isVolumeFixed)
    }

    private fun setVolume(volume: Double): Boolean {
        if (audio.isVolumeFixed) return false
        val min = minIndex()
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        return try {
            // Flags 0: no system volume panel.
            audio.setStreamVolume(
                AudioManager.STREAM_MUSIC,
                (min + Math.round(volume * (max - min))).toInt(),
                0
            )
            true
        } catch (e: SecurityException) {
            false
        }
    }

    /** Tells Dart the volume if it differs from what Dart was told last. */
    private fun report() {
        val current = state()
        if (current == reported) return
        reported = current
        channel?.invokeMethod("changed", current)
    }
}
