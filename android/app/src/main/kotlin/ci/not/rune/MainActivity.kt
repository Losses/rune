package ci.not.rune

import android.os.Bundle
import android.util.Log
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity: FlutterActivity() {
    private var popChannel: MethodChannel? = null

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

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        popChannel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ci.not.rune/pop")
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
