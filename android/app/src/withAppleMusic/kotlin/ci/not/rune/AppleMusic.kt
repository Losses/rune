package ci.not.rune

import android.content.Intent
import ci.not.rune.applemusic.AppleMusicHost
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.PluginRegistry

/** This build includes Apple Music, from the `apple_music` module. */
internal fun appleMusicPlugin(): FlutterPlugin? = AppleMusicPlugin()

/**
 * Connects the process's [AppleMusicHost] to Dart on `ci.not.rune/apple_music`, as the macOS and
 * iOS runners do: `status`, `authorize` and `batch` go to the host, and its events come back as
 * `event` calls. `available` tells Dart whether this build has the host and it can play in this
 * process.
 */
private class AppleMusicPlugin :
    FlutterPlugin,
    ActivityAware,
    MethodChannel.MethodCallHandler,
    PluginRegistry.ActivityResultListener {
    private var channel: MethodChannel? = null
    private var host: AppleMusicHost? = null
    private var activity: ActivityPluginBinding? = null
    private val sink: (String) -> Unit = { json -> channel?.invokeMethod("event", json) }

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        host = AppleMusicHost.shared(binding.applicationContext).also { it.connect(sink) }
        channel = MethodChannel(binding.binaryMessenger, "ci.not.rune/apple_music").also {
            it.setMethodCallHandler(this)
        }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        host?.disconnect(sink)
        channel?.setMethodCallHandler(null)
        channel = null
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val host = host ?: return result.notImplemented()
        when (call.method) {
            "available" -> result.success(AppleMusicHost.supported)
            "status" -> host.status { result.success(it) }
            "authorize" -> {
                val interactive = call.argument<Boolean>("interactive") ?: false
                host.authorize(activity?.activity, interactive) { reply ->
                    if (reply != null) {
                        result.success(reply)
                    } else {
                        result.error("unavailable", "Apple Music cannot play in this build", null)
                    }
                }
            }
            "batch" -> {
                val json = call.arguments as? String
                    ?: return result.error("bad_arguments", "A batch is JSON text", null)
                host.batch(json)
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        activity = binding
        binding.addActivityResultListener(this)
    }

    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) =
        onAttachedToActivity(binding)

    override fun onDetachedFromActivityForConfigChanges() = onDetachedFromActivity()

    override fun onDetachedFromActivity() {
        activity?.removeActivityResultListener(this)
        activity = null
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean =
        host?.onActivityResult(requestCode, resultCode, data) ?: false
}
