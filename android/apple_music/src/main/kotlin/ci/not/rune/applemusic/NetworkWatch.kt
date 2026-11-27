package ci.not.rune.applemusic

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Whether the device has a network that reaches the internet, as the system's default network
 * says. Until the system says, and without the permission to ask, the device counts as online.
 * [changed] runs on the main thread whenever that changes.
 */
internal class NetworkWatch(context: Context, private val changed: () -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var started = false

    var online = true
        private set

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            val reaches = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            main.post { update(reaches) }
        }

        override fun onLost(network: Network) {
            main.post { update(false) }
        }
    }

    /** Starts watching, once. */
    fun start() {
        if (started) return
        started = true
        try {
            // Registered first, so that no change after the state was read goes unseen. Without a
            // network the callback says nothing at first.
            connectivity.registerDefaultNetworkCallback(callback)
            online = connectivity.activeNetwork
                ?.let(connectivity::getNetworkCapabilities)
                ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
        } catch (error: SecurityException) {
            online = true
            Log.w("AppleMusicHost", "The network cannot be watched: ${error.javaClass.simpleName}")
        }
    }

    private fun update(reaches: Boolean) {
        if (reaches == online) return
        online = reaches
        changed()
    }
}
