import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/api/call_id.dart';
import 'package:rune/utils/api/create_mix.dart';
import 'package:rune/utils/api/create_playlist.dart';
import 'package:rune/utils/api/get_all_mixes.dart';
import 'package:rune/utils/api/get_all_playlists.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/mix/remove_mix_dialog.dart';
import 'package:rune/utils/dialogs/playlist/remove_playlist_dialog.dart';
import 'package:rune/widgets/band_screen_fallback_button.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late LibraryRegistryProvider registry;
  late FileSystemEntity database;
  setUpAll(() async {
    final root = await Directory.systemTemp.createTemp('rune-removal-');
    addTearDown(() => root.delete(recursive: true));
    await initializeRust(assignRustSignal);
    addTearDown(finalizeRust);
    await awaitResponse(
      'InitializeNodeRequest',
      InitializeNodeResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (id) => InitializeNodeRequest(
        configPath: root.path,
        callId: id,
      ).sendSignalToRust(),
    );
    registry = LibraryRegistryProvider();
    addTearDown(registry.dispose);
    await registry.ready;
    final music = await Directory('${root.path}/music').create();
    final registration = await registry.registerLocalFolder(
      music.path,
      'Removal test',
      LibraryInitializeMode.portable,
    );
    final recordId = registration.record!.recordId;
    addTearDown(() => registry.remove(recordId));
    await registry.open(recordId);
    database = await music
        .list(recursive: true)
        .where((file) => file is File && file.path.endsWith('.db'))
        .single;
  });

  for (final mix in [false, true]) {
    final request = mix ? 'RemoveMixRequest' : 'RemovePlaylistRequest';
    testWidgets('$request survives failure and resizing', (tester) async {
      final screen = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(screen, () async => null);
      addTearDown(screen.dispose);
      addTearDown(responsive.dispose);
      addTearDown(tester.view.reset);
      tester.view.devicePixelRatio = 1;
      late BuildContext home;
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: registry),
            ChangeNotifierProvider.value(value: responsive),
          ],
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: Builder(
              builder: (context) {
                home = context;
                return const SizedBox.expand();
              },
            ),
          ),
        ),
      );
      final labels = AppLocalizations.of(home)!;
      const ordinary = Size(900, 650);
      Future<void> resize(Size size) async {
        tester.view.physicalSize = size;
        screen.didChangeMetrics();
        await tester.pumpAndSettle();
      }

      for (final size in [
        const Size(900, 120),
        const Size(900, 200),
        const Size(100, 650),
        ordinary,
      ]) {
        final id = mix
            ? (await createMix('Test mix', 'Test', null, [])).id
            : (await createPlaylist('Test playlist', 'Test')).id;
        await resize(size);
        final result = mix
            ? showRemoveMixDialog(home, id)
            : showRemovePlaylistDialog(home, id);
        await tester.pumpAndSettle();

        final compact = size != ordinary;
        final remove = compact
            ? tester
                  .widget<BandScreenFallbackButton>(
                    find.byType(BandScreenFallbackButton).first,
                  )
                  .onPressed
            : tester
                  .widget<Button>(find.widgetWithText(Button, labels.delete))
                  .onPressed;
        expect(remove, isNotNull);
        final marker = nextCallId();
        final failures = <RequestFailedResponse>[];
        final markerSeen = Completer<void>();
        late bool actionsDisabled;
        final subscription = RequestFailedResponse.rustSignalStream.listen((
          signal,
        ) {
          if (signal.message.request != request) return;
          failures.add(signal.message);
          if (signal.message.callId == marker) markerSeen.complete();
        });
        // Keep the temporary library busy until both the UI and control
        // requests have received SQLite's typed write-lock failure.
        final writer = await Process.start('python3', [
          '-u',
          '-c',
          'import sqlite3, sys\n'
              'with sqlite3.connect(sys.argv[1]) as db:\n'
              '    db.execute("BEGIN IMMEDIATE")\n'
              '    print("locked")\n'
              '    sys.stdin.readline()',
          database.path,
        ]);
        try {
          expect(
            await writer.stdout
                .transform(utf8.decoder)
                .transform(const LineSplitter())
                .first,
            'locked',
          );
          remove!();
          remove();
          // The native handler reads requests on this channel in order.
          // The marker accounts for its own failure after all UI requests.
          if (mix) {
            RemoveMixRequest(mixId: id, callId: marker).sendSignalToRust();
          } else {
            RemovePlaylistRequest(
              playlistId: id,
              callId: marker,
            ).sendSignalToRust();
          }
          await tester.pump();
          actionsDisabled = compact
              ? tester
                    .widgetList<BandScreenFallbackButton>(
                      find.byType(BandScreenFallbackButton),
                    )
                    .every((button) => button.onPressed == null)
              : tester
                            .widget<Button>(
                              find.widgetWithText(Button, labels.delete),
                            )
                            .onPressed ==
                        null &&
                    tester
                            .widget<FilledButton>(
                              find.widgetWithText(FilledButton, labels.cancel),
                            )
                            .onPressed ==
                        null;
          await markerSeen.future.timeout(const Duration(seconds: 45));
        } finally {
          await writer.stdin.close();
          await writer.exitCode;
          await subscription.cancel();
        }
        expect(
          failures,
          hasLength(2),
          reason: '$request should send once at $size',
        );
        expect(
          actionsDisabled,
          isTrue,
          reason: '$request should disable its actions at $size',
        );
        expect(failures.first.kind, RequestErrorKind.unavailable);
        expect(failures.first.cause, isA<RequestFailureCauseStorageBusy>());
        await tester.pumpAndSettle();
        final error = find.text(labels.requestStorageBusy);
        expect(error.hitTestable(), findsOneWidget);
        await resize(compact ? ordinary : const Size(900, 120));
        expect(error.hitTestable(), findsOneWidget);
        await resize(size);
        await tester.tap(
          compact
              ? find.byType(BandScreenFallbackButton).first
              : find.widgetWithText(Button, labels.delete),
        );
        await tester.pumpAndSettle();
        expect(await result.timeout(const Duration(seconds: 15)), isTrue);
        final ids = mix
            ? (await getAllMixes()).map((item) => item.id)
            : (await getAllPlaylists()).map((item) => item.id);
        expect(ids, isNot(contains(id)));
      }
    }, timeout: const Timeout(Duration(minutes: 4)));
  }
}
