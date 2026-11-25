import 'dart:async';

import 'package:flutter/foundation.dart';

import '../bindings/bindings.dart';

class RendererAttachmentProvider extends ChangeNotifier {
  RendererAttachmentProvider({
    Stream<RendererAttachmentChanged>? changes,
    RendererAttachmentChanged? initial,
  }) : _renderer =
           (initial ?? RendererAttachmentChanged.latestRustSignal?.message)
               ?.renderer {
    _subscription =
        (changes ??
                RendererAttachmentChanged.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_update);
  }

  AttachedRenderer? _renderer;
  late final StreamSubscription<RendererAttachmentChanged> _subscription;

  AttachedRenderer? get renderer => _renderer;

  void _update(RendererAttachmentChanged change) {
    _renderer = change.renderer;
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }
}
