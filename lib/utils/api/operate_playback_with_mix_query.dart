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
  final rustSignal = await awaitResponse(
    'OperatePlaybackWithMixQueryRequest',
    OperatePlaybackWithMixQueryResponse.rustSignalStream,
    send: () => OperatePlaybackWithMixQueryRequest(
      queries: queries.toQueryList(),
      playbackMode: playbackMode,
      hintPosition: hintPosition,
      initialPlaybackItem: PlayingItem.inLibrary(initialPlaybackId).toRequest(),
      instantlyPlay: instantlyPlay,
      operateMode: operateMode,
      fallbackPlayingItems: fallbackPlayingItems
          .map((x) => x.toRequest())
          .toList(),
      castTo: castTo,
    ).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.playingItems.map(PlayingItem.fromRequest).toList();
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
