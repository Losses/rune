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
import 'package:rune/utils/api/create_mix.dart';
import 'package:rune/utils/api/get_mix_by_id.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/mix/mix_studio_dialog.dart';
import 'package:rune/utils/dialogs/mix/widgets/mix_editor.dart';
import 'package:rune/utils/dialogs/mix/widgets/select_input_section.dart';
import 'package:rune/widgets/playback_controller/playback_mode_button.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final root = await Directory.systemTemp.createTemp('rune-mix-mode-');
    addTearDown(() => root.delete(recursive: true));
    await initializeRust(assignRustSignal);
    addTearDown(finalizeRust);
    await awaitResponse(
      'InitializeNodeRequest',
      InitializeNodeResponse.rustSignalStream,
      responseCallId: (signal) => signal.message.callId?.toInt(),
      send: (id) => InitializeNodeRequest(
        configPath: root.path,
        callId: id,
      ).sendSignalToRust(),
    );
    final registry = LibraryRegistryProvider();
    addTearDown(registry.dispose);
    await registry.ready;
    final music = await Directory('${root.path}/Music').create();
    final record = (await registry.registerLocalFolder(
      music.path,
      'Music',
      LibraryInitializeMode.portable,
    )).record!;
    addTearDown(() => registry.remove(record.recordId));
    await registry.open(record.recordId);
  });

  Future<void> editMix(
    WidgetTester tester,
    PlaybackMode? initialMode, {
    bool choose = false,
    PlaybackMode? choice,
  }) async {
    final original = await createMix(
      'Morning walks',
      'Outdoors',
      initialMode,
      [],
    );
    expect((await getMixById(original.id)).mode, initialMode);
    final screen = ScreenSizeProvider.withWriter(null, (_) async {});
    final responsive = ResponsiveProvider(screen, () async => null);
    addTearDown(screen.dispose);
    addTearDown(responsive.dispose);
    addTearDown(tester.view.reset);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(900, 700);
    screen.didChangeMetrics();
    final closed = Completer<Mix?>();
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider.value(value: screen),
          ChangeNotifierProvider.value(value: responsive),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: RevealEffectContext(
            child: MixStudioDialog(mixId: original.id, $close: closed.complete),
          ),
        ),
      ),
    );
    final watch = Stopwatch()..start();
    final title = find.byWidgetPredicate(
      (w) => w is EditableText && w.controller.text == original.name,
    );
    while (title.evaluate().isEmpty &&
        watch.elapsed < const Duration(seconds: 15)) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(title, findsOneWidget);
    await tester.enterText(title, 'Evening walks');
    final context = tester.element(find.byType(MixStudioDialog));
    final labels = AppLocalizations.of(context)!;
    final section = find.byWidgetPredicate(
      (w) => w is SelectInputSection && w.title == labels.mode,
    );
    final list = find
        .descendant(
          of: find.byType(MixEditor),
          matching: find.byType(Scrollable),
        )
        .first;
    await tester.scrollUntilVisible(section, 150, scrollable: list);
    final picker = find.descendant(
      of: section,
      matching: find.byType(ComboBox<String>),
    );
    final visibleMode = initialMode == null
        ? labels.defaultMode
        : modeToLabel(context, initialMode);
    final initialLabelVisible = find
        .descendant(of: picker, matching: find.text(visibleMode))
        .hitTestable()
        .evaluate()
        .isNotEmpty;
    final options = tester.widget<ComboBox<String>>(picker).items!;
    expect(options, hasLength(initialMode == PlaybackMode.repeatOne ? 5 : 4));
    if (choose) {
      final label = choice == null
          ? labels.defaultMode
          : modeToLabel(context, choice);
      await tester.tap(picker);
      await tester.pumpAndSettle();
      await tester.tap(find.text(label).hitTestable().last);
      await tester.pumpAndSettle();
    }
    await tester.tap(find.widgetWithText(FilledButton, labels.save));
    expect(
      (await closed.future.timeout(const Duration(seconds: 15)))?.id,
      original.id,
    );
    final reopened = await getMixById(original.id);
    expect(reopened.name, 'Evening walks');
    expect(
      reopened.mode,
      choose ? choice : initialMode,
      reason:
          'Reloading the saved mix should reflect the mode chosen in the editor.',
    );
    expect(
      initialLabelVisible,
      isTrue,
      reason: 'Opening the mix should show its stored mode.',
    );
  }

  for (final mode in [null, ...PlaybackMode.values]) {
    testWidgets(
      'renaming a saved mix preserves its playback mode ($mode)',
      (tester) => editMix(tester, mode),
    );
  }
  testWidgets(
    'a saved mix can switch to shuffle',
    (tester) => editMix(
      tester,
      PlaybackMode.sequential,
      choose: true,
      choice: PlaybackMode.shuffle,
    ),
  );
  testWidgets(
    'a saved mix can return to the player mode',
    (tester) => editMix(tester, PlaybackMode.repeatOne, choose: true),
  );
}
