package ci.not.rune

import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity

class MainActivity: FlutterActivity() {

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
}
