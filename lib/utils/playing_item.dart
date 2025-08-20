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

  static PlayingItem fromRequest(PlayingItemRequest request) {
    final inLibrary = request.inLibrary;

    if (inLibrary == null) {
      return PlayingItem.unknown();
    } else {
      final fileId = inLibrary.fileId;
      final independentFile = request.independentFile;

      if (fileId != 0) {
        return PlayingItem.inLibrary(fileId);
      } else if (independentFile != null) {
        return PlayingItem.independentFile(independentFile.rawPath);
      } else {
        return PlayingItem.unknown();
      }
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

  static PlayingItem fromString(String str) {
    if (str.startsWith('PlayingItem::InLibrary(') && str.endsWith(')')) {
      final idStr = str.substring(23, str.length - 1);
      final id = int.tryParse(idStr);
      if (id != null) {
        return PlayingItem.inLibrary(id);
      }
    } else if (str.startsWith('PlayingItem::IndependentFile(') &&
        str.endsWith(')')) {
      final path = str.substring(28, str.length - 1);
      return PlayingItem.independentFile(path);
    } else if (str == 'PlayingItem::Unknown()') {
      return PlayingItem.unknown();
    }
    throw ArgumentError('Invalid string format');
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
