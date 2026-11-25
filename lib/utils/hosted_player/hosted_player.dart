import 'dart:async';
import 'dart:convert';

import '../../bindings/bindings.dart';
import '../logger.dart';

/// A platform host that plays tracks Rune cannot stream. Commands come from the player as JSON
/// maps, and events go back the same way.
abstract class HostedPlayerHost {
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  );
}

/// A host that also reports by itself, as a host on this device does when it starts or when its
/// access changes. The bridge attaches it once, before it passes on any command, so that the
/// player knows whether the host can play before it hands the host a track.
abstract class SelfReportingHost implements HostedPlayerHost {
  void attach(void Function(Map<String, dynamic>) emit);
}

/// The hosts this platform provides, by family. Platform code adds its hosts here at startup,
/// and the player bridge looks them up.
final Map<String, HostedPlayerHost> platformHosts = {};

/// Routes the player's commands to the host registered for each family.
///
/// A family with no host is answered once with a host that cannot play, so its tracks become
/// unavailable and are skipped. A real host replaces that by being passed in [hosts].
class HostedPlayerBridge {
  final Map<String, HostedPlayerHost> hosts;
  final void Function(String payload) _send;
  final Set<String> _unsupported = {};
  late final StreamSubscription<String> _subscription;

  HostedPlayerBridge({
    required Stream<String> commands,
    required void Function(String payload) send,
    this.hosts = const {},
  }) : _send = send {
    _subscription = commands.listen(_onCommand);
    for (final host in hosts.values) {
      if (host is SelfReportingHost) host.attach(_emit);
    }
  }

  /// Wires the bridge to the player's messages.
  factory HostedPlayerBridge.attached({
    Map<String, HostedPlayerHost> hosts = const {},
  }) {
    return HostedPlayerBridge(
      commands: HostedPlayerCommand.rustSignalStream.map(
        (signal) => signal.message.payload,
      ),
      send: (payload) => HostedPlayerEvent(payload: payload).sendSignalToRust(),
      hosts: hosts.isEmpty ? platformHosts : hosts,
    );
  }

  void _onCommand(String payload) {
    final Object? decoded;
    try {
      decoded = jsonDecode(payload);
    } on FormatException catch (error) {
      logger.w('Ignoring a host command that does not parse: $error');
      return;
    }
    final host = decoded is Map<String, dynamic> ? decoded['host'] : null;
    if (decoded is! Map<String, dynamic> || host is! String) return;
    final implementation = hosts[host];
    if (implementation != null) {
      implementation.handle(decoded, _emit);
    } else if (_unsupported.add(host)) {
      // Nothing here asks for authorization: this platform has no player for
      // the host, so its tracks show as unavailable rather than revoked.
      _send(
        jsonEncode({
          'type': 'hostReady',
          'host': host,
          'authorized': true,
          'canPlay': false,
        }),
      );
    }
  }

  void _emit(Map<String, dynamic> event) => _send(jsonEncode(event));

  void dispose() => _subscription.cancel();
}
