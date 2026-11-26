import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';

import '../logger.dart';
import 'hosted_player.dart';

/// Plays Apple Music through the macOS or iOS runner's MusicKit host. Dart only
/// forwards: batches go to the runner as JSON text, and the runner's events
/// come back the same way.
class AppleMusicHost
    implements SelfReportingHost, AuthorizingHost, NetworkReportingHost {
  AppleMusicHost({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel(channelName);

  static const channelName = 'ci.not.rune/apple_music';

  /// The family the player names this host by.
  static const family = 'appleMusic';

  final MethodChannel _channel;
  void Function(Map<String, dynamic>)? _emit;
  Future<void>? _announced;
  Future<void> _queue = Future.value();

  /// The latest `hostReady` passed on: the announcement's, or one the runner
  /// sent since.
  Map<String, dynamic>? _ready;

  /// Whether the runner last reported access, as far as it said; unknown until
  /// it reports.
  bool? _authorized;

  /// The revision of the latest batch whose track failed for lack of
  /// authorization.
  Object? _lostRevision;

  /// Whether the runner last reported access. Null until it reports.
  bool? get authorized => _authorized;

  final _accessLost = StreamController<void>.broadcast();

  /// Fires when Apple Music stops letting Rune play: the access the runner
  /// reported was withdrawn while Rune runs, or a track failed for lack of
  /// authorization, once per batch, while the runner has not reported access. A
  /// host that starts without access, or whose start is unknown, has lost
  /// nothing. The sign-in flow takes it from there.
  @override
  Stream<void> get accessLost => _accessLost.stream;

  /// Whether the runner last reported a network. Null until it reports.
  bool? _online;

  @override
  bool? get online => _online;

  final _networkBack = StreamController<void>.broadcast();

  /// Fires when the runner reports the network back after it reported none. The
  /// sign-in flow tries again the sign-ins that failed without it.
  @override
  Stream<void> get networkBack => _networkBack.stream;

  /// Registers the host on macOS and iOS, where the runner provides the
  /// channel.
  static void registerOnPlatform() {
    if (Platform.isMacOS || Platform.isIOS) {
      platformHosts[family] = AppleMusicHost();
    }
  }

  @override
  Future<HostAuthorizationResult> authorize({bool interactive = false}) async {
    const unavailable = HostAuthorizationFailed(
      HostAuthorizationFailure.unavailable,
    );
    final Object? reply;
    try {
      reply = await _channel.invokeMethod<Object>('authorize', {
        'interactive': interactive,
      });
    } on PlatformException catch (error) {
      logger.w('Apple Music authorization failed: ${error.code}');
      return unavailable;
    } on MissingPluginException {
      return unavailable;
    } catch (error) {
      logger.w('Apple Music authorization failed: ${error.runtimeType}');
      return unavailable;
    }
    return parseAuthorization(reply);
  }

  /// Reads the runner's reply. Without both an authorization and a user token
  /// there is nothing to sign in with; the runner's `reason` tells a system too
  /// old for Apple Music from a refusal, and an account that cannot use Apple
  /// Music, or a privacy notice still to accept, from a token that could not be
  /// had otherwise.
  static HostAuthorizationResult parseAuthorization(Object? reply) {
    if (reply is! Map) {
      return const HostAuthorizationFailed(
        HostAuthorizationFailure.unavailable,
      );
    }
    if (reply['authorized'] != true) {
      return HostAuthorizationFailed(
        reply['reason'] == 'unsupported'
            ? HostAuthorizationFailure.unsupported
            : HostAuthorizationFailure.refused,
      );
    }
    final token = reply['token'];
    if (token is! String || token.isEmpty) {
      return HostAuthorizationFailed(switch (reply['reason']) {
        'account' => HostAuthorizationFailure.account,
        'privacy' => HostAuthorizationFailure.privacy,
        _ => HostAuthorizationFailure.noToken,
      });
    }
    final developerToken = reply['developerToken'];
    return HostAuthorization(
      token: token,
      developerToken: developerToken is String && developerToken.isNotEmpty
          ? developerToken
          : null,
    );
  }

  /// Asks the runner what it can do and passes that on, before the player hands
  /// over anything. The runner drops whatever it still held, since what it
  /// reports is a host that just started.
  @override
  void attach(void Function(Map<String, dynamic>) emit) {
    _listen(emit);
    _announced = _announce();
  }

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {
    _listen(emit);
    // A host that was not attached announces itself before its first batch.
    // Batches wait in line behind the announcement, in order.
    final announced = _announced ??= _announce();
    _queue = _queue.then((_) async {
      await announced;
      final ready = _ready;
      if (ready != null && !_canPlay(ready)) {
        // The hub drops what comes before it is ready, as an announcement made
        // at startup can. A host that cannot play says so again, so the player
        // fails the track at once rather than waiting for a report. One that
        // can is not repeated, since the player would hand its track over
        // again.
        _emit?.call(ready);
        return;
      }
      await _forward(command);
    });
  }

  static bool _canPlay(Map<String, dynamic> ready) =>
      ready['authorized'] == true && ready['canPlay'] == true;

  void _listen(void Function(Map<String, dynamic>) emit) {
    if (_emit == null) {
      _channel.setMethodCallHandler(_onCall);
    }
    _emit = emit;
  }

  /// Asks the runner what it can do and passes the answer on. Never throws.
  /// When the runner cannot be reached, Rust is told that nothing can be played
  /// here.
  Future<void> _announce() async {
    try {
      final reply = await _channel.invokeMethod<Object>('status');
      if (reply is String && _deliver(reply)) return;
      logger.w('Apple Music status was not usable');
    } on PlatformException catch (error) {
      logger.w('Apple Music status failed: ${error.code}');
    } on MissingPluginException {
      // The runner has no host.
    } catch (error) {
      logger.w('Apple Music status failed: ${error.runtimeType}');
    }
    _ready = _unavailable;
    _emit?.call(_unavailable);
  }

  static const _unavailable = {
    'type': 'hostReady',
    'host': family,
    'authorized': true,
    'canPlay': false,
  };

  Future<void> _forward(Map<String, dynamic> command) async {
    try {
      await _channel.invokeMethod<void>('batch', jsonEncode(command));
    } on PlatformException catch (error) {
      logger.w('Apple Music batch failed: ${error.code}');
    } on MissingPluginException {
      // The runner has no host; the announcement already said so.
    } catch (error) {
      logger.w('Apple Music batch failed: ${error.runtimeType}');
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method == 'event' && call.arguments is String) {
      _deliver(call.arguments as String);
    }
    return null;
  }

  /// Passes an event of the runner on. False when it is not a JSON object.
  bool _deliver(String json) {
    final Object? decoded;
    try {
      decoded = jsonDecode(json);
    } on FormatException {
      logger.w('Ignoring an Apple Music event that does not parse');
      return false;
    }
    if (decoded is! Map<String, dynamic>) return false;
    var lost = false;
    var back = false;
    switch (decoded['type']) {
      case 'networkState':
        final online = decoded['online'];
        back = _online == false && online == true;
        if (online is bool) _online = online;
      case 'hostReady':
        final authorized = decoded['authorized'];
        lost = _authorized == true && authorized == false;
        if (authorized is bool) _authorized = authorized;
        _ready = decoded;
      // While the runner reports access, a sign-in would not let the track
      // play: the track shows as revoked, and nothing is asked. A batch whose
      // tracks fail one after another loses access once.
      case 'jukeboxItemFailed'
          when decoded['reason'] == 'notAuthorized' && _authorized != true:
        final revision = decoded['revision'];
        lost = revision == null || revision != _lostRevision;
        _lostRevision = revision;
    }
    _emit?.call(decoded);
    if (lost) _accessLost.add(null);
    if (back) _networkBack.add(null);
    return true;
  }
}
