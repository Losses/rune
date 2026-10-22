import '../bindings/bindings.dart';

class PlayingItem {
  final int? inLibrary;
  final String? independentFile;
  final bool isUnknown;

  const PlayingItem._(
      {this.inLibrary, this.independentFile, this.isUnknown = false});

  const PlayingItem.inLibrary(int id) : this._(inLibrary: id);
  const PlayingItem.independentFile(String path)
      : this._(independentFile: path);
  const PlayingItem.unknown() : this._(isUnknown: true);

  // Mirrors `From<PlayingItemRequest> for PlayingItem` in the hub: a file
  // outside the library is sent without `inLibrary`.
  static PlayingItem fromRequest(PlayingItemRequest request) {
    final inLibrary = request.inLibrary;
    final independentFile = request.independentFile;

    if (inLibrary != null && inLibrary.fileId != 0) {
      return PlayingItem.inLibrary(inLibrary.fileId);
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
          independentFile == other.independentFile &&
          isUnknown == other.isUnknown;

  @override
  int get hashCode => Object.hash(inLibrary, independentFile, isUnknown);

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
      );
    } else if (independentFile != null) {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: IndependentFilePlayingItem(rawPath: independentFile!),
      );
    } else {
      return PlayingItemRequest(
        inLibrary: null,
        independentFile: null,
      );
    }
  }
}
