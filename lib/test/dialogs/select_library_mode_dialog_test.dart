import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/utils/dialogs/select_library_mode/select_library_mode_dialog.dart';
import 'package:rune/widgets/subtitle_button.dart';

void main() {
  testWidgets('library mode dialog returns the selected enum', (tester) async {
    final choices = <LibraryInitializeMode?>[];
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: SelectLibraryModeDialog(onClose: choices.add),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byType(SubtitleButton).at(0));
    expect(choices, [LibraryInitializeMode.portable]);
    choices.clear();
    await tester.tap(find.byType(SubtitleButton).at(1));
    expect(choices, [LibraryInitializeMode.redirected]);
    await tester.pump(const Duration(milliseconds: 150));
  });
}
