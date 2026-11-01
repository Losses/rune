package ci.not.rune

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Log
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File
import kotlin.concurrent.thread

class MainActivity: FlutterActivity() {
    private var popChannel: MethodChannel? = null
    private var sceneChannel: MethodChannel? = null

    // Scene file paths received before Dart called takePending.
    private val pendingSceneFiles = mutableListOf<String>()
    private var dartListening = false

    companion object {
        init {
            System.loadLibrary("hub")  // load Rust library
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        initializeContext(this)
    }

    override fun onDestroy() {
        releaseContext()
        super.onDestroy()
    }

    private external fun initializeContext(context: Any)
    private external fun releaseContext()

    /** Hands the engine's texture registry to the hub, which shows the visualizer in a texture. */
    private external fun attachFlutter(registry: Any): Boolean

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        if (!attachFlutter(flutterEngine.renderer)) {
            Log.e("Visualizer", "The hub could not take the texture registry")
        }
        popChannel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ci.not.rune/pop")

        // Scene files: paths queue until Dart has called takePending once, then they are sent
        // with "open".
        sceneChannel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ci.not.rune/scene_files")
        sceneChannel?.setMethodCallHandler { call, result ->
            if (call.method == "takePending") {
                val paths = ArrayList(pendingSceneFiles)
                pendingSceneFiles.clear()
                dartListening = true
                result.success(paths)
            } else {
                result.notImplemented()
            }
        }
        // The file that launched the app.
        handleSceneIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleSceneIntent(intent)
    }

    /**
     * Copies the file a VIEW or SEND intent points at into the cache and passes its path to Dart.
     * The content is not checked here: the MIME types of the intent filters are broad and content
     * URIs often have no extension, so the importer validates the archive.
     */
    private fun handleSceneIntent(intent: Intent?) {
        if (intent == null || intent.getBooleanExtra("sceneHandled", false)) return
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        val uri: Uri? = when (intent.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
            else -> null
        }
        if (uri == null) return
        // The same intent is handed back when the activity is recreated, import it only once.
        intent.putExtra("sceneHandled", true)

        thread {
            val path = copyToCache(uri)
            if (path != null) {
                runOnUiThread { deliverSceneFile(path) }
            }
        }
    }

    private fun deliverSceneFile(path: String) {
        val channel = sceneChannel
        if (dartListening && channel != null) {
            channel.invokeMethod("open", listOf(path))
        } else {
            pendingSceneFiles.add(path)
        }
    }

    private fun copyToCache(uri: Uri): String? {
        return try {
            var name: String? = null
            if (uri.scheme == "content") {
                contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) name = it.getString(0)
                }
            } else {
                name = uri.lastPathSegment
            }
            // Only the last path component, so that a name can not escape the folder.
            val fileName = File(name ?: "").name.ifEmpty { "scene.runescene" }
            val folder = File(cacheDir, "scene-imports")
            folder.mkdirs()
            val target = File(folder, fileName)
            val input = contentResolver.openInputStream(uri) ?: return null
            input.use { source -> target.outputStream().use { source.copyTo(it) } }
            target.absolutePath
        } catch (e: Exception) {
            Log.e("SceneFiles", "Could not copy the scene file: $e")
            null
        }
    }

    /**
     * Though this method is marked as deprecated, it is still called by the system. And to avoid
     * Flutter killing entire app when back button is pressed, we override this method and handle
     * the back button press event here.
     */
    @Deprecated("Deprecated in newer Android versions for predictive back feature.")
    override fun onBackPressed() {
        // Check if the Flutter side wants to handle the back button press event
        popChannel?.invokeMethod("pop", null, object: MethodChannel.Result {
            override fun success(result: Any?) {
                if (result is Boolean) {
                    // true: Flutter side handled the back button press event, popping the route
                    // false: Flutter side did not handle, let's go to background
                    if (result) return else moveTaskToBack(true)
                }
            }

            override fun error(errorCode: String, errorMessage: String?, errorDetails: Any?) {
                Log.e("Pop", "Error: $errorCode, $errorMessage, $errorDetails")
            }

            override fun notImplemented() {
                Log.e("Pop", "Not implemented")
            }
        })
    }
}
