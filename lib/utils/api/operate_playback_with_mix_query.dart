import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/queries_has_recommendation.dart';
import '../../utils/dialogs/no_analysis/show_no_analysis_dialog.dart';
import '../../messages/playback.pb.dart';

Future<List<int>> operatePlaybackWithMixQuery({
  required QueryList queries,
  required int playbackMode,
  required int hintPosition,
  required int initialPlaybackId,
  required bool instantlyPlay,
  required bool replacePlaylist,
  required List<int> fallbackFileIds,
}) async {
  OperatePlaybackWithMixQueryRequest(
    queries: queries.toQueryList(),
    playbackMode: playbackMode,
    hintPosition: hintPosition,
    initialPlaybackId: initialPlaybackId,
    instantlyPlay: instantlyPlay,
    replacePlaylist: replacePlaylist,
    fallbackMediaFileIds: fallbackFileIds,
  ).sendSignalToRust();

  final rustSignal =
      await OperatePlaybackWithMixQueryResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.fileIds;
}

Future<List<int>> safeOperatePlaybackWithMixQuery({
  required BuildContext context,
  required QueryList queries,
  required int playbackMode,
  required int hintPosition,
  required int initialPlaybackId,
  required bool instantlyPlay,
  required bool replacePlaylist,
  required List<int> fallbackFileIds,
}) async {
  final hasRecommendation = queriesHasRecommendation(queries);

  final result = await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: playbackMode,
    hintPosition: hintPosition,
    initialPlaybackId: initialPlaybackId,
    instantlyPlay: instantlyPlay,
    replacePlaylist: replacePlaylist,
    fallbackFileIds: fallbackFileIds,
  );

  if (result.isEmpty && hasRecommendation && context.mounted) {
    await showNoAnalysisDialog(context, true);
  }

  return result;
}
