import '../bindings/bindings.dart';

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

  // Mirrors `From<PlayingItemRequest> for PlayingItem` in the hub: a file
  // outside the library is sent without `inLibrary`.
  static PlayingItem fromRequest(PlayingItemRequest request) {
    final inLibrary = request.inLibrary;
    final independentFile = request.independentFile;
    final libraryItem = request.libraryItem;
    final subsonicItem = request.subsonicItem;

    if (subsonicItem != null) {
      return PlayingItem.subsonicTrack(
        libraryId: subsonicItem.libraryId,
        trackId: subsonicItem.trackId,
      );
    } else if (inLibrary != null && inLibrary.fileId != 0) {
      return PlayingItem.inLibrary(inLibrary.fileId);
    } else if (libraryItem != null) {
      return PlayingItem.libraryItem(
        libraryId: libraryItem.libraryId,
        fileUuid: libraryItem.fileUuid,
      );
    } else if (independentFile != null && independentFile.rawPath.isNotEmpty) {
      return PlayingItem.independentFile(independentFile.rawPath);
    } else {
      return PlayingItem.unknown();
    }
  }

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
  int get hashCode =>
      Object.hash(inLibrary, libraryId, fileUuid, subsonicLibraryId,
          subsonicTrackId, independentFile, isUnknown);

  static const independentFilePrefix = 'PlayingItem::IndependentFile(';

  static PlayingItem fromString(String str) {
    if (str.startsWith('PlayingItem::InLibrary(') && str.endsWith(')')) {
      final idStr = str.substring(23, str.length - 1);
      final id = int.tryParse(idStr);
      if (id != null) {
        return PlayingItem.inLibrary(id);
      }
    } else if (str.startsWith(independentFilePrefix) &&
        str.endsWith(')')) {
      final path = str.substring(independentFilePrefix.length, str.length - 1);
      return PlayingItem.independentFile(path);
    }
    // Unknown, Online and anything else the Dart side cannot represent
    return PlayingItem.unknown();
  }

  @override
  String toString() {
    if (inLibrary != null) {
      return 'PlayingItem::InLibrary($inLibrary)';
    } else if (independentFile != null) {
      return 'PlayingItem::IndependentFile($independentFile)';
    } else if (libraryId != null && fileUuid != null) {
      return 'PlayingItem::LibraryItem($libraryId, $fileUuid)';
    } else if (subsonicLibraryId != null && subsonicTrackId != null) {
      return 'PlayingItem::Subsonic($subsonicLibraryId, $subsonicTrackId)';
    } else if (isUnknown) {
      return 'PlayingItem::Unknown()';
    }
    return 'PlayingItem::Unknown()';
  }
}

extension PlayingItemExtension on PlayingItem {
  PlayingItemRequest toRequest() {
    if (inLibrary != null) {
      return PlayingItemRequest(
        inLibrary: InLibraryPlayingItem(fileId: inLibrary!),
        independentFile: null,
        libraryItem: null,
      );
    } else if (libraryId != null && fileUuid != null) {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: null,
        libraryItem: LibraryItemRequest(
          libraryId: libraryId!,
          fileUuid: fileUuid!,
        ),
      );
    } else if (subsonicLibraryId != null && subsonicTrackId != null) {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: null,
        libraryItem: null,
        subsonicItem: SubsonicItemRequest(
          libraryId: subsonicLibraryId!,
          trackId: subsonicTrackId!,
        ),
      );
    } else if (independentFile != null) {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: IndependentFilePlayingItem(rawPath: independentFile!),
        libraryItem: null,
      );
    } else {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: null,
        libraryItem: null,
      );
    }
  }
}
