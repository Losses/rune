import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';
import '../utils/dialogs/mix/config/mode_select_items.dart';
import '../utils/dialogs/mix/widgets/mix_editor_controller.dart';
import '../utils/dialogs/mix/widgets/select_input_section.dart';
import '../widgets/playback_controller/playback_mode_button.dart';

void main() {
  testWidgets(
    'every playback mode the editor offers is the one it is labelled as',
    (tester) async {
      late List<SelectItem> items;
      late Map<PlaybackMode, String> labels;
      await tester.pumpWidget(
        FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Builder(
            builder: (inner) {
              items = modeSelectItems(inner);
              labels = {
                for (final mode in PlaybackMode.values)
                  mode: modeToLabel(inner, mode),
              };
              return const SizedBox();
            },
          ),
        ),
      );
      final controller = MixEditorController();
      addTearDown(controller.dispose);

      // Left alone, a mix keeps the player's mode.
      expect(controller.getData().mode, isNull);

      for (final mode in PlaybackMode.values) {
        final label = labels[mode]!;
        final offered = items.where((item) => item.title == label);
        if (offered.isEmpty) continue;
        controller.modeController.selectedValue = offered.single.value;
        expect(controller.getData().mode, mode, reason: label);
      }
      // "Repeat all" is offered, and it is repeat all, not repeat one.
      controller.modeController.selectedValue = items
          .singleWhere((item) => item.title == labels[PlaybackMode.repeatAll])
          .value;
      expect(controller.getData().mode, PlaybackMode.repeatAll);

      controller.modeController.selectedValue = keepModeSelectValue;
      expect(controller.getData().mode, isNull);
    },
  );
}
