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
 * Its controller is made on the first queue change, once the SDK's native libraries are loaded,
 * and kept for the life of the process: MusicKit shares state across the process, and a
 * controller released while another is made crashes. Until then the player holds nothing, and
 * every other call does nothing. It calls back on the main thread, where the host runs.
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

    @Volatile
    private var controller: MediaPlayerController? = null

    /**
     * Makes the controller unless it exists, which takes seconds: the host does it on a thread of
     * its own once it is likely to play, so that the main thread does not wait for it then.
     */
    fun makeController() {
        controller()
    }

    private fun controller(): MediaPlayerController = controller ?: synchronized(this) {
        controller ?: MediaPlayerControllerFactory.createLocalController(
            context,
            Handler(Looper.getMainLooper()),
            tokens,
        ).also {
            it.addListener(this)
            // The player keeps the play order and its modes, so MusicKit plays the queue as it is.
            it.setRepeatMode(PlaybackRepeatMode.REPEAT_MODE_OFF)
            it.setShuffleMode(PlaybackShuffleMode.SHUFFLE_MODE_OFF)
            controller = it
        }
    }

    /**
     * Tells MusicKit that the developer token or the user's token changed, or that the user's was
     * forgotten. MusicKit keeps its own copy of the tokens and reads them from [tokens] again only
     * when told so.
     */
    fun tokensChanged() {
        controller?.playbackStoreConfiguration?.notifyTokenChanged(context, tokens)
    }

    override val state: PlayerState
        get() = when (controller?.playbackState) {
            PlaybackState.PLAYING -> PlayerState.PLAYING
            PlaybackState.PAUSED -> PlayerState.PAUSED
            else -> PlayerState.STOPPED
        }

    override val positionMs: Long get() = controller?.currentPosition ?: -1

    override val bufferedMs: Long get() = controller?.bufferedPosition ?: 0

    override val current: QueueItem? get() = controller?.currentItem?.let(::queueItem)

    override val upcoming: List<QueueItem>
        get() = controller?.queueItems?.map(::queueItem) ?: emptyList()

    override fun replace(id: String) {
        // A plain prepare refuses to replace a queue that songs were added to.
        controller().prepare(
            provider(listOf(id)),
            PlaybackQueueInsertionType.INSERTION_TYPE_CLEAR_AND_REPLACE,
            false,
        )
    }

    override fun append(ids: List<String>) {
        controller().addQueueItems(provider(ids), PlaybackQueueInsertionType.INSERTION_TYPE_AT_END)
    }

    override fun remove(queueId: Long) {
        controller?.removeQueueItemWithId(queueId)
    }

    override fun play() {
        controller?.play()
    }

    override fun pause() {
        controller?.pause()
    }

    override fun stop() {
        controller?.stop()
    }

    override fun seek(positionMs: Long) {
        controller?.seekToPosition(positionMs)
    }

    override fun skipToNext() {
        controller?.skipToNextItem()
    }

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
