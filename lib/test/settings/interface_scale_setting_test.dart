import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_theme/widgets/interface_scale_setting.dart';
import 'package:rune/widgets/settings/settings_box_combo_box.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  _Responsive(this.compact);
  final bool compact;
  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      compact;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Screen extends ChangeNotifier implements ScreenSizeProvider {
  @override
  Size get screenSize => const Size(800, 600);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Widget _app(AppScaleProvider scale, {bool compact = false}) => MultiProvider(
  providers: [
    ChangeNotifierProvider.value(value: scale),
    ChangeNotifierProvider<ResponsiveProvider>(
      create: (_) => _Responsive(compact),
    ),
    ChangeNotifierProvider<ScreenSizeProvider>(create: (_) => _Screen()),
  ],
  child: const FluentApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: ScaffoldPage(content: InterfaceScaleSetting()),
  ),
);

void main() {
  testWidgets('choosing a scale persists it and updates the control', (
    tester,
  ) async {
    int? saved;
    final scale = AppScaleProvider(
      read: () async => 100,
      write: (value) async {
        saved = value;
      },
    );
    await scale.initialize();
    await tester.pumpWidget(_app(scale));
    final setting = tester.widget<SettingsBoxComboBox<int>>(
      find.byType(SettingsBoxComboBox<int>),
    );
    expect(
      setting.items.map((item) => item.value),
      AppScaleProvider.supportedPercentages,
    );
    setting.onChanged!(125);
    await tester.pumpAndSettle();
    expect(saved, 125);
    expect(
      tester
          .widget<SettingsBoxComboBox<int>>(
            find.byType(SettingsBoxComboBox<int>),
          )
          .value,
      125,
    );
    await tester.pumpWidget(const SizedBox());
    scale.dispose();
  });

  testWidgets('compact setting exposes scale choices', (tester) async {
    final scale = AppScaleProvider(read: () async => 100, write: (_) async {});
    await tester.pumpWidget(_app(scale, compact: true));
    await tester.tap(find.text('Interface scale'));
    await tester.pumpAndSettle();
    expect(find.text('200%'), findsOneWidget);
    await tester.tap(find.text('200%'));
    await tester.pumpAndSettle();
    expect(scale.percentage, 200);
    await tester.pumpWidget(const SizedBox());
    scale.dispose();
  });
  testWidgets('save failure is reported without an unhandled future', (
    tester,
  ) async {
    final scale = AppScaleProvider(
      read: () async => 100,
      write: (_) async => throw StateError('storage failed'),
    );
    await tester.pumpWidget(_app(scale));
    final setting = tester.widget<SettingsBoxComboBox<int>>(
      find.byType(SettingsBoxComboBox<int>),
    );
    setting.onChanged!(125);
    await tester.pumpAndSettle();
    expect(find.text('Could not save interface scale'), findsOneWidget);
    expect(find.textContaining('storage failed'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    scale.dispose();
  });
}
