import 'request_failure.dart';
import '../dialogs/information/error.dart';
import '../l10n.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../utils/dialogs/no_analysis/show_no_analysis_dialog.dart';
import '../../bindings/bindings.dart';
import '../playing_item.dart';

/// A load that leaves the player's playback mode alone when [playbackMode] is
/// `null` and decides by itself whether to start playing.
LoadResolvedItemsRequest loadResolvedItemsRequest({
  required List<PlayingItemRef> items,
  required int? startIndex,
  required PlaylistOperateMode operateMode,
  required Uint64? expectedRevision,
  required PlaybackMode? playbackMode,
}) {
  return LoadResolvedItemsRequest(
    items: items,
    startIndex: startIndex,
    mode: operateMode,
    expectedRevision: expectedRevision,
    playbackMode: playbackMode,
    positionSeconds: null,
    play: LoadPlay.auto,
  );
}

/// The tracks of the player the UI shows that a mix should leave out: its whole
/// queue, unless the result is cast to another renderer.
List<PlayingItemRef> queueItemsToExclude({
  required CastTarget? castTo,
  required List<PlaylistItem>? queue,
}) {
  if (castTo != null) return const [];
  return [for (final entry in queue ?? const <PlaylistItem>[]) entry.item];
}

/// Whether a mix played with [operateMode] leaves out the tracks already in the
/// queue. A replaced queue goes as a whole, so playing an album or a track that
/// it holds keeps them; adding to the queue, or a recommendation mix, leaves
/// them out.
bool mixExcludesQueue({
  required PlaylistOperateMode operateMode,
  required QueryList queries,
}) =>
    operateMode != PlaylistOperateMode.replace ||
    queriesHasRecommendation(queries);

/// The items a playback action has converted for its resolve request: the
/// track to start with, when it names one, then its fallback tracks. An action
/// that names no track to start with has a `null` [initialPlaybackId], and no
/// catalog row stands in for one.
List<PlayingItem> playbackItemsToResolve({
  required int? initialPlaybackId,
  required List<PlayingItem> fallbackPlayingItems,
}) => [
  if (initialPlaybackId != null) PlayingItem.inLibrary(initialPlaybackId),
  ...fallbackPlayingItems,
];

/// The track a playback action starts with and its fallback tracks, as a
/// resolve request carries them. Without a start track the request has no
/// initial item.
Future<({PlayingItemRef? initial, List<PlayingItemRef> fallback})>
playbackStartItems({
  required int? initialPlaybackId,
  required List<PlayingItem> fallbackPlayingItems,
}) async {
  final refs = await playingItemRefs(
    playbackItemsToResolve(
      initialPlaybackId: initialPlaybackId,
      fallbackPlayingItems: fallbackPlayingItems,
    ),
  );
  if (initialPlaybackId == null) return (initial: null, fallback: refs);
  return (initial: refs.first, fallback: refs.sublist(1));
}

/// Starts or queues what [queries] resolve to. [initialPlaybackId] is the
/// catalog row of the track to start with, or `null` to start where
/// [hintPosition] says.
Future<List<PlayingItem>> operatePlaybackWithMixQuery({
  required QueryList queries,
  required PlaybackMode? playbackMode,
  required int hintPosition,
  required int? initialPlaybackId,
  required bool instantlyPlay,
  required PlaylistOperateMode operateMode,
  required List<PlayingItem> fallbackPlayingItems,
  CastTarget? castTo,
  void Function(ResolvedItemsLoadResult result)? onCastResult,
}) async {
  final start = await playbackStartItems(
    initialPlaybackId: initialPlaybackId,
    fallbackPlayingItems: fallbackPlayingItems,
  );
  final resolvedSignal = await awaitResponse(
    'ResolveMixQueryRequest',
    ResolveMixQueryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ResolveMixQueryRequest(
      queries: queries.toQueryList(),
      hintPosition: hintPosition,
      initialPlaybackItem: start.initial,
      instantlyPlay: instantlyPlay,
      fallbackPlayingItems: start.fallback,
      excluded: ExcludedItems(
        items: mixExcludesQueue(operateMode: operateMode, queries: queries)
            ? queueItemsToExclude(
                castTo: castTo,
                queue: PlaylistUpdate.latestRustSignal?.message.items,
              )
            : const [],
      ),
      callId: callId,
    ).sendSignalToRust(),
  );
  final resolved = resolvedSignal.message;
  final items = resolved.playingItems;
  final startIndex = resolved.startIndex;

  final previousRenderer =
      RendererAttachmentChanged.latestRustSignal?.message.renderer?.fingerprint;
  final localWasPlaying =
      PlaybackStatus.latestRustSignal?.message.state == PlaybackState.playing;
  var attachedForLoad = false;
  if (castTo != null && castTo.attach) {
    await awaitResponse(
      'AttachRendererRequest',
      AttachRendererResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => AttachRendererRequest(
        fingerprint: castTo.fingerprint,
        callId: callId,
      ).sendSignalToRust(),
    );
    attachedForLoad = true;
  }

  try {
    final expectedRevision = castTo != null && !castTo.attach
        ? (await awaitResponse(
            'RemotePlaylistRevisionRequest',
            RemotePlaylistRevisionResponse.rustSignalStream,
            responseCallId: (response) => response.message.callId?.toInt(),
            send: (callId) => RemotePlaylistRevisionRequest(
              fingerprint: castTo.fingerprint,
              callId: callId,
            ).sendSignalToRust(),
          )).message.revision
        : (await awaitResponse(
            'PlaylistRevisionRequest',
            PlaylistRevisionResponse.rustSignalStream,
            responseCallId: (response) => response.message.callId?.toInt(),
            send: (callId) =>
                PlaylistRevisionRequest(callId: callId).sendSignalToRust(),
          )).message.revision;
    final load = loadResolvedItemsRequest(
      items: items,
      startIndex: startIndex,
      operateMode: operateMode,
      expectedRevision: expectedRevision,
      playbackMode: instantlyPlay ? playbackMode : null,
    );
    if (castTo != null && !castTo.attach) {
      final response = await awaitResponse(
        'CastResolvedItemsRequest',
        CastResolvedItemsResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => CastResolvedItemsRequest(
          fingerprint: castTo.fingerprint,
          load: load,
          callId: callId,
        ).sendSignalToRust(),
      );
      onCastResult?.call(response.message.result);
    } else {
      final response = await awaitResponse(
        'LoadResolvedItemsRequest',
        LoadResolvedItemsResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            load.copyWith(callId: () => callId).sendSignalToRust(),
      );
      onCastResult?.call(response.message.result);
    }
  } catch (_) {
    if (attachedForLoad) {
      await restoreCastAttachmentAfterLoadFailure(
        previousRenderer: previousRenderer,
        wasPlaying: localWasPlaying,
        attach: (fingerprint) async {
          await awaitResponse(
            'AttachRendererRequest',
            AttachRendererResponse.rustSignalStream,
            responseCallId: (response) => response.message.callId?.toInt(),
            send: (callId) => AttachRendererRequest(
              fingerprint: fingerprint,
              callId: callId,
            ).sendSignalToRust(),
          );
        },
        detach: () async {
          DetachRendererRequest().sendSignalToRust();
          await Future<void>.delayed(Duration.zero);
        },
        resume: () => PlayRequest().sendSignalToRust(),
      );
    }
    rethrow;
  }

  return items.map(PlayingItem.fromRequest).toList();
}

Future<void> restoreCastAttachmentAfterLoadFailure({
  required String? previousRenderer,
  required bool wasPlaying,
  required Future<void> Function(String fingerprint) attach,
  required Future<void> Function() detach,
  required void Function() resume,
}) async {
  try {
    if (previousRenderer case final fingerprint?) {
      await attach(fingerprint);
    } else {
      await detach();
    }
    if (wasPlaying) resume();
  } catch (_) {
    // Keep the original load failure as the user's error.
  }
}

Future<List<PlayingItem>> safeOperatePlaybackWithMixQuery({
  required BuildContext context,
  required QueryList queries,
  required PlaybackMode? playbackMode,
  required int hintPosition,
  required int? initialPlaybackId,
  required bool instantlyPlay,
  required PlaylistOperateMode operateMode,
  required List<PlayingItem> fallbackPlayingItems,
  CastTarget? castTo,
}) async {
  final hasRecommendation = queriesHasRecommendation(queries);

  late final List<PlayingItem> result;
  try {
    result = await operatePlaybackWithMixQuery(
      queries: queries,
      playbackMode: playbackMode,
      hintPosition: hintPosition,
      initialPlaybackId: initialPlaybackId,
      instantlyPlay: instantlyPlay,
      operateMode: operateMode,
      fallbackPlayingItems: fallbackPlayingItems,
      castTo: castTo,
    );
  } on RequestFailure catch (error) {
    if (context.mounted) {
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: requestFailureMessage(S.of(context), error),
      );
    }
    return [];
  }

  if (result.isEmpty && hasRecommendation && context.mounted) {
    await showNoAnalysisDialog(context, true);
  }

  return result;
}
