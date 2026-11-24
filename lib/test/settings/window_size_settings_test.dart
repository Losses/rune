import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/screens/settings_theme/widgets/remember_window_size_setting_state.dart';
import 'package:rune/screens/settings_theme/widgets/window_size_setting.dart';
import 'package:rune/widgets/settings/settings_box_combo_box.dart';
import 'package:rune/widgets/settings/settings_box_toggle.dart';

Widget _app(Widget child) => FluentApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: ScaffoldPage(content: child),
);

void main() {
  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets(
      'window size settings render nothing on $platform',
      (tester) async {
        await tester.pumpWidget(
          _app(
            const Column(
              children: [WindowSizeSetting(), RememberWindowSizeSetting()],
            ),
          ),
        );

        expect(find.byType(SettingsBoxComboBox<String>), findsNothing);
        expect(find.byType(SettingsBoxToggle), findsNothing);
      },
      variant: TargetPlatformVariant.only(platform),
    );
  }
}
