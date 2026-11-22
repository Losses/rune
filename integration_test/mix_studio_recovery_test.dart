import 'dart:convert';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/utils/api/get_all_mixes.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/mix/mix_studio_dialog.dart';
import 'package:rune/utils/dialogs/mix/widgets/mix_editor.dart';

import 'mix_studio_test_support.dart';
import 'runtime_probe_support.dart';

Future<void> withLockedDatabase(
  FileSystemEntity database,
  Future<void> Function() run,
) async {
  // A temporary exclusive lock prevents both preview reads and mix writes.
  final writer = await Process.start('python3', [
    '-u',
    '-c',
    'import sqlite3, sys\n'
        'with sqlite3.connect(sys.argv[1]) as db:\n'
        '    db.execute("BEGIN EXCLUSIVE")\n'
        '    print("locked")\n'
        '    sys.stdin.readline()',
    database.path,
  ]);
  try {
    expect(
      await writer.stdout
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .first
          .timeout(const Duration(seconds: 15)),
      'locked',
    );
    await run();
  } finally {
    await writer.stdin.close();
    expect(await writer.exitCode.timeout(const Duration(seconds: 15)), 0);
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  late FileSystemEntity database;
  setUpAll(() async {
    final music = await openMixLibrary();
    database = await music
        .list(recursive: true)
        .where((entry) => entry is File && entry.path.endsWith('.db'))
        .single;
  });

  for (final failPreview in [true, false]) {
    final request = failPreview ? 'MixQueryRequest' : 'CreateMixRequest';
    testWidgets('a failed $request keeps the same editable draft', (
      tester,
    ) async {
      final closed = await openMixStudio(tester);
      final dialog = tester.element(find.byType(MixStudioDialog));
      final labels = AppLocalizations.of(dialog)!;
      final editor = find.byType(MixEditor);
      final controller = tester.widget<MixEditor>(editor).controller!;
      final create = find.widgetWithText(FilledButton, labels.create);
      final retry = find.widgetWithText(Button, labels.playOnRetry);
      final title = 'Evening walks after $request';
      Finder field(TextEditingController target) => find.byWidgetPredicate(
        (widget) =>
            widget is EditableText && identical(widget.controller, target),
      );
      Future<void> enter(TextEditingController target, String value) async {
        final input = field(target);
        await tester.ensureVisible(input);
        await tester.tap(input);
        await tester.pump();
        await tester.enterText(input, value);
        await tester.pump();
        expect(target.text, value);
      }

      var previewCount = 0;
      final previews = MixQueryResponse.rustSignalStream.listen(
        (_) => previewCount++,
      );
      addTearDown(previews.cancel);
      final failures = <RequestFailedResponse>[];
      final errors = RequestFailedResponse.rustSignalStream.listen(
        (s) => failures.add(s.message),
      );
      addTearDown(errors.cancel);
      controller.randomTrackController.text = '1';
      await enter(controller.titleController, title);
      await enter(controller.groupController, 'Outdoors');
      await probeWait(tester, () => previewCount > 0, 'the initial preview');

      await withLockedDatabase(database, () async {
        if (failPreview) {
          await enter(controller.groupController, 'Outdoors edited');
        } else {
          await tester.tap(create);
        }
        await probeWait(
          tester,
          () => failures.any((f) => f.request == request),
          'the $request failure',
        );
      });
      await tester.pump();
      expect(
        editor,
        findsOneWidget,
        reason: 'The draft must remain editable after $request fails.',
      );
      expect(tester.widget<MixEditor>(editor).controller, same(controller));
      expect(controller.titleController.text, title);
      expect(
        controller.groupController.text,
        failPreview ? 'Outdoors edited' : 'Outdoors',
      );
      final response = failures.firstWhere((f) => f.request == request);
      final message = requestFailureMessage(
        labels,
        RequestFailure(request, response.kind, response.detail, response.cause),
      );
      expect(find.text(message), findsWidgets);
      expect(tester.widget<FilledButton>(create).onPressed, isNotNull);

      if (failPreview) {
        final before = previewCount;
        await tester.tap(retry);
        await probeWait(
          tester,
          () => previewCount > before && retry.evaluate().isEmpty,
          'a successful preview retry',
        );
      }
      await enter(controller.groupController, 'Recovered');
      await tester.tap(create);
      final saved = await closed.future.timeout(const Duration(seconds: 15));
      expect(saved?.name, title);
      final stored = (await getAllMixes()).where((mix) => mix.name == title);
      expect(stored, hasLength(1));
      expect(stored.single.group, 'Recovered');
      expect(stored.single.id, saved!.id);
    });
  }

  testWidgets('a failed initial mix read cannot save empty defaults', (
    tester,
  ) async {
    final before = await getAllMixes();
    final closed = await openMixStudio(tester, mixId: -1);
    final labels = AppLocalizations.of(
      tester.element(find.byType(MixStudioDialog)),
    )!;
    final cancel = find.widgetWithText(Button, labels.cancel);
    await probeWait(
      tester,
      () =>
          find.byType(MixEditor).evaluate().isEmpty &&
          tester.widget<Button>(cancel).onPressed != null,
      'the initial read failure',
    );
    expect(
      tester
          .widget<FilledButton>(find.widgetWithText(FilledButton, labels.save))
          .onPressed,
      isNull,
    );
    await tester.tap(cancel);
    expect(await closed.future.timeout(const Duration(seconds: 15)), isNull);
    expect((await getAllMixes()).map((m) => m.id), before.map((m) => m.id));
  });
}
