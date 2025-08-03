import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../utils/playing_item.dart';

final class PlaylistEntry {
  final int index;
  final PlaylistItem entry;
  final PlayingItem item;

  factory PlaylistEntry(int index, PlaylistItem entry) = PlaylistEntry._;

  PlaylistEntry._(this.index, this.entry)
      : item = PlayingItem.fromRequest(entry.item);

  @override
  String toString() => "PlaylistEntry($index: $entry)";
}

class PlaylistProvider with ChangeNotifier {
  List<PlaylistEntry> _items = [];

  List<PlaylistEntry> get items => _items;

  late StreamSubscription<RustSignalPack<PlaylistUpdate>> subscription;

  PlaylistProvider() {
    subscription = PlaylistUpdate.rustSignalStream.listen(_updatePlaylist);
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void _updatePlaylist(RustSignalPack<PlaylistUpdate> event) {
    final playlistUpdate = event.message;
    final newItems = playlistUpdate.items;
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
