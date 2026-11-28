package ci.not.rune

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel

/** Offers a temporary tree selection. Dart checks the open library before persisting it. */
class DocumentTreeRecovery(private val activity: Activity, messenger: BinaryMessenger) {
    private val channel = MethodChannel(messenger, "ci.not.rune/document_tree_recovery")
    private var pending: MethodChannel.Result? = null
    private val accessFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION

    init {
        channel.setMethodCallHandler { call, result ->
            try {
                when (call.method) {
                    "pick" -> {
                        if (pending != null) {
                            result.error("BUSY", "A recovery folder picker is already open", null)
                            return@setMethodCallHandler
                        }
                        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                            .addFlags(accessFlags or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                            intent.putExtra(DocumentsContract.EXTRA_INITIAL_URI, Uri.parse(call.arguments as String))
                        }
                        activity.startActivityForResult(intent, REQUEST_CODE)
                        pending = result
                    }
                    "persist" -> {
                        val uri = Uri.parse(call.argument<String>("uri")!!)
                        val flags = call.argument<Int>("flags")!! and accessFlags
                        require(flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
                        activity.contentResolver.takePersistableUriPermission(uri, flags)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            } catch (error: Exception) {
                result.error("FOLDER_ACCESS", error.message, null)
            }
        }
    }

    fun onResult(resultCode: Int, data: Intent?) {
        val result = pending ?: return
        pending = null
        val uri = data?.data
        if (resultCode == Activity.RESULT_OK && uri != null) {
            result.success(mapOf("uri" to uri.toString(), "flags" to (data.flags and accessFlags)))
        } else {
            result.success(null)
        }
    }

    fun detach() {
        pending?.success(null)
        pending = null
        channel.setMethodCallHandler(null)
    }

    companion object {
        const val REQUEST_CODE = 0x5254
    }
}
