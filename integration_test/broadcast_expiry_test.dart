import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/utils/api/get_sync_settings.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/storage_key_manager.dart';

class _Paths extends PathProviderPlatform {
  _Paths(this.path);
  final String path;

  @override
  Future<String> getApplicationDocumentsPath() async => path;

  @override
  Future<String> getApplicationSupportPath() async => path;
}

Future<void> _waitUntilStopped(BroadcastProvider provider) async {
  if (!provider.isBroadcasting) return;
  final stopped = Completer<void>();
  void listener() {
    if (!provider.isBroadcasting && !stopped.isCompleted) stopped.complete();
  }

  provider.addListener(listener);
  try {
    await stopped.future.timeout(const Duration(seconds: 8));
  } finally {
    provider.removeListener(listener);
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('timed broadcasts use the native cadence and stop at expiry', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final root = await Directory.systemTemp.createTemp('rune-broadcast');
      final previousPaths = PathProviderPlatform.instance;
      PathProviderPlatform.instance = _Paths(root.path);
      addTearDown(() async {
        PathProviderPlatform.instance = previousPaths;
        await root.delete(recursive: true);
      });
      StorageKeyManager.initialize('broadcast-expiry-test');
      await initializeRust(assignRustSignal);
      addTearDown(finalizeRust);
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => InitializeNodeRequest(
          configPath: root.path,
          callId: callId,
        ).sendSignalToRust(),
      );

      final socket = await RawDatagramSocket.bind(
        InternetAddress.anyIPv4,
        57863,
        reusePort: true,
      );
      socket.joinMulticast(InternetAddress('224.0.0.167'));
      socket.writeEventsEnabled = false;
      final announcements = socket
          .where((event) => event == RawSocketEvent.read)
          .expand((_) sync* {
            var packet = socket.receive();
            while (packet != null) {
              yield jsonDecode(utf8.decode(packet.data));
              packet = socket.receive();
            }
          })
          .where((message) => message is Map<String, dynamic>)
          .cast<Map<String, dynamic>>()
          .where((message) => message['announce'] == true)
          .asBroadcastStream();
      addTearDown(socket.close);

      final provider = BroadcastProvider();
      addTearDown(provider.dispose);
      Future<bool> announces(String alias, Duration within) => announcements
          .where((message) => message['alias'] == alias)
          .map((_) => true)
          .timeout(within, onTimeout: (sink) => sink.add(false))
          .first;

      final alias = root.uri.pathSegments.where((part) => part.isNotEmpty).last;
      final automatic = announces(alias, const Duration(seconds: 20));
      await provider.updateDeviceAlias(alias);
      await provider.startServer('127.0.0.1');
      expect(await automatic, isTrue, reason: 'server announces on startup');

      await provider.startBroadcast(5);
      expect(
        provider.remainingSeconds,
        0,
        reason: 'an already-running start must not arm a countdown',
      );
      expect(provider.isBroadcasting, isTrue);
      await provider.stopBroadcast();
      expect(provider.isBroadcasting, isFalse);

      final timedAlias = '$alias-timed';
      await provider.updateDeviceAlias(timedAlias);
      final clock = Stopwatch()..start();
      final cadence = announcements
          .where((message) => message['alias'] == timedAlias)
          .map((_) => clock.elapsedMilliseconds)
          .distinct((previous, next) => next - previous < 250)
          .timeout(const Duration(seconds: 10))
          .take(2)
          .toList();
      await provider.startBroadcast(5);
      expect(provider.isBroadcasting, isTrue);
      final ticks = await cadence;
      expect(
        ticks[1] - ticks[0],
        inInclusiveRange(2500, 4000),
        reason: 'distinct announcements should be three seconds apart',
      );
      await _waitUntilStopped(provider);
      expect(
        await announces(timedAlias, const Duration(seconds: 4)),
        isFalse,
        reason: 'native announcements cease after expiry',
      );

      final restarted = announces(timedAlias, const Duration(seconds: 10));
      await provider.startBroadcast(20);
      expect(provider.isBroadcasting, isTrue);
      expect(
        await restarted,
        isTrue,
        reason: 'native accepts restart after expiry',
      );
      await provider.stopBroadcast();
      expect(provider.isBroadcasting, isFalse);
      expect(
        await announces(timedAlias, const Duration(seconds: 4)),
        isFalse,
        reason: 'explicit stop also ends native announcements',
      );

      Future<void> startNativeBroadcast(int duration) async {
        await awaitResponse(
          'StartBroadcastRequest',
          StartBroadcastResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => StartBroadcastRequest(
            durationSeconds: duration,
            alias: timedAlias,
            callId: callId,
          ).sendSignalToRust(),
        );
      }

      await startNativeBroadcast(20);
      await expectLater(
        provider.startBroadcast(5),
        throwsA(
          isA<RequestFailure>().having(
            (error) => error.kind,
            'kind',
            RequestErrorKind.conflict,
          ),
        ),
      );
      expect(
        provider.isBroadcasting,
        isFalse,
        reason: 'Settings does not adopt a timed broadcast started elsewhere',
      );

      await awaitResponse(
        'StopBroadcastRequest',
        StopBroadcastResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            StopBroadcastRequest(callId: callId).sendSignalToRust(),
      );
      expect((await getSyncSettings()).listen, isTrue);
      await provider.startBroadcast(20);
      expect(provider.isBroadcasting, isTrue);
      provider.stopServer();
      await _waitUntilStopped(provider);
      expect(provider.isServerRunning, isFalse);
      // A new native start would conflict if the sync listener were announcing.
      await startNativeBroadcast(1);
    });
  });
}
