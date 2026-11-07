import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_path.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/widgets/settings/settings_button.dart';

// A folder on the iPhone, as the Files app hands it to the app.
const _onDevice =
    '/private/var/mobile/Containers/Shared/AppGroup/'
    '2A36B718-E77C-43AD-9B4A-C15B1D7F74FC/File Provider Storage/music';

class _Libraries extends ChangeNotifier implements LibraryPathProvider {
  final _entries = [LibraryPathEntry(_onDevice)];

  @override
  Set<LibraryPathEntry> get libraryHistory => _entries.toSet();

  @override
  List<LibraryPathEntry> getLLPaths() => _entries;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Future<String?> _subtitle(WidgetTester tester) async {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: sizes),
        ChangeNotifierProvider.value(value: responsive),
        ChangeNotifierProvider<LibraryPathProvider>.value(value: _Libraries()),
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
  return tester
      .widget<SettingsButton>(find.byType(SettingsButton).last)
      .subtitle;
}

void main() {
  testWidgets('the home page shows an iOS library where the Files app does', (
    tester,
  ) async {
    expect(await _subtitle(tester), 'On My iPhone › music');
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('the home page shows a library\'s path elsewhere', (
    tester,
  ) async {
    expect(await _subtitle(tester), _onDevice);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));
}
