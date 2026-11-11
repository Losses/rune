import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../utils/api/request_failure.dart';
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
  List<PlaylistEntry>? _pendingReorder;
  Uint64? _pendingReorderCallId;
  int _nextCallId = DateTime.now().microsecondsSinceEpoch;

  List<PlaylistEntry> get items => _items;

  late StreamSubscription<PlaylistUpdate> subscription;
  late final StreamSubscription<RequestFailedResponse> _failureSubscription;
  final StreamController<RequestFailure> _failureFeedback =
      StreamController<RequestFailure>.broadcast();

  PlaylistProvider({
    Stream<PlaylistUpdate>? playlistUpdates,
    Stream<RequestFailedResponse>? requestFailures,
    void Function(MovePlaylistItemRequest)? sendMoveRequest,
    void Function(RemoveRequest)? sendRemoveRequest,
    void Function(SwitchRequest)? sendSwitchRequest,
  }) : _sendMoveRequest = sendMoveRequest ?? _sendMoveRequestToRust {
    _sendRemoveRequest = sendRemoveRequest ?? _sendRemoveRequestToRust;
    _sendSwitchRequest = sendSwitchRequest ?? _sendSwitchRequestToRust;
    subscription =
        (playlistUpdates ??
                PlaylistUpdate.rustSignalStream.map((event) => event.message))
            .listen(_updatePlaylist);
    _failureSubscription =
        (requestFailures ??
                RequestFailedResponse.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_handleRequestFailure);
  }

  final void Function(MovePlaylistItemRequest) _sendMoveRequest;
  late final void Function(RemoveRequest) _sendRemoveRequest;
  late final void Function(SwitchRequest) _sendSwitchRequest;

  Stream<RequestFailure> get failureFeedback => _failureFeedback.stream;

  static void _sendMoveRequestToRust(MovePlaylistItemRequest request) {
    request.sendSignalToRust();
  }

  static void _sendRemoveRequestToRust(RemoveRequest request) {
    request.sendSignalToRust();
  }

  static void _sendSwitchRequestToRust(SwitchRequest request) {
    request.sendSignalToRust();
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
    _failureSubscription.cancel();
    _failureFeedback.close();
  }

  void _handleRequestFailure(RequestFailedResponse response) {
    if (response.kind == RequestErrorKind.conflict &&
        _pendingReorder != null &&
        response.callId == _pendingReorderCallId) {
      _items = _pendingReorder!;
      _pendingReorder = null;
      _pendingReorderCallId = null;
      notifyListeners();
    }
    if (response.request == 'MovePlaylistItemRequest' ||
        response.request == 'RemoveRequest' ||
        response.request == 'SwitchRequest') {
      _failureFeedback.add(
        RequestFailure(
          response.request,
          response.kind,
          response.detail,
          response.cause,
        ),
      );
    }
  }

  void _updatePlaylist(PlaylistUpdate playlistUpdate) {
    _pendingReorder = null;
    _pendingReorderCallId = null;
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
    _pendingReorder = List<PlaylistEntry>.of(_items);
    final callId = Uint64.fromBigInt(BigInt.from(_nextCallId++));
    _pendingReorderCallId = callId;
    final item = items.removeAt(oldIndex);
    items.insert(newIndex, item);
    notifyListeners();

    _sendMoveRequest(
      MovePlaylistItemRequest(
        oldIndex: oldIndex,
        newIndex: newIndex,
        expectedRevision: _revision,
        callId: callId,
      ),
    );
  }

  void removeItem(int index) {
    _sendRemoveRequest(
      RemoveRequest(index: index, expectedRevision: _revision),
    );
  }

  void playItem(int index) {
    _sendSwitchRequest(
      SwitchRequest(index: index, expectedRevision: _revision),
    );
  }
}
