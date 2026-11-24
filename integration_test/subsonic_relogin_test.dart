import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/get_all_playlists.dart';
import 'package:rune/utils/api/library_registry.dart';
import 'package:rune/utils/api/request_failure.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('a playlist request recovers after re-login without reopening', (
    tester,
  ) async {
    await tester.runAsync(() async {
      var password = 'old-password';
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      server.listen((request) async {
        final query = request.uri.queryParameters;
        final token = md5.convert(utf8.encode(password + (query['s'] ?? '')));
        final ok = query['u'] == 'alice' && query['t'] == token.toString();
        final response = {
          'status': ok ? 'ok' : 'failed',
          'version': '1.16.1',
          if (!ok)
            'error': {'code': 40, 'message': 'Credentials rejected'}
          else if (request.uri.path.endsWith('/getUser.view'))
            'user': {'streamRole': true}
          else if (request.uri.path.endsWith('/getPlaylists.view'))
            'playlists': {
              'playlist': [
                {'id': 'road', 'name': 'Road'},
              ],
            },
        };
        request.response.headers.contentType = ContentType.json;
        request.response.write(jsonEncode({'subsonic-response': response}));
        await request.response.close();
      });

      final root = await Directory.systemTemp.createTemp('rune-relogin-');
      addTearDown(() async {
        if (!Platform.isWindows) await root.delete(recursive: true);
      });
      await initializeRust(assignRustSignal);
      addTearDown(finalizeRust);
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (r) => r.message.callId?.toInt(),
        send: (id) => InitializeNodeRequest(
          configPath: root.path,
          callId: id,
        ).sendSignalToRust(),
      );
      final registry = LibraryRegistryProvider();
      addTearDown(registry.dispose);
      await registry.ready;
      final registration = await registerSubsonicLibrary(
        'http://127.0.0.1:${server.port}',
        'alice',
        password,
        'Loopback',
        SubsonicOpenModeMessage.playOnThisDevice,
      );
      final record = registration.record!.recordId;
      await registry.open(record);
      final generation = registry.openGeneration;
      final before = (await getAllPlaylists()).single;
      expect(before.name, 'Road');

      password = 'new-password';
      await expectLater(
        getAllPlaylists(),
        throwsA(
          isA<RequestFailure>().having(
            (e) => e.cause,
            'cause',
            isA<RequestFailureCauseSubsonicNotAuthorized>(),
          ),
        ),
      );
      await registry.replaceSubsonicCredential(record, 'alice', password);
      final after = (await getAllPlaylists()).single;
      expect(after.name, before.name);
      expect(after.id, before.id);
      expect(registry.openGeneration, generation);
    });
  });
}
