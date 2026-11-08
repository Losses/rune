import 'dart:async';

import 'package:flutter/foundation.dart';

import '../bindings/bindings.dart';

class RemoteLibraryReconnectNotice extends ValueNotifier<String?> {
  final List<StreamSubscription<Object?>> _subscriptions = [];
  String? _fingerprint;

  RemoteLibraryReconnectNotice({
    required Stream<RemoteLibraryReconnecting> reconnecting,
    required Stream<RemoteLibraryReconnected> reconnected,
    required Stream<RemoteConnectionLost> losses,
  }) : super(null) {
    _subscriptions.add(
      reconnecting.listen((event) {
        _fingerprint = event.fingerprint;
        value = event.alias;
      }),
    );
    _subscriptions.add(
      reconnected.listen((event) {
        if (event.fingerprint == _fingerprint) clear();
      }),
    );
    _subscriptions.add(losses.listen((_) => clear()));
  }

  void clear() {
    _fingerprint = null;
    value = null;
  }

  @override
  void dispose() {
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    super.dispose();
  }
}
