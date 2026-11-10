import 'request_failure.dart';
import '../dialogs/information/error.dart';
import '../l10n.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../utils/dialogs/no_analysis/show_no_analysis_dialog.dart';
import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<List<PlayingItem>> operatePlaybackWithMixQuery({
  required QueryList queries,
  required int playbackMode,
  required int hintPosition,
  required int initialPlaybackId,
  required bool instantlyPlay,
  required PlaylistOperateMode operateMode,
  required List<PlayingItem> fallbackPlayingItems,
  CastTarget? castTo,
}) async {
  final resolvedSignal = await awaitResponse(
    'ResolveMixQueryRequest',
    ResolveMixQueryResponse.rustSignalStream,
    send: () => ResolveMixQueryRequest(
      queries: queries.toQueryList(),
      hintPosition: hintPosition,
      initialPlaybackItem: PlayingItem.inLibrary(initialPlaybackId).toRequest(),
      instantlyPlay: instantlyPlay,
      fallbackPlayingItems: fallbackPlayingItems
          .map((item) => item.toRequest())
          .toList(),
      excludeLocalQueue: castTo == null,
    ).sendSignalToRust(),
  );
  final resolved = resolvedSignal.message;
  final items = resolved.playingItems;
  final startIndex = resolved.startIndex;

  if (castTo != null && castTo.attach) {
    await awaitResponse(
      'AttachRendererRequest',
      AttachRendererResponse.rustSignalStream,
      send: () => AttachRendererRequest(fingerprint: castTo.fingerprint)
          .sendSignalToRust(),
    );
  }

  if (castTo != null && !castTo.attach) {
    await awaitResponse(
      'CastResolvedItemsRequest',
      CastResolvedItemsResponse.rustSignalStream,
      send: () => CastResolvedItemsRequest(
        fingerprint: castTo.fingerprint,
        items: items,
        startIndex: startIndex,
        mode: operateMode,
        expectedRevision: null,
        playbackMode: instantlyPlay ? playbackMode : null,
      ).sendSignalToRust(),
    );
  } else {
    await awaitResponse(
      'LoadResolvedItemsRequest',
      LoadResolvedItemsResponse.rustSignalStream,
      send: () => LoadResolvedItemsRequest(
        items: items,
        startIndex: startIndex,
        mode: operateMode,
        expectedRevision: null,
        playbackMode: instantlyPlay ? playbackMode : null,
      ).sendSignalToRust(),
    );
  }

  return items.map(PlayingItem.fromRequest).toList();
}

Future<List<PlayingItem>> safeOperatePlaybackWithMixQuery({
  required BuildContext context,
  required QueryList queries,
  required int playbackMode,
  required int hintPosition,
  required int initialPlaybackId,
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
