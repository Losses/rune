import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import 'status.dart';

class RequestCapabilitiesProvider extends ChangeNotifier {
  RequestCapabilitiesProvider({
    Stream<RequestCapabilities>? changes,
    RequestCapabilities? initial,
  }) : _snapshot = initial ?? RequestCapabilities.latestRustSignal?.message {
    _subscription =
        (changes ??
                RequestCapabilities.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_update);
  }

  RequestCapabilities? _snapshot;
  late final StreamSubscription<RequestCapabilities> _subscription;

  RequestCapabilities? get snapshot => _snapshot;

  RequestCapability get volumeRequest =>
      _snapshot?.volumeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get playRequest =>
      _snapshot?.playRequest ?? const RequestCapabilityAvailable();
  RequestCapability get pauseRequest =>
      _snapshot?.pauseRequest ?? const RequestCapabilityAvailable();
  RequestCapability get nextRequest =>
      _snapshot?.nextRequest ?? const RequestCapabilityAvailable();
  RequestCapability get previousRequest =>
      _snapshot?.previousRequest ?? const RequestCapabilityAvailable();
  RequestCapability get switchRequest =>
      _snapshot?.switchRequest ?? const RequestCapabilityAvailable();
  RequestCapability get seekRequest =>
      _snapshot?.seekRequest ?? const RequestCapabilityAvailable();
  RequestCapability get removeRequest =>
      _snapshot?.removeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get setPlaybackModeRequest =>
      _snapshot?.setPlaybackModeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get movePlaylistItemRequest =>
      _snapshot?.movePlaylistItemRequest ?? const RequestCapabilityAvailable();

  void _update(RequestCapabilities snapshot) {
    _snapshot = snapshot;
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }
}

RequestCapability playPauseCapabilityFor(
  BuildContext context, {
  bool listen = true,
}) {
  final capabilities = requestCapabilitiesOf(context, listen: listen);
  if (capabilities == null) return const RequestCapabilityAvailable();
  if (playPauseIsHidden(capabilities)) return const RequestCapabilityHidden();
  return _playPauseCapability(context, capabilities, listen: listen);
}

RequestCapabilitiesProvider? requestCapabilitiesOf(
  BuildContext context, {
  bool listen = true,
}) => Provider.of<RequestCapabilitiesProvider?>(context, listen: listen);

RequestCapability capabilityForRequest(
  BuildContext context,
  RequestCapability Function(RequestCapabilitiesProvider provider) select, {
  bool listen = true,
}) {
  final capabilities = requestCapabilitiesOf(context, listen: listen);
  return capabilities == null
      ? const RequestCapabilityAvailable()
      : select(capabilities);
}

RequestCapability _playPauseCapability(
  BuildContext context,
  RequestCapabilitiesProvider capabilities, {
  required bool listen,
}) {
  final playing =
      Provider.of<PlaybackStatusProvider?>(
        context,
        listen: listen,
      )?.playbackStatus.state ==
      'Playing';
  final active = playing ? capabilities.pauseRequest : capabilities.playRequest;
  if (active is RequestCapabilityHidden &&
      (capabilities.pauseRequest is! RequestCapabilityHidden ||
          capabilities.playRequest is! RequestCapabilityHidden)) {
    return const RequestCapabilityDisabled(
      reason: CapabilityReason.notImplemented,
    );
  }
  return active;
}

bool playPauseIsHidden(RequestCapabilitiesProvider capabilities) =>
    capabilities.playRequest is RequestCapabilityHidden &&
    capabilities.pauseRequest is RequestCapabilityHidden;
