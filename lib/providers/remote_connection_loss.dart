import 'dart:async';

import '../bindings/bindings.dart';
import '../utils/rune_log.dart';
import '../utils/api/request_failure.dart';

class RemoteConnectionLossHandler {
  final Future<void> Function() closeLibrary;
  final void Function(RemoteLossKind) showLoss;
  late final StreamSubscription<RemoteConnectionLost> _subscription;
  bool _handling = false;

  RemoteConnectionLossHandler({
    required Stream<RemoteConnectionLost> losses,
    required this.closeLibrary,
    required this.showLoss,
  }) {
    _subscription = losses.listen(_handle);
  }

  Future<void> _handle(RemoteConnectionLost loss) async {
    if (_handling) return;
    _handling = true;
    error$('Remote connection lost: ${loss.detail}');
    try {
      try {
        await closeLibrary();
      } on RequestFailure {
        // The connection is already lost; continue to the loss screen.
      }
      showLoss(loss.kind);
    } finally {
      _handling = false;
    }
  }

  void dispose() => _subscription.cancel();
}
