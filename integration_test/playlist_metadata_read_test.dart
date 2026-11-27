import 'dart:async';
import 'dart:io';

import 'package:fast_file_picker/fast_file_picker.dart';
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
import 'package:rune/utils/api/get_all_playlists.dart';
import 'package:rune/utils/api/get_playlist_by_id.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/api/update_playlist.dart';
import 'package:rune/utils/dialogs/playlist/create_edit_playlist_dialog.dart';
import 'package:rune/utils/dialogs/playlist/import_m3u8_playlist.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  late LibraryRegistryProvider registry;
  late Directory root;
  late String original;
  late String empty;
  late String editLibrary;
  late String otherLibrary;
  late String importLibrary;
  setUpAll(() async {
    root = await Directory.systemTemp.createTemp('rune-playlist-read-');
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
    editLibrary = await register('Edit source');
    otherLibrary = await register('Other');
    importLibrary = await register('Import');
  });

  Future<void> withUnavailableTable(
    String library,
    Future<void> Function() run,
  ) async {
    final database = await Directory('${root.path}/$library')
        .list(recursive: true)
        .where((entry) => entry is File && entry.path.endsWith('.db'))
        .single;
    Future<void> sql(String statement) async {
      final result = await Process.run('python3', [
        '-c',
        'import sqlite3, sys\n'
            'with sqlite3.connect(sys.argv[1]) as db: db.execute(sys.argv[2])',
        database.path,
        statement,
      ]);
      expect(result.exitCode, 0, reason: result.stderr.toString());
    }

    // Keep every row intact while the dialog receives a real storage error.
    await sql('ALTER TABLE playlists RENAME TO fixture_playlists');
    try {
      await run();
    } finally {
      await sql('ALTER TABLE fixture_playlists RENAME TO playlists');
    }
  }

  Future<(AppLocalizations, Completer<Playlist?>)> openEditor(
    WidgetTester tester,
    int? id, {
    LibraryRegistryProvider? formRegistry,
    FastFilePickerPath? pickedPlaylist,
  }) async {
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
          ChangeNotifierProvider.value(value: formRegistry ?? registry),
          ChangeNotifierProvider.value(value: responsive),
          ChangeNotifierProvider.value(value: screen),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: pickedPlaylist == null
              ? CreateEditPlaylistDialog(
                  playlistId: id,
                  defaultTitle: '',
                  $close: closed.complete,
                )
              : Builder(
                  builder: (context) => Button(
                    onPressed: () async {
                      closed.complete(
                        await showCreateImportM3u8PlaylistDialog(
                          context,
                          pickedPlaylist,
                        ),
                      );
                    },
                    child: const Text('Open picked playlist'),
                  ),
                ),
        ),
      ),
    );
    if (pickedPlaylist != null) {
      await tester.tap(find.text('Open picked playlist'));
      await tester.pumpAndSettle();
    }
    final labels = AppLocalizations.of(
      tester.element(find.byType(CreateEditPlaylistDialog)),
    )!;
    return (labels, closed);
  }

  testWidgets('an unreadable document cannot create a blank playlist', (
    tester,
  ) async {
    await registry.open(importLibrary);
    final before = await getAllPlaylists();
    final (labels, closed) = await openEditor(
      tester,
      null,
      pickedPlaylist: FastFilePickerPath.fromUri(
        'Unreadable.m3u8',
        'content://rune.test.invalid/Unreadable.m3u8',
      ),
    );
    final failure = RequestFailedResponse.rustSignalStream.firstWhere(
      (signal) => signal.message.request == 'CreateM3u8PlaylistRequest',
    );
    await tester.tap(find.widgetWithText(FilledButton, labels.create));
    final reply = (await failure.timeout(const Duration(seconds: 10))).message;
    await tester.pumpAndSettle();
    final message = requestFailureMessage(
      labels,
      RequestFailure(reply.request, reply.kind, reply.detail, reply.cause),
    );
    expect(find.text(message).hitTestable(), findsOneWidget);
    expect(find.text(labels.importM3u8Failed).hitTestable(), findsOneWidget);
    expect(await getAllPlaylists(), before);
    expect(closed.isCompleted, isFalse);
    await tester.tap(find.widgetWithText(Button, labels.cancel));
    await tester.pumpAndSettle();
    expect(await closed.future, isNull);
    expect(await getAllPlaylists(), before);
  });

  testWidgets('cancelling a picked file does not create a playlist', (
    tester,
  ) async {
    await registry.open(importLibrary);
    final before = await getAllPlaylists();
    final path = File('${root.path}/selected.m3u8');
    await path.writeAsString('#EXTM3U\nnot-in-the-library.ogg\n');
    Future<(AppLocalizations, Completer<Playlist?>)> openImport() => openEditor(
      tester,
      null,
      pickedPlaylist: FastFilePickerPath.fromPath('Selected.m3u8', path.path),
    );
    final (labels, cancelled) = await openImport();
    await tester.tap(find.widgetWithText(Button, labels.cancel));
    await tester.pumpAndSettle();
    expect(await cancelled.future, isNull);
    expect(await getAllPlaylists(), before);

    await tester.pumpWidget(const SizedBox());
    final (_, imported) = await openImport();
    final import = CreateM3u8PlaylistResponse.rustSignalStream.first;
    await tester.tap(find.widgetWithText(FilledButton, labels.create));
    final reply = (await import.timeout(const Duration(seconds: 10))).message;
    expect(reply.notFoundPaths, ['not-in-the-library.ogg']);
    expect(
      (await imported.future.timeout(const Duration(seconds: 10)))?.name,
      'Selected.m3u8',
    );
  });

  testWidgets(
    'a loaded edit cannot write to another library with the same ID',
    (tester) async {
      await registry.open(otherLibrary);
      final other = await createPlaylist('Garden', 'Other library');
      await registry.open(editLibrary);
      final stored = await createPlaylist('Morning walks', 'Outdoors');
      expect(stored.id, other.id);
      final loaded = GetPlaylistByIdResponse.rustSignalStream.first;
      final (labels, closed) = await openEditor(tester, stored.id);
      await loaded;
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextBox).first, 'Evening walks');
      final save = find.widgetWithText(FilledButton, labels.save);
      final pendingTap = tester.widget<FilledButton>(save).onPressed!;
      await registry.open(otherLibrary);
      // A callback captured before the rebuild still targets the loaded record.
      pendingTap();
      await tester.pumpAndSettle();
      final control = await createPlaylist('Control', 'Control');
      await updatePlaylist(control.id, control.name, control.group);
      final untouched = await getPlaylistById(other.id);
      expect(
        [untouched.name, untouched.group],
        [other.name, other.group],
        reason: 'the same numeric ID belongs to a different library',
      );
      expect(tester.widget<FilledButton>(save).onPressed, isNull);

      await registry.open(editLibrary);
      await tester.pumpAndSettle();
      await tester.tap(save);
      expect(
        (await closed.future.timeout(const Duration(seconds: 15)))?.name,
        'Evening walks',
      );
    },
  );

  testWidgets('Save waits for every pending library open', (tester) async {
    await registry.open(original);
    final stored = await createPlaylist('Walks', 'Outdoors');
    final requests = <Completer<OpenLibraryResponse>>[];
    final formRegistry = LibraryRegistryProvider(
      openRecord: (_, _) {
        final response = Completer<OpenLibraryResponse>();
        requests.add(response);
        return response.future;
      },
    );
    addTearDown(formRegistry.dispose);
    await formRegistry.ready;
    final loaded = GetPlaylistByIdResponse.rustSignalStream.first;
    final (labels, _) = await openEditor(
      tester,
      stored.id,
      formRegistry: formRegistry,
    );
    await loaded;
    await tester.pumpAndSettle();
    final save = find.widgetWithText(FilledButton, labels.save);
    expect(tester.widget<FilledButton>(save).onPressed, isNotNull);

    final failedOpen = expectLater(formRegistry.open(empty), throwsStateError);
    final reopen = formRegistry.open(original);
    await tester.pumpAndSettle();
    final duringBoth = tester.widget<FilledButton>(save).onPressed;
    requests.first.completeError(StateError('Open failed'));
    await failedOpen;
    await tester.pumpAndSettle();
    final duringReopen = tester.widget<FilledButton>(save).onPressed;
    requests.last.complete(
      OpenLibraryResponse(
        recordId: original,
        generation: Uint64.fromBigInt(
          BigInt.from(formRegistry.openGeneration! + 1),
        ),
      ),
    );
    await reopen;
    await tester.pumpAndSettle();
    expect(duringBoth, isNull);
    expect(duringReopen, isNull);
    expect(tester.widget<FilledButton>(save).onPressed, isNotNull);
    expect(find.text(stored.name), findsOneWidget);
  });

  for (final openBeforeMount in [false, true]) {
    testWidgets(
      'a pending open defers the initial read (open before mount: $openBeforeMount)',
      (tester) async {
        await registry.open(original);
        final stored = await createPlaylist('Evening walk', 'Outdoors');
        final control = await createPlaylist('Control', 'Control');
        final acknowledgement = Completer<OpenLibraryResponse>();
        final formRegistry = LibraryRegistryProvider(
          openRecord: (_, _) => acknowledgement.future,
        );
        addTearDown(formRegistry.dispose);
        await formRegistry.ready;
        var reads = 0;
        final started = Completer<void>();
        late Future<void> failedOpen;
        void startOpen() {
          failedOpen = expectLater(formRegistry.open(empty), throwsStateError);
          started.complete();
        }

        final responses = GetPlaylistByIdResponse.rustSignalStream.listen((
          signal,
        ) {
          if (signal.message.playlist.id != stored.id) return;
          reads++;
          if (!started.isCompleted) startOpen();
        });
        addTearDown(responses.cancel);
        if (openBeforeMount) startOpen();
        final (labels, _) = await openEditor(
          tester,
          stored.id,
          formRegistry: formRegistry,
        );
        await started.future.timeout(const Duration(seconds: 10));
        await tester.pumpAndSettle();
        final save = find.widgetWithText(FilledButton, labels.save);
        final form = tester.state<CreateEditPlaylistDialogState>(
          find.byType(CreateEditPlaylistDialog),
        );
        final title = find.byWidgetPredicate(
          (widget) =>
              widget is EditableText &&
              widget.controller == form.titleController,
        );
        final whilePending = tester.widget<FilledButton>(save).onPressed;
        if (openBeforeMount) {
          await tester.enterText(title, 'Typed while waiting');
        }
        acknowledgement.completeError(StateError('Open failed'));
        await failedOpen;
        await tester.pumpAndSettle();
        // The control response follows reads issued by the rebuilt editor.
        await getPlaylistById(control.id);
        await tester.pumpAndSettle();
        expect(whilePending, isNull);
        expect(reads, openBeforeMount ? 1 : 2);
        expect(
          find.text(openBeforeMount ? 'Typed while waiting' : stored.name),
          findsOneWidget,
        );
        expect(find.text(stored.group), findsOneWidget);
        expect(tester.widget<FilledButton>(save).onPressed, isNotNull);
        final draftTitle = openBeforeMount
            ? 'Typed while waiting'
            : 'Edited draft';
        if (!openBeforeMount) {
          await tester.enterText(title, draftTitle);
          await tester.pump();
        }
        expect(form.titleController.text, draftTitle);
        await formRegistry.refresh();
        await tester.pumpAndSettle();
        await getPlaylistById(control.id);
        expect(reads, openBeforeMount ? 1 : 2);
        expect(form.titleController.text, draftTitle);
      },
    );
  }

  testWidgets('a late group failure keeps the playlist read error visible', (
    tester,
  ) async {
    await registry.open(empty);
    final missing = RequestFailedResponse.rustSignalStream.firstWhere(
      (s) => s.message.request == 'GetPlaylistByIdRequest',
    );
    final (labels, _) = await openEditor(tester, 99);
    expect((await missing).message.kind, RequestErrorKind.notFound);
    await tester.pumpAndSettle();
    expect(find.text(labels.requestItemMissing), findsOneWidget);
    final groups = RequestFailedResponse.rustSignalStream.firstWhere(
      (s) => s.message.request == 'FetchCollectionGroupSummaryRequest',
    );
    final state = tester.state<CreateEditPlaylistDialogState>(
      find.byType(CreateEditPlaylistDialog),
    );
    await withUnavailableTable('Empty', state.fetchGroupList);
    final groupFailure = (await groups.timeout(
      const Duration(seconds: 10),
    )).message;
    expect(groupFailure.kind, isNot(RequestErrorKind.notFound));
    await tester.pumpAndSettle();
    expect(
      find.text(labels.requestItemMissing),
      findsOneWidget,
      reason: 'playlist metadata is required, group suggestions are optional',
    );
  });

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
          final failure = RequestFailedResponse.rustSignalStream.firstWhere(
            (s) => s.message.request == 'UpdatePlaylistRequest',
          );
          await withUnavailableTable('Original', () async {
            await tester.tap(save);
            expect((await failure).message.request, 'UpdatePlaylistRequest');
          });
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
