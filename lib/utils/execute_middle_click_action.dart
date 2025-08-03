import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../constants/configurations.dart';

import 'api/operate_playback_with_mix_query.dart';

import 'query_list.dart';
import 'build_query.dart';
import 'playing_item.dart';
import 'settings_manager.dart';
import 'get_non_replace_operate_mode.dart';

Future<void> executeMiddleClickAction(
  BuildContext context,
  CollectionType collectionType,
  int id,
) async {
  String middleClickAction =
      await SettingsManager().getValue<String>(kMiddleClickActionKey) ??
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
    operateMode: PlaylistOperateMode.replace,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
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
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
  );
}

Future<void> startRoaming(
  BuildContext context,
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  final queries = QueryList(
    withRecommend(
      await buildQuery(
        collectionType,
        id,
      ),
    ),
  );

  if (!context.mounted) return;

  await safeOperatePlaybackWithMixQuery(
    context: context,
    queries: queries,
    playbackMode: 99,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: true,
    operateMode: PlaylistOperateMode.replace,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
  );
}
