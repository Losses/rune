import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/create_mix.dart';
import 'package:rune/utils/api/get_mix_by_id.dart';
import 'package:rune/utils/dialogs/mix/mix_studio_dialog.dart';
import 'package:rune/utils/dialogs/mix/widgets/mix_editor.dart';
import 'package:rune/utils/dialogs/mix/widgets/select_input_section.dart';
import 'package:rune/widgets/playback_controller/playback_mode_button.dart';

import 'mix_studio_test_support.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  late LibraryRegistryProvider registry;
  setUpAll(() async {
    registry = (await openMixLibrary()).registry;
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
    final closed = await openMixStudio(tester, registry, mixId: original.id);
    final watch = Stopwatch()..start();
    final context = tester.element(find.byType(MixStudioDialog));
    final labels = AppLocalizations.of(context)!;
    final save = find.widgetWithText(FilledButton, labels.save);
    final title = find.byWidgetPredicate(
      (w) => w is EditableText && w.controller.text == original.name,
    );
    while ((title.evaluate().isEmpty ||
            tester.widget<FilledButton>(save).onPressed == null) &&
        watch.elapsed < const Duration(seconds: 15)) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(title, findsOneWidget);
    expect(tester.widget<FilledButton>(save).onPressed, isNotNull);
    await tester.enterText(title, 'Evening walks');
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
    await tester.tap(save);
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
