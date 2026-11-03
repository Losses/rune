import 'dart:async';

import '../bindings/bindings.dart';
import '../utils/rune_log.dart';

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
    await closeLibrary();
    showLoss(loss.kind);
    _handling = false;
  }

  void dispose() => _subscription.cancel();
}
