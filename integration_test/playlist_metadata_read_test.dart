import 'dart:async';
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
import 'package:rune/utils/api/create_playlist.dart';
import 'package:rune/utils/api/get_playlist_by_id.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/api/update_playlist.dart';
import 'package:rune/utils/dialogs/playlist/create_edit_playlist_dialog.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  late LibraryRegistryProvider registry;
  late String original;
  late String empty;
  setUpAll(() async {
    final root = await Directory.systemTemp.createTemp('rune-playlist-read-');
    addTearDown(() => root.delete(recursive: true));
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
    registry = LibraryRegistryProvider();
    addTearDown(registry.dispose);
    await registry.ready;
    Future<String> register(String name) async {
      final directory = await Directory('${root.path}/$name').create();
      final r = await registry.registerLocalFolder(
        directory.path,
        name,
        LibraryInitializeMode.portable,
      );
      final id = r.record!.recordId;
      addTearDown(() => registry.remove(id));
      return id;
    }

    original = await register('Original');
    empty = await register('Empty');
  });

  Future<(AppLocalizations, Completer<Playlist?>)> openEditor(
    WidgetTester tester,
    int? id,
  ) async {
    final screen = ScreenSizeProvider.withWriter(null, (_) async {});
    final responsive = ResponsiveProvider(screen, () async => null);
    addTearDown(screen.dispose);
    addTearDown(responsive.dispose);
    addTearDown(tester.view.reset);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(900, 650);
    screen.didChangeMetrics();
    final closed = Completer<Playlist?>();
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider.value(value: registry),
          ChangeNotifierProvider.value(value: responsive),
          ChangeNotifierProvider.value(value: screen),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: CreateEditPlaylistDialog(
            playlistId: id,
            defaultTitle: '',
            $close: closed.complete,
          ),
        ),
      ),
    );
    final labels = AppLocalizations.of(
      tester.element(find.byType(CreateEditPlaylistDialog)),
    )!;
    return (labels, closed);
  }

  for (final rename in [false, true]) {
    testWidgets('failed read protects the stored metadata (rename: $rename)', (
      tester,
    ) async {
      await registry.open(original);
      final stored = await createPlaylist('Morning walks', 'Outdoors');
      // While the original library is unavailable, this real lookup fails.
      await registry.open(empty);
      final failure = RequestFailedResponse.rustSignalStream.firstWhere(
        (s) => s.message.request == 'GetPlaylistByIdRequest',
      );
      final (labels, closed) = await openEditor(tester, stored.id);
      final failed = (await failure.timeout(
        const Duration(seconds: 15),
      )).message;
      expect(failed.kind, RequestErrorKind.notFound);
      await tester.pumpAndSettle();
      final failureMessage = requestFailureMessage(
        labels,
        RequestFailure(
          failed.request,
          failed.kind,
          failed.detail,
          failed.cause,
        ),
      );
      expect(find.text(failureMessage).hitTestable(), findsOneWidget);
      await registry.open(original);
      final control = await createPlaylist('Control', 'Control');
      final updated = <int>[];
      final updates = UpdatePlaylistResponse.rustSignalStream.listen(
        (signal) => updated.add(signal.message.playlist.id),
      );
      addTearDown(updates.cancel);
      if (rename) {
        await tester.enterText(find.byType(TextBox).first, 'Renamed only');
      }
      final save = find.widgetWithText(FilledButton, labels.save);
      expect(save.hitTestable(), findsOneWidget);
      final enabled = tester.widget<FilledButton>(save).onPressed != null;
      await tester.tap(save);
      await tester.pumpAndSettle();
      // The handler processes updates in order. The control response follows
      // any update submitted by the tap, even if the dialog stayed open.
      await updatePlaylist(control.id, control.name, control.group);
      final reloaded = await getPlaylistById(stored.id);
      expect(
        [reloaded.name, reloaded.group],
        [stored.name, stored.group],
        reason: 'a failed playlist read must preserve both stored fields',
      );
      expect(enabled, isFalse);
      expect(updated, [control.id]);
      await tester.tap(find.widgetWithText(Button, labels.cancel));
      expect(await closed.future.timeout(const Duration(seconds: 15)), isNull);
      expect(tester.takeException(), isNull);
    });
  }

  for (final edit in [false, true]) {
    testWidgets(
      edit
          ? 'a loaded playlist can retry a failed save'
          : 'a new playlist can be created',
      (tester) async {
        await registry.open(original);
        final stored = edit
            ? await createPlaylist('Morning walks', 'Outdoors')
            : null;
        final loaded = edit
            ? GetPlaylistByIdResponse.rustSignalStream.first
            : null;
        final (labels, closed) = await openEditor(tester, stored?.id);
        if (loaded != null) {
          expect((await loaded).message.playlist.id, stored!.id);
          await tester.pumpAndSettle();
          expect(find.text(stored.name), findsOneWidget);
          expect(find.text(stored.group), findsOneWidget);
        }
        final fields = find.byType(TextBox);
        await tester.enterText(fields.first, 'Evening walks');
        await tester.enterText(fields.last, 'After work');
        await tester.tap(fields.first);
        await tester.pumpAndSettle();
        final save = find.widgetWithText(
          FilledButton,
          edit ? labels.save : labels.create,
        );
        if (edit) {
          await registry.open(empty);
          final failure = RequestFailedResponse.rustSignalStream.firstWhere(
            (s) => s.message.request == 'UpdatePlaylistRequest',
          );
          await tester.tap(save);
          expect((await failure).message.kind, RequestErrorKind.notFound);
          await registry.open(original);
          await tester.pumpAndSettle();
        }
        await tester.tap(save);
        final result = await closed.future.timeout(const Duration(seconds: 15));
        expect(result, isNotNull);
        if (edit) expect(result!.id, stored!.id);
        final reloaded = await getPlaylistById(result!.id);
        expect(
          [reloaded.name, reloaded.group],
          ['Evening walks', 'After work'],
        );
      },
    );
  }
}
