import 'dart:async';

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
  Uint64? _revision;

  List<PlaylistEntry> get items => _items;

  late StreamSubscription<PlaylistUpdate> subscription;

  PlaylistProvider({
    Stream<PlaylistUpdate>? playlistUpdates,
    void Function(MovePlaylistItemRequest)? sendMoveRequest,
    void Function(RemoveRequest)? sendRemoveRequest,
  }) : _sendMoveRequest = sendMoveRequest ?? _sendMoveRequestToRust {
    _sendRemoveRequest = sendRemoveRequest ?? _sendRemoveRequestToRust;
    subscription =
        (playlistUpdates ??
                PlaylistUpdate.rustSignalStream.map((event) => event.message))
            .listen(_updatePlaylist);
  }

  final void Function(MovePlaylistItemRequest) _sendMoveRequest;
  late final void Function(RemoveRequest) _sendRemoveRequest;

  static void _sendMoveRequestToRust(MovePlaylistItemRequest request) {
    request.sendSignalToRust();
  }

  static void _sendRemoveRequestToRust(RemoveRequest request) {
    request.sendSignalToRust();
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void _updatePlaylist(PlaylistUpdate playlistUpdate) {
    _revision = playlistUpdate.revision;
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

    _sendMoveRequest(
      MovePlaylistItemRequest(
        oldIndex: oldIndex,
        newIndex: newIndex,
        expectedRevision: _revision,
      ),
    );
  }

  void removeItem(int index) {
    _sendRemoveRequest(
      RemoveRequest(index: index, expectedRevision: _revision),
    );
  }
}
