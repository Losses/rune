import 'dart:async';

import '../bindings/bindings.dart';

class RendererLinkLossHandler {
  final void Function(String alias) showNotice;
  late final StreamSubscription<RendererLinkLost> _subscription;

  RendererLinkLossHandler({
    required Stream<RendererLinkLost> losses,
    required this.showNotice,
  }) {
    _subscription = losses.listen((loss) => showNotice(loss.alias));
  }

  void dispose() => _subscription.cancel();
}
