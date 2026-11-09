import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/widgets/settings/settings_button.dart';

// A folder on the iPhone, as the Files app hands it to the app.
const _onDevice =
    '/private/var/mobile/Containers/Shared/AppGroup/'
    '2A36B718-E77C-43AD-9B4A-C15B1D7F74FC/File Provider Storage/music';

class _Libraries extends ChangeNotifier implements LibraryRegistryProvider {
  @override
  final records = [
    LibraryRecordSummary(
      recordId: 'record-1',
      alias: 'music',
      kind: const LibraryRecordKindSummaryLocal(
        libraryId: 'library-1',
        path: _onDevice,
      ),
    ),
  ];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Future<(String, String)> _texts(WidgetTester tester) async {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: sizes),
        ChangeNotifierProvider.value(value: responsive),
        ChangeNotifierProvider<LibraryRegistryProvider>.value(
          value: _Libraries(),
        ),
        ChangeNotifierProvider<LibraryManagerProvider>.value(
          value: _LibraryManager(),
        ),
      ],
      child: const FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: HomePage(),
      ),
    ),
  );
  await tester.pump(const Duration(seconds: 2));
  final button = tester.widget<SettingsButton>(
    find.byType(SettingsButton).last,
  );
  return (button.title, button.subtitle);
}

void main() {
  testWidgets('the home page shows an iOS library where the Files app does', (
    tester,
  ) async {
    expect(await _texts(tester), ('music', 'On My iPhone › music'));
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('the home page shows a library\'s path elsewhere', (
    tester,
  ) async {
    expect(await _texts(tester), ('music', _onDevice));
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));
}
