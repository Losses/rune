import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import '../utils/playing_item.dart';
import 'status.dart';

/// What the item this device plays supports of the requests answered by the
/// library it belongs to, as Rust last declared. The open library's
/// capabilities cannot say this: the queue may hold items of other libraries.
class PlayingItemCapabilitiesProvider extends ChangeNotifier {
  PlayingItemCapabilitiesProvider({
    Stream<PlayingItemCapabilities>? changes,
    PlayingItemCapabilities? initial,
  }) {
    final first = initial ?? PlayingItemCapabilities.latestRustSignal?.message;
    if (first != null) _remember(first);
    _subscription =
        (changes ??
                PlayingItemCapabilities.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_update);
  }

  /// How many items' declarations are kept. Rust declares the next item before
  /// the status that names it reaches the UI, so the item still shown must keep
  /// its own declaration meanwhile.
  static const _kept = 3;

  // The latest declaration for each item, in the order declared, the most
  // recent last.
  final _declared = <PlayingItem, PlayingItemCapabilities>{};
  late final StreamSubscription<PlayingItemCapabilities> _subscription;

  /// What [item] supports of the request [select] picks, as Rust last declared
  /// for that item. Without a declaration for it the request is taken to be
  /// available, as before item capabilities were declared.
  RequestCapability capabilityFor(
    PlayingItem? item,
    RequestCapability Function(PlayingItemCapabilities capabilities) select,
  ) {
    final declared = item == null ? null : _declared[item];
    return declared == null
        ? const RequestCapabilityAvailable()
        : select(declared);
  }

  void _remember(PlayingItemCapabilities capabilities) {
    final item = capabilities.item;
    if (item == null) return;
    final key = PlayingItem.fromRequest(item);
    _declared.remove(key);
    _declared[key] = capabilities;
    while (_declared.length > _kept) {
      _declared.remove(_declared.keys.first);
    }
  }

  void _update(PlayingItemCapabilities capabilities) {
    _remember(capabilities);
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }
}

/// What the item playing now supports of the request [select] picks; available
/// without a provider.
RequestCapability playingItemCapability(
  BuildContext context,
  RequestCapability Function(PlayingItemCapabilities capabilities) select, {
  bool listen = true,
}) {
  final capabilities = Provider.of<PlayingItemCapabilitiesProvider?>(
    context,
    listen: listen,
  );
  if (capabilities == null) return const RequestCapabilityAvailable();
  final item = Provider.of<PlaybackStatusProvider?>(
    context,
    listen: listen,
  )?.playingItem;
  return capabilities.capabilityFor(item, select);
}
