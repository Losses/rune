package ci.not.rune.applemusic

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.apple.android.music.playback.controller.MediaPlayerController
import com.apple.android.music.playback.controller.MediaPlayerControllerFactory
import com.apple.android.music.playback.model.MediaItemType
import com.apple.android.music.playback.model.MediaPlayerException
import com.apple.android.music.playback.model.PlaybackRepeatMode
import com.apple.android.music.playback.model.PlaybackShuffleMode
import com.apple.android.music.playback.model.PlaybackState
import com.apple.android.music.playback.model.PlayerQueueItem
import com.apple.android.music.playback.queue.CatalogPlaybackQueueItemProvider
import com.apple.android.music.playback.queue.LibraryPlaybackQueueItemProvider
import com.apple.android.music.playback.queue.PlaybackModeContainer
import com.apple.android.music.playback.queue.PlaybackQueueInsertionType
import com.apple.android.music.playback.queue.PlaybackQueueItemProvider
import com.apple.android.music.playback.queue.PlaybackQueueItemsAdded
import com.apple.android.sdk.authentication.TokenProvider
import java.io.IOException

/**
 * MusicKit's player as a [QueuePlayer].
 *
 * Its controller is made on the first queue change or ahead of it ([makeController]), once the
 * SDK's native libraries are loaded, and kept for the life of the process: MusicKit shares state
 * across the process, and a controller released while another is made crashes. Until then the
 * player holds nothing, and every other call does nothing, while a queue change and the calls
 * after it wait for the controller. It calls back on the main thread, where the host runs.
 *
 * MusicKit requests and gives up audio focus by itself, and makes no media session: Rune's
 * playback service has those for every track.
 */
internal class MusicKitPlayer(
    private val context: Context,
    private val tokens: TokenProvider,
) : QueuePlayer, MediaPlayerController.Listener {
    private companion object {
        const val TAG = "AppleMusicHost"
    }

    override var listener: QueuePlayer.Listener? = null

    private val main = Handler(Looper.getMainLooper())

    /**
     * The controller, made on a thread of its own, which takes seconds: the main thread, where the
     * host runs, never waits for it. A call that needs it before it exists runs once it does.
     * A controller that could not be made leaves the player without one, and the next queue
     * change tries again.
     */
    private val controller = LateValue<MediaPlayerController> { finished ->
        Thread({
            val made = try {
                MediaPlayerControllerFactory.createLocalController(
                    context,
                    Handler(Looper.getMainLooper()),
                    tokens,
                )
            } catch (error: Throwable) {
                Log.w(TAG, "MusicKit's player could not be made: ${error.javaClass.simpleName}")
                null
            }
            main.post {
                made?.let(::configure)
                finished(made)
            }
        }, "MusicKitPlayer").start()
    }

    /** The player keeps the play order and its modes, so MusicKit plays the queue as it is. */
    private fun configure(made: MediaPlayerController) {
        made.addListener(this)
        made.setRepeatMode(PlaybackRepeatMode.REPEAT_MODE_OFF)
        made.setShuffleMode(PlaybackShuffleMode.SHUFFLE_MODE_OFF)
    }

    /**
     * Starts making the controller unless it exists, which the host does once it is likely to
     * play, so that the first track finds it made.
     */
    fun makeController() {
        main.post { controller.make() }
    }

    /**
     * Tells MusicKit that the developer token or the user's token changed, or that the user's was
     * forgotten. MusicKit keeps its own copy of the tokens and reads them from [tokens] again only
     * when told so.
     */
    fun tokensChanged() {
        controller.value?.playbackStoreConfiguration?.notifyTokenChanged(context, tokens)
    }

    override val state: PlayerState
        get() = when (controller.value?.playbackState) {
            PlaybackState.PLAYING -> PlayerState.PLAYING
            PlaybackState.PAUSED -> PlayerState.PAUSED
            else -> PlayerState.STOPPED
        }

    override val positionMs: Long get() = controller.value?.currentPosition ?: -1

    override val bufferedMs: Long get() = controller.value?.bufferedPosition ?: 0

    override val current: QueueItem? get() = controller.value?.currentItem?.let(::queueItem)

    override val upcoming: List<QueueItem>
        get() = controller.value?.queueItems?.map(::queueItem) ?: emptyList()

    override fun replace(id: String) = queueChange {
        // A plain prepare refuses to replace a queue that songs were added to.
        it.prepare(
            provider(listOf(id)),
            PlaybackQueueInsertionType.INSERTION_TYPE_CLEAR_AND_REPLACE,
            false,
        )
    }

    override fun append(ids: List<String>) = queueChange {
        it.addQueueItems(provider(ids), PlaybackQueueInsertionType.INSERTION_TYPE_AT_END)
    }

    /**
     * A queue change, which makes the controller if need be. When that fails, the change is
     * answered with an error, as the host expects an answer to each.
     */
    private fun queueChange(work: (MediaPlayerController) -> Unit) =
        controller.call(creates = true, abandoned = { listener?.onError(PlayerError.QUEUE) }, work)

    override fun remove(queueId: Long) = controller.call { it.removeQueueItemWithId(queueId) }

    override fun play() = controller.call { it.play() }

    override fun pause() = controller.call { it.pause() }

    override fun stop() = controller.call { it.stop() }

    override fun seek(positionMs: Long) = controller.call { it.seekToPosition(positionMs) }

    override fun skipToNext() = controller.call { it.skipToNextItem() }

    /** The provider of [ids], which are all catalogue songs or all library songs. */
    private fun provider(ids: List<String>): PlaybackQueueItemProvider {
        val songs = ids.map(::storeId).toTypedArray()
        return if (isCatalogId(ids.first())) {
            CatalogPlaybackQueueItemProvider.Builder()
                .items(MediaItemType.SONG, *songs)
                .startItemIndex(0)
                .shuffleMode(PlaybackShuffleMode.SHUFFLE_MODE_OFF)
                .featureName("rune")
                .build()
        } else {
            LibraryPlaybackQueueItemProvider.Builder()
                .items(MediaItemType.SONG, *songs)
                .startItemIndex(0)
                .shuffleMode(PlaybackShuffleMode.SHUFFLE_MODE_OFF)
                .featureName("rune")
                .build()
        }
    }

    private fun queueItem(item: PlayerQueueItem) = QueueItem(
        queueId = item.playbackQueueId,
        storeId = item.item?.playbackStoreId ?: "",
        durationMs = item.item?.duration ?: 0,
    )

    // MediaPlayerController.Listener

    override fun onPlaybackStateChanged(
        source: MediaPlayerController,
        previous: Int,
        current: Int,
    ) {
        val state = when (current) {
            PlaybackState.PLAYING -> PlayerState.PLAYING
            PlaybackState.PAUSED -> PlayerState.PAUSED
            else -> PlayerState.STOPPED
        }
        Log.d(TAG, "MusicKit $state at ${source.currentPosition} ms")
        listener?.onStateChanged(state)
    }

    override fun onCurrentItemChanged(
        source: MediaPlayerController,
        previous: PlayerQueueItem?,
        current: PlayerQueueItem?,
    ) {
        val item = current?.let(::queueItem)
        Log.d(TAG, "MusicKit plays $item")
        listener?.onCurrentItemChanged(item)
    }

    override fun onItemEnded(source: MediaPlayerController, item: PlayerQueueItem, position: Long) {
        val ended = queueItem(item)
        Log.d(TAG, "MusicKit left $ended at $position ms")
        listener?.onItemEnded(ended, position)
    }

    override fun onPlaybackQueueChanged(
        source: MediaPlayerController,
        items: List<PlayerQueueItem>,
    ) {
        Log.d(TAG, "MusicKit queues ${items.map { it.playbackQueueId }} after the current item")
        listener?.onQueueChanged()
    }

    override fun onPlaybackQueueItemsAdded(
        source: MediaPlayerController,
        added: PlaybackQueueItemsAdded,
    ) {
        listener?.onItemsAdded()
    }

    override fun onPlaybackError(source: MediaPlayerController, error: MediaPlayerException) {
        // The type and the class of the cause only: messages may carry request details.
        Log.w(TAG, "MusicKit failed: type ${error.type}, ${error.cause?.javaClass?.simpleName}")
        listener?.onError(
            when (error.type) {
                MediaPlayerException.TYPE_PLAYBACK_QUEUE ->
                    if (findsNoSong(error)) PlayerError.MISSING else PlayerError.QUEUE
                MediaPlayerException.TYPE_MUSIC_USER_TOKEN_ERROR -> PlayerError.TOKEN
                MediaPlayerException.TYPE_USER_SUBSCRIPTION_INVALID -> PlayerError.SUBSCRIPTION
                MediaPlayerException.TYPE_EXPLICIT_VERIFY_AGE -> PlayerError.RESTRICTED
                MediaPlayerException.TYPE_IO -> PlayerError.IO
                MediaPlayerException.TYPE_DRM, MediaPlayerException.TYPE_PLAYBACK_LEASE ->
                    PlayerError.LICENSE
                else -> PlayerError.OTHER
            },
        )
    }

    /**
     * Whether a queue change failed as the service has no such song: MusicKit then says that the
     * catalogue's or the library's data response is null.
     */
    private fun findsNoSong(error: MediaPlayerException): Boolean {
        val cause = error.cause
        return cause is IOException && cause.message?.endsWith("Data Response is null") == true
    }

    override fun onPlayerStateRestored(source: MediaPlayerController) = Unit

    override fun onPlaybackStateUpdated(source: MediaPlayerController) = Unit

    override fun onBufferingStateChanged(source: MediaPlayerController, buffering: Boolean) {
        listener?.onBufferingChanged()
    }

    override fun onMetadataUpdated(source: MediaPlayerController, item: PlayerQueueItem) = Unit

    override fun onPlaybackModeChanged(
        source: MediaPlayerController,
        mode: PlaybackModeContainer?,
    ) = Unit

    override fun onPlaybackRepeatModeChanged(source: MediaPlayerController, mode: Int) = Unit

    override fun onPlaybackShuffleModeChanged(source: MediaPlayerController, mode: Int) = Unit
}
