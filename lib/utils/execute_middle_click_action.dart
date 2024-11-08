import 'package:fluent_ui/fluent_ui.dart';

import '../messages/all.dart';
import '../screens/settings_playback/settings_playback.dart';
import 'api/if_analysis_exists.dart';
import 'api/operate_playback_with_mix_query.dart';
import 'build_query.dart';
import 'dialogs/no_analysis/show_no_analysis_dialog.dart';
import 'get_non_replace_operate_mode.dart';
import 'query_list.dart';
import 'settings_manager.dart';

Future<void> executeMiddleClickAction(
  BuildContext context,
  CollectionType collectionType,
  int id,
) async {
  String middleClickAction =
      await SettingsManager().getValue<String>(middleClickActionKey) ??
          "StartPlaying";

  if (!context.mounted) return;

  switch (middleClickAction) {
    case "StartPlaying":
      await startPlaying(collectionType, id);
      break;
    case "AddToQueue":
      await addToQueue(collectionType, id);
      break;
    case "StartRoaming":
      await startRoaming(context, collectionType, id);
      break;
    default:
      await startPlaying(collectionType, id);
  }
}

Future<void> startPlaying(
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  final queries = QueryList(await buildQuery(collectionType, id));
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: 99,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: true,
    operateMode: PlaylistOperateMode.Replace,
    fallbackFileIds: fallbackFileIds,
  );
}

Future<void> addToQueue(
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  final queries = QueryList(await buildQuery(collectionType, id));
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: 99,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: false,
    operateMode: await getNonReplaceOperateMode(),
    fallbackFileIds: fallbackFileIds,
  );
}

Future<void> startRoaming(
  BuildContext context,
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  // TODO: IMPLEMENT COLLECTION ANALYSED CHECK HERE
  final analysed = await ifAnalyseExists(id);
  if (!analysed) {
    if (!context.mounted) return;
    showNoAnalysisDialog(context);
    return;
  }

  final queries = QueryList(
    withRecommend(
      await buildQuery(
        collectionType,
        id,
      ),
    ),
  );

  if (context.mounted) {
    await safeOperatePlaybackWithMixQuery(
      context: context,
      queries: queries,
      playbackMode: 99,
      hintPosition: -1,
      initialPlaybackId: 0,
      instantlyPlay: true,
      operateMode: PlaylistOperateMode.Replace,
      fallbackFileIds: fallbackFileIds,
    );
  }
}
