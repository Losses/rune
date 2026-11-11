import '../bindings/bindings.dart';
import 'api/fetch_media_file_by_ids.dart';
import '../widgets/track_list/utils/internal_media_file.dart';

class PlayingItem {
  final int? inLibrary;
  final String? libraryId;
  final String? fileUuid;
  final String? subsonicLibraryId;
  final String? subsonicTrackId;
  final String? independentFile;
  final bool isUnknown;

  const PlayingItem._({
    this.inLibrary,
    this.libraryId,
    this.fileUuid,
    this.subsonicLibraryId,
    this.subsonicTrackId,
    this.independentFile,
    this.isUnknown = false,
  });

  const PlayingItem.inLibrary(int id) : this._(inLibrary: id);
  const PlayingItem.libraryItem({
    required String libraryId,
    required String fileUuid,
  }) : this._(libraryId: libraryId, fileUuid: fileUuid);
  const PlayingItem.subsonicTrack({
    required String libraryId,
    required String trackId,
  }) : this._(subsonicLibraryId: libraryId, subsonicTrackId: trackId);
  const PlayingItem.independentFile(String path)
    : this._(independentFile: path);
  const PlayingItem.unknown() : this._(isUnknown: true);

  bool get hasLocalFileId => inLibrary != null;

  static PlayingItem fromRequest(PlayingItemRef request) => switch (request) {
    PlayingItemRefLibraryTrack(:final libraryId, :final track)
        when _isUuid(track) =>
      PlayingItem.libraryItem(libraryId: libraryId, fileUuid: track),
    PlayingItemRefLibraryTrack(:final libraryId, :final track) =>
      PlayingItem.subsonicTrack(libraryId: libraryId, trackId: track),
    PlayingItemRefIndependentFile(:final path) when path.isNotEmpty =>
      PlayingItem.independentFile(path),
    _ => const PlayingItem.unknown(),
  };

  static bool _isUuid(String value) => RegExp(
    r'^[0-9a-fA-F]{8}-([0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$',
  ).hasMatch(value);

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is PlayingItem &&
          runtimeType == other.runtimeType &&
          inLibrary == other.inLibrary &&
          libraryId == other.libraryId &&
          fileUuid == other.fileUuid &&
          subsonicLibraryId == other.subsonicLibraryId &&
          subsonicTrackId == other.subsonicTrackId &&
          independentFile == other.independentFile &&
          isUnknown == other.isUnknown;

  @override
  int get hashCode => Object.hash(
    inLibrary,
    libraryId,
    fileUuid,
    subsonicLibraryId,
    subsonicTrackId,
    independentFile,
    isUnknown,
  );

  static const independentFilePrefix = 'PlayingItem::IndependentFile(';

  static PlayingItem fromString(String str) {
    if (str.startsWith(independentFilePrefix) && str.endsWith(')')) {
      return PlayingItem.independentFile(
        str.substring(independentFilePrefix.length, str.length - 1),
      );
    }
    return const PlayingItem.unknown();
  }

  @override
  String toString() {
    if (inLibrary != null) return 'PlayingItem::InLibrary($inLibrary)';
    if (independentFile != null) {
      return 'PlayingItem::IndependentFile($independentFile)';
    }
    if (libraryId != null && fileUuid != null) {
      return 'PlayingItem::LibraryTrack($libraryId, $fileUuid)';
    }
    if (subsonicLibraryId != null && subsonicTrackId != null) {
      return 'PlayingItem::LibraryTrack($subsonicLibraryId, $subsonicTrackId)';
    }
    return 'PlayingItem::Unknown()';
  }
}

/// The single conversion point from a local catalog row to its stable wire identity.
PlayingItemRef localTrackRef(int rowId, Iterable<InternalMediaFile> files) {
  InternalMediaFile? file;
  for (final candidate in files) {
    if (candidate.id == rowId) {
      file = candidate;
      break;
    }
  }
  if (file == null) {
    throw StateError(
      'Track row $rowId was not present in the resolved catalog items',
    );
  }
  return PlayingItemRefLibraryTrack(
    libraryId: file.libraryId,
    track: file.track,
  );
}

Future<List<PlayingItemRef>> playingItemRefs(List<PlayingItem> items) async {
  final rowIds = items
      .map((item) => item.inLibrary)
      .whereType<int>()
      .toSet()
      .toList();
  final files = rowIds.isEmpty
      ? <InternalMediaFile>[]
      : await fetchMediaFileByIds(rowIds, false);
  return items.map((item) {
    if (item.inLibrary != null) return localTrackRef(item.inLibrary!, files);
    return item.toRequest();
  }).toList();
}

extension PlayingItemExtension on PlayingItem {
  PlayingItemRef toRequest() {
    if (libraryId != null && fileUuid != null) {
      return PlayingItemRefLibraryTrack(
        libraryId: libraryId!,
        track: fileUuid!,
      );
    }
    if (subsonicLibraryId != null && subsonicTrackId != null) {
      return PlayingItemRefLibraryTrack(
        libraryId: subsonicLibraryId!,
        track: subsonicTrackId!,
      );
    }
    if (independentFile != null) {
      return PlayingItemRefIndependentFile(path: independentFile!);
    }
    if (inLibrary != null) {
      throw StateError(
        'Local row ids must be converted with localTrackRef before sending',
      );
    }
    throw StateError('Unknown playing items cannot be sent');
  }
}
