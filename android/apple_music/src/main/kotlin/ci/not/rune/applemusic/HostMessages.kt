package ci.not.rune.applemusic

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/** The family the player names this host by. */
internal const val HOST_FAMILY = "appleMusic"

/** One action of a batch, as `HostAction` in `playback/src/hosted.rs`. */
internal sealed interface HostAction {
    data class Set(val ids: List<String>) : HostAction
    data class Add(val ids: List<String>) : HostAction
    data class Remove(val index: Int) : HostAction
    data object Clear : HostAction
    data class Skip(val index: Int, val offsetMs: Long) : HostAction
    data object Start : HostAction
    data object Stop : HostAction
    data class SetGain(val gain: Double) : HostAction

    /** An action this version does not know, which is ignored. */
    data object Unknown : HostAction
}

internal data class HostBatch(val revision: Long, val actions: List<HostAction>)

/** Why an entry cannot be played, as `HostFailure` in `playback/src/hosted.rs`. */
internal enum class Failure(val wire: String) {
    /** The service does not have the song. */
    UNPLAYABLE("unplayable"),

    /** The account may not play it. */
    NOT_AUTHORIZED("notAuthorized"),

    /** The song could not be reached, for whatever other reason. */
    NETWORK("network"),
}

/** What the host reports about its queue. Every event carries the latest batch's revision. */
internal sealed interface HostEvent {
    data class Status(
        val revision: Long,
        val currentIndex: Int?,
        val currentId: String?,
        val playing: Boolean,
        val positionMs: Long,
    ) : HostEvent

    data class ItemFailed(val revision: Long, val index: Int, val reason: Failure) : HostEvent

    data class Ended(val revision: Long, val index: Int?) : HostEvent
}

/** Reads the player's batches and writes the host's events, as JSON in camelCase. */
internal object HostMessages {
    /** Reads a batch, or null when it does not parse. Actions of unknown kinds are kept as such. */
    fun parseBatch(json: String): HostBatch? = try {
        val batch = JSONObject(json)
        val actions = batch.getJSONArray("actions")
        HostBatch(
            revision = batch.getLong("revision"),
            actions = (0 until actions.length()).map { action(actions.getJSONObject(it)) },
        )
    } catch (error: JSONException) {
        null
    }

    private fun action(action: JSONObject): HostAction = when (action.optString("action")) {
        "set" -> HostAction.Set(ids(action))
        "add" -> HostAction.Add(ids(action))
        "remove" -> HostAction.Remove(action.getInt("index"))
        "clear" -> HostAction.Clear
        "skip" -> HostAction.Skip(
            action.getInt("index"),
            action.optDouble("offsetMs", 0.0).coerceAtLeast(0.0).toLong(),
        )
        "start" -> HostAction.Start
        "stop" -> HostAction.Stop
        "setGain" -> HostAction.SetGain(action.optDouble("gain", 1.0))
        else -> HostAction.Unknown
    }

    private fun ids(action: JSONObject): List<String> {
        val ids = action.getJSONArray("ids")
        return (0 until ids.length()).map { ids.getString(it) }
    }

    fun encode(event: HostEvent): String = when (event) {
        is HostEvent.Status -> JSONObject()
            .put("type", "jukeboxStatus")
            .put("host", HOST_FAMILY)
            .put("revision", event.revision)
            .put("currentIndex", event.currentIndex ?: JSONObject.NULL)
            .put("currentId", event.currentId ?: JSONObject.NULL)
            .put("playing", event.playing)
            .put("positionMs", event.positionMs)
        is HostEvent.ItemFailed -> JSONObject()
            .put("type", "jukeboxItemFailed")
            .put("host", HOST_FAMILY)
            .put("revision", event.revision)
            .put("index", event.index)
            .put("reason", event.reason.wire)
        is HostEvent.Ended -> JSONObject()
            .put("type", "jukeboxEnded")
            .put("host", HOST_FAMILY)
            .put("revision", event.revision)
            .apply { event.index?.let { put("index", it) } }
    }.toString()

    /**
     * The `hostReady` event, which a host sends when it starts and again whenever what it can do
     * changes. Every field it carries is set here.
     */
    fun ready(authorized: Boolean, canPlay: Boolean): String = JSONObject()
        .put("type", "hostReady")
        .put("host", HOST_FAMILY)
        .put("authorized", authorized)
        .put("canPlay", canPlay)
        .toString()

    /**
     * Whether the device is online. Offline it names the songs stored on the device, of which
     * there are none: MusicKit for Android only streams.
     */
    fun networkState(online: Boolean): String = JSONObject()
        .put("type", "networkState")
        .put("host", HOST_FAMILY)
        .put("online", online)
        .apply { if (!online) put("downloaded", JSONArray()) }
        .toString()
}
