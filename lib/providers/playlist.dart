import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../messages/playback.pb.dart';

final class PlaylistEntry {
  final int index;
  final PlaylistItem entry;

  const factory PlaylistEntry(int index, PlaylistItem entry) = PlaylistEntry._;

  const PlaylistEntry._(this.index, this.entry);

  @override
  String toString() => "PlaylistEntry($index: $entry)";
}

class PlaylistProvider with ChangeNotifier {
  List<PlaylistEntry> _items = [];

  List<PlaylistEntry> get items => _items;

  void updatePlaylist(List<PlaylistItem> newItems) {
    _items = newItems
        .asMap()
        .entries
        .map((entry) => PlaylistEntry(entry.key, entry.value))
        .toList();
    notifyListeners();
  }

  void reorderItems(int oldIndex, int newIndex) {
    if (newIndex > oldIndex) {
      newIndex -= 1;
    }
    final item = items.removeAt(oldIndex);
    items.insert(newIndex, item);
    notifyListeners();

    MovePlaylistItemRequest(oldIndex: oldIndex, newIndex: newIndex)
        .sendSignalToRust();
  }
}

class PlaylistUpdateHandler {
  static void init(BuildContext context) {
    PlaylistUpdate.rustSignalStream.listen((event) {
      final playlistUpdate = event.message;
      final items = playlistUpdate.items;

      Provider.of<PlaylistProvider>(context, listen: false)
          .updatePlaylist(items);
    });
  }
}
