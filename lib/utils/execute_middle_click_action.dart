import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

import 'api/operate_playback_with_mix_query.dart';
import 'api/request_failure.dart';
import 'dialogs/information/error.dart';
import 'l10n.dart';

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

  try {
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
  } on RequestFailure catch (error) {
    if (context.mounted) {
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: requestFailureMessage(S.of(context), error),
      );
    }
  }
}

Future<void> startPlaying(
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  final queries = await _buildCollectionQueries(collectionType, id);
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: true,
    operateMode: PlaylistOperateMode.replace,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
  );
}

Future<QueryList> _buildCollectionQueries(
  CollectionType collectionType,
  int id,
) async => QueryList(await buildQuery(collectionType, id));

Future<int> castCollection(
  CollectionType collectionType,
  int id,
  List<int> fallbackFileIds,
  CastTarget castTo,
) async {
  final behavior = castCollectionBehavior();
  final queries = await _buildCollectionQueries(collectionType, id);
  var leftOutCount = 0;
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: behavior.instantlyPlay,
    operateMode: behavior.operateMode,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
    castTo: castTo,
    onCastResult: (result) => leftOutCount = result.leftOutItems.length,
  );
  return leftOutCount;
}

({PlaylistOperateMode operateMode, bool instantlyPlay})
castCollectionBehavior() =>
    (operateMode: PlaylistOperateMode.replace, instantlyPlay: true);

CastTarget castTargetFor(String fingerprint, bool attach) =>
    CastTarget(fingerprint: fingerprint, attach: attach);

Future<bool> _castAttaches() async =>
    await $settingsManager.getValue<bool>(kCastControlRendererKey) ?? false;

/// Casts a track to [fingerprint] as the cast settings say.
Future<int> castTrackWithSettings(int id, String fingerprint) async {
  final playNext =
      castTrackBehaviorForSetting(
        await $settingsManager.getValue<String>(kCastTrackModeKey),
      ).operateMode ==
      PlaylistOperateMode.playNext;
  return castTrack(
    id,
    castTargetFor(fingerprint, await _castAttaches()),
    playNext,
  );
}

/// Casts a collection to [fingerprint] as the cast settings say.
Future<int> castCollectionWithSettings(
  CollectionType collectionType,
  int id,
  List<int> fallbackFileIds,
  String fingerprint,
) async => castCollection(
  collectionType,
  id,
  fallbackFileIds,
  castTargetFor(fingerprint, await _castAttaches()),
);

({PlaylistOperateMode operateMode, bool instantlyPlay}) castTrackBehavior(
  bool playNext,
) => playNext
    ? (operateMode: PlaylistOperateMode.playNext, instantlyPlay: false)
    : (operateMode: PlaylistOperateMode.replace, instantlyPlay: true);

({PlaylistOperateMode operateMode, bool instantlyPlay})
castTrackBehaviorForSetting(String? value) =>
    castTrackBehavior(value == 'playNext');

Future<int> castTrack(int id, CastTarget castTo, bool playNext) async {
  final behavior = castTrackBehavior(playNext);
  var leftOutCount = 0;
  await operatePlaybackWithMixQuery(
    queries: await _buildCollectionQueries(CollectionType.track, id),
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: behavior.instantlyPlay,
    operateMode: behavior.operateMode,
    fallbackPlayingItems: const [],
    castTo: castTo,
    onCastResult: (result) => leftOutCount = result.leftOutItems.length,
  );
  return leftOutCount;
}

Future<void> addToQueue(
  CollectionType collectionType,
  int id, [
  List<int> fallbackFileIds = const [],
]) async {
  final queries = QueryList(await buildQuery(collectionType, id));
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: null,
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
    withRecommend(await buildQuery(collectionType, id)),
  );

  if (!context.mounted) return;

  await safeOperatePlaybackWithMixQuery(
    context: context,
    queries: queries,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: 0,
    instantlyPlay: true,
    operateMode: PlaylistOperateMode.replace,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
  );
}
