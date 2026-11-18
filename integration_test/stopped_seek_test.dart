import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/test/widgets/seek_progress_controls_test.dart';
import 'package:rune/utils/api/call_id.dart';
import 'package:rune/utils/api/request_failure.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  registerSeekProgressControlsTest(
    verifyNoSeek: (tester, attempt) async {
      final marker = nextCallId();
      final failures = <RequestFailedResponse>[];
      late final Completer<void> markerSeen;
      await tester.runAsync(() async {
        final directory = await Directory.systemTemp.createTemp('rune-seek');
        addTearDown(() async {
          if (!Platform.isWindows) await directory.delete(recursive: true);
        });
        await initializeRust(assignRustSignal);
        addTearDown(finalizeRust);
        await awaitResponse(
          'InitializeNodeRequest',
          InitializeNodeResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => InitializeNodeRequest(
            configPath: directory.path,
            callId: callId,
          ).sendSignalToRust(),
        );
        final music = await Directory('${directory.path}/music').create();
        final library = await awaitResponse(
          'RegisterLocalLibraryRequest',
          RegisterLibraryResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => RegisterLocalLibraryRequest(
            path: music.path,
            alias: 'Seek test',
            mode: LibraryInitializeMode.portable,
            callId: callId,
          ).sendSignalToRust(),
        );
        await awaitResponse(
          'OpenLibraryRequest',
          OpenLibraryResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => OpenLibraryRequest(
            recordId: library.message.record!.recordId,
            callId: callId,
          ).sendSignalToRust(),
        );
        markerSeen = Completer<void>();
        final subscription = RequestFailedResponse.rustSignalStream.listen((
          signal,
        ) {
          final failure = signal.message;
          if (failure.request != 'SeekRequest') return;
          failures.add(failure);
          if (failure.callId == marker && !markerSeen.isCompleted) {
            markerSeen.complete();
          }
        });
        addTearDown(subscription.cancel);
      });

      await attempt();
      await tester.runAsync(() async {
        // listen_gui_with_io awaits each request before reading the next one.
        // Every seek fails with an empty player, so any UI seek would report
        // its failure on this same stream before the control request.
        SeekRequest(positionSeconds: 5, callId: marker).sendSignalToRust();
        await markerSeen.future.timeout(const Duration(seconds: 10));
      });
      expect(failures.map((failure) => failure.callId), [
        marker,
      ], reason: 'only the control request may reach the stopped player');
      expect(failures.single.kind, RequestErrorKind.invalidInput);
    },
  );
}
