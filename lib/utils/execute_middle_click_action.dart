import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

import '../providers/request_capabilities.dart';
import 'api/operate_playback_with_mix_query.dart';
import 'api/request_failure.dart';
import 'dialogs/information/error.dart';
import 'l10n.dart';

import 'query_list.dart';
import 'build_query.dart';
import 'playing_item.dart';
import 'settings_manager.dart';
import 'get_non_replace_operate_mode.dart';

/// The action a middle click runs. A library without analysis data cannot
/// roam, so the click plays instead.
String middleClickActionFor(String action, {required bool canRoam}) =>
    action == "StartRoaming" && !canRoam ? "StartPlaying" : action;

Future<void> executeMiddleClickAction(
  BuildContext context,
  CollectionType collectionType,
  int id,
) => _runMiddleClickAction(
  context,
  play: () => startPlaying(collectionType, id),
  queue: () => addToQueue(collectionType, id),
  roam: () => startRoaming(context, collectionType, id),
);

/// The middle click of an item that is a list of queries, as a release section
/// of an artist.
Future<void> executeQueryListMiddleClickAction(
  BuildContext context,
  QueryList queries,
) => _runMiddleClickAction(
  context,
  play: () => startPlayingQueries(queries),
  queue: () => addQueriesToQueue(queries),
  roam: () => startRoamingQueries(context, queries),
);

Future<void> _runMiddleClickAction(
  BuildContext context, {
  required Future<void> Function() play,
  required Future<void> Function() queue,
  required Future<void> Function() roam,
}) async {
  String middleClickAction =
      await SettingsManager().getValue<String>(kMiddleClickActionKey) ??
      "StartPlaying";

  if (!context.mounted) return;

  middleClickAction = middleClickActionFor(
    middleClickAction,
    canRoam: canStartRoaming(context),
  );

  try {
    switch (middleClickAction) {
      case "StartPlaying":
        await play();
        break;
      case "AddToQueue":
        await queue();
        break;
      case "StartRoaming":
        await roam();
        break;
      default:
        await play();
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
  await startPlayingQueries(
    await _buildCollectionQueries(collectionType, id),
    fallbackFileIds,
  );
}

/// Starts playing what [queries] resolve to.
Future<void> startPlayingQueries(
  QueryList queries, [
  List<int> fallbackFileIds = const [],
]) async {
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: null,
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
    initialPlaybackId: null,
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
    initialPlaybackId: null,
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
  await addQueriesToQueue(
    QueryList(await buildQuery(collectionType, id)),
    fallbackFileIds,
  );
}

/// Adds what [queries] resolve to to the queue.
Future<void> addQueriesToQueue(
  QueryList queries, [
  List<int> fallbackFileIds = const [],
]) async {
  await operatePlaybackWithMixQuery(
    queries: queries,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: null,
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
  final queries = QueryList(await buildQuery(collectionType, id));

  if (!context.mounted) return;

  await startRoamingQueries(context, queries, fallbackFileIds);
}

/// Starts roaming from what [queries] resolve to.
Future<void> startRoamingQueries(
  BuildContext context,
  QueryList queries, [
  List<int> fallbackFileIds = const [],
]) async {
  final roaming = QueryList(withRecommend(queries));

  await safeOperatePlaybackWithMixQuery(
    context: context,
    queries: roaming,
    playbackMode: null,
    hintPosition: -1,
    initialPlaybackId: null,
    instantlyPlay: true,
    operateMode: PlaylistOperateMode.replace,
    fallbackPlayingItems: fallbackFileIds.map(PlayingItem.inLibrary).toList(),
  );
}
