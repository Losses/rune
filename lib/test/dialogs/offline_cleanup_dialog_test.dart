import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../l10n/app_localizations.dart';
import '../../utils/context_menu/offline_selection.dart';

void main() {
  testWidgets('offline cleanup dialog confirms the selected option', (
    tester,
  ) async {
    bool? result;
    await tester.pumpWidget(
      FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: OfflineCleanupDialog(
          initialValue: true,
          onClose: (value) => result = value,
        ),
      ),
    );
    expect(tester.widget<Checkbox>(find.byType(Checkbox)).checked, isTrue);
    await tester.tap(find.byType(Checkbox));
    await tester.pump();
    await tester.tap(find.text('Confirm'));
    expect(result, isFalse);
    await tester.pump(const Duration(milliseconds: 200));
  });
}
