import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/screens/settings_laboratory/widgets/settings/branding_animation_settings.dart';
import 'package:rune/screens/settings_laboratory/widgets/settings_combo_box.dart';
import 'package:rune/widgets/banding_animation/branding_animation.dart';

void main() {
  for (final platform in TargetPlatform.values) {
    final iOS = platform == TargetPlatform.iOS;
    testWidgets(
      iOS
          ? 'no startup sound plays on iOS until one is chosen'
          : 'the startup sound plays on ${platform.name} until another is chosen',
      (tester) async {
        expect(startUpSfxAsset(null), iOS ? isNull : 'assets/startup_1.ogg');
      },
      variant: TargetPlatformVariant.only(platform),
    );

    testWidgets(
      'the laboratory shows the default startup sound on ${platform.name}',
      (tester) async {
        await tester.pumpWidget(
          const FluentApp(
            home: ScaffoldPage(content: BrandingAnimationSettings()),
          ),
        );
        final setting = tester.widget<SettingsComboBox<String>>(
          find.byType(SettingsComboBox<String>),
        );
        expect(setting.value, iOS ? 'mute' : 'tech');
        expect(startUpSfxAsset(setting.value), startUpSfxAsset(null));
      },
      variant: TargetPlatformVariant.only(platform),
    );
  }

  testWidgets(
    'a chosen startup sound plays on iOS',
    (tester) async {
      expect(startUpSfxAsset('fantasy'), 'assets/startup_0.ogg');
      expect(startUpSfxAsset('tech'), 'assets/startup_1.ogg');
      expect(startUpSfxAsset('mute'), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}
