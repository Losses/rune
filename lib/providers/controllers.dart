import 'dart:async';

import 'package:flutter/foundation.dart';

import '../bindings/bindings.dart';
import '../utils/api/controllers.dart';

class ControllersProvider extends ChangeNotifier {
  ControllersProvider({
    Future<List<AttachedControllerInfo>> Function()? load,
    Stream<ControllersUpdated>? changes,
    Future<void> Function(String id)? detach,
  }) : _load = load ?? listControllers {
    _detach = detach ?? detachController;
    _subscription =
        (changes ??
                ControllersUpdated.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_update);
    unawaited(_loadInitial());
  }

  final Future<List<AttachedControllerInfo>> Function() _load;
  late final Future<void> Function(String id) _detach;
  late final StreamSubscription<ControllersUpdated> _subscription;
  List<AttachedControllerInfo> _controllers = const [];
  bool _receivedUpdate = false;

  bool isControlling(String fingerprint) =>
      _controllers.any((controller) => controller.fingerprint == fingerprint);

  Future<void> disconnect(String fingerprint) async {
    final ids = _controllers
        .where((controller) => controller.fingerprint == fingerprint)
        .map((controller) => controller.id)
        .toList();
    await Future.wait(ids.map(_detach));
  }

  Future<void> _loadInitial() async {
    try {
      final controllers = await _load();
      if (_receivedUpdate) return;
      _controllers = controllers;
      notifyListeners();
    } catch (error) {
      debugPrint('Could not load attached controllers: $error');
    }
  }

  void _update(ControllersUpdated update) {
    _receivedUpdate = true;
    _controllers = update.controllers;
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }
}
