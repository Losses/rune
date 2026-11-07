import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_playback/widgets/mix_with_others_setting.dart';
import 'package:rune/widgets/settings/settings_box_toggle.dart';

Widget _app() {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
      ChangeNotifierProvider<ResponsiveProvider>(
        create: (_) => ResponsiveProvider(sizes, () async => null),
      ),
    ],
    child: const FluentApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: ScaffoldPage(content: MixWithOthersSetting()),
    ),
  );
}

void main() {
  for (final platform in TargetPlatform.values) {
    final iOS = platform == TargetPlatform.iOS;
    testWidgets(
      iOS
          ? 'the setting is shown on iOS'
          : 'the setting is hidden on ${platform.name}',
      (tester) async {
        await tester.pumpWidget(_app());
        // Lets the screen size provider save the window size it read.
        await tester.pump(const Duration(seconds: 2));
        expect(
          find.byType(SettingsBoxToggle),
          iOS ? findsOneWidget : findsNothing,
        );
      },
      variant: TargetPlatformVariant.only(platform),
    );
  }
}
