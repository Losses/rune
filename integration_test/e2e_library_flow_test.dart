import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/screens/settings_library/settings_library.dart';
import 'package:rune/screens/settings_home/settings_home.dart';
import 'package:rune/screens/tracks/tracks.dart';
import 'package:rune/utils/l10n.dart';
import 'package:rune/l10n/app_localizations.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final phase = int.parse(Platform.environment['E2E_PHASE']!);
  final folderA = Platform.environment['E2E_LIBRARY_A']!;
  final folderB = Platform.environment['E2E_LIBRARY_B']!;

  testWidgets('real app library lifecycle phase $phase', (tester) async {
    app.main(const []);
    await waitFor(
      tester,
      () => find.byType(FluentApp).evaluate().isNotEmpty,
      description: 'the real app to finish startup',
    );
    await pumpFor(tester, const Duration(seconds: 2));

    switch (phase) {
      case 1:
        await expectVisible(
          tester,
          find.byType(WelcomePage),
          description: 'Welcome for an empty library registry',
        );
        await addLibrary(tester, folderA, expectLibraryHome: true);
        await openTracks(tester);
        await expectVisible(tester, find.textContaining('Startup A'));

        await openLibrarySettings(tester);
        await addLibrary(tester, folderB);
        await openLibrarySettings(tester);
        await expectVisible(tester, find.text('Music B'));
        await selectLibrary(tester, 'Music A', 'switch');
        await openTracks(tester);
        await expectVisible(tester, find.textContaining('Startup A'));

        await openLibrarySettings(tester);
        await expectVisible(tester, find.text('Music A'));
        await expectVisible(tester, find.text('Music B'));
      case 2:
        await expectStartupInLibrary(tester, folderA);
        await openTracks(tester);
        await expectVisible(tester, find.textContaining('Startup A'));
        await openLibrarySettings(tester);
        await selectLibrary(tester, 'Music B', 'remove');
        await expectAbsent(tester, find.text('Music B'));
        expect(Directory(folderB).existsSync(), isTrue);
        expect(File('$folderB/startup_1.ogg').existsSync(), isTrue);
      case 3:
        await expectStartupInLibrary(tester, folderA);
        await openTracks(tester);
        await expectVisible(tester, find.textContaining('Startup A'));
        await openLibrarySettings(tester);
        await selectLibrary(tester, 'Music A', 'remove');
        await expectVisible(
          tester,
          find.byType(WelcomePage),
          description: 'Welcome after removing the open library',
        );
      case 4:
        await expectVisible(tester, find.byType(WelcomePage));
        final context = tester.element(find.byType(WelcomePage));
        expect(context.read<LibraryRegistryProvider>().records, isEmpty);
      default:
        fail('Unknown E2E_PHASE=$phase');
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}

Future<void> addLibrary(
  WidgetTester tester,
  String path, {
  bool expectLibraryHome = false,
}) async {
  final page = find.byType(WelcomePage).evaluate().isNotEmpty
      ? find.byType(WelcomePage)
      : find.byType(SettingsLibraryPage);
  final context = tester.element(page);
  final manager = context.read<LibraryManagerProvider>();
  final openState = context.read<LibraryOpenStateProvider>();
  context.read<LocalFolderPickerProvider>().pickDirectory = () async => path;

  final addLibraryLabel = localized(tester).addLibrary;
  await expectVisible(tester, find.text(addLibraryLabel));
  await tester.tap(find.text(addLibraryLabel).last);
  await tester.pumpAndSettle();
  await expectVisible(tester, find.text(localized(tester).localFolder));
  await tester.tap(find.text(localized(tester).localFolder));
  await tester.pumpAndSettle();
  await expectVisible(tester, find.text(localized(tester).confirm));
  await tester.tap(find.text(localized(tester).confirm));
  await tester.pumpAndSettle();

  final modeLabel = S.of(context).portableMode;
  final modeOption = find.text(modeLabel);
  await waitFor(
    tester,
    () => modeOption.evaluate().isNotEmpty,
    description: 'the Library Setup step in the guide',
  );
  // The Library Setup step completes on a single tap of the chosen mode.
  await tester.tap(modeOption);
  await tester.pump();
  final name = path.split(Platform.pathSeparator).last;
  await waitFor(
    tester,
    () => openState.openRecord?.alias == name,
    description: 'the app state to observe the newly added library',
  );
  final record = openState.openRecord!;
  expect(record.alias, name);
  await waitFor(
    tester,
    () =>
        manager.getScanTaskProgress(record.recordId)?.status ==
        TaskStatus.finished,
    description: 'the initial scan to finish',
  );
  expect(
    manager.getScanTaskProgress(record.recordId)?.status,
    TaskStatus.finished,
    reason: 'the real app failed to scan $name',
  );
  if (expectLibraryHome) {
    await waitFor(
      tester,
      () => $router.path == '/library',
      description: 'the initial scan to navigate to /library',
    );
    await expectVisible(
      tester,
      find.byType(LibraryHomePage),
      description: 'the library home page after initial scan',
    );
  }
}

Future<void> openTracks(WidgetTester tester) async {
  if ($router.path == '/tracks') return;
  final tracksLink = find.text(localized(tester).tracks);
  await expectVisible(
    tester,
    tracksLink,
    description: 'Tracks tile on the populated library home page',
  );
  await tester.ensureVisible(tracksLink.last);
  await tester.tap(tracksLink.last);
  await waitFor(
    tester,
    () => $router.path == '/tracks',
    description: 'navigation to the tracks page',
  );
  await pumpFor(tester, const Duration(seconds: 8));
}

Future<void> expectStartupInLibrary(WidgetTester tester, String path) async {
  await expectVisible(
    tester,
    find.byType(LibraryHomePage),
    description: 'the library page to finish startup restoration',
  );
  expect(
    $router.path,
    '/library',
    reason: 'app did not start directly in the last open library',
  );
  expect(
    find.byType(HomePage),
    findsNothing,
    reason: 'app started at Home instead of the last open library',
  );
  final context = tester.element(find.byType(LibraryHomePage));
  final registry = context.read<LibraryRegistryProvider>();
  final record = registry.openRecord;
  expect(record, isNotNull, reason: 'startup left the library closed');
  switch (record!.kind) {
    case LibraryRecordKindSummaryLocal(path: final recordPath):
      expect(recordPath, path, reason: 'startup opened a different library');
    default:
      fail('startup did not open the expected local library');
  }
}

Future<void> openLibrarySettings(WidgetTester tester) async {
  if (find.byType(SettingsLibraryPage).evaluate().isNotEmpty) return;
  if ($router.path != '/settings') {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyS);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await waitFor(
      tester,
      () => $router.path == '/settings',
      description: 'navigation to Settings with its registered shortcut',
    );
    await pumpFor(tester, const Duration(seconds: 8));
  }
  final libraryLabel = localized(tester).library;
  await expectVisible(tester, find.text(libraryLabel));
  await tester.tap(find.text(libraryLabel).last);
  await waitFor(
    tester,
    () => $router.path == '/settings/library',
    description: 'navigation to Settings > Library',
  );
}

AppLocalizations localized(WidgetTester tester) {
  for (final type in [
    SettingsLibraryPage,
    SettingsHomePage,
    LibraryHomePage,
    TracksPage,
    WelcomePage,
  ]) {
    final page = find.byType(type);
    if (page.evaluate().isNotEmpty) return S.of(tester.element(page));
  }
  fail('No localized app page is mounted at route ${$router.path}');
}

Future<void> selectLibrary(
  WidgetTester tester,
  String name,
  String action,
) async {
  final actionLabel = switch (action) {
    'switch' => localized(tester).switchTo,
    'remove' => localized(tester).removeLibrary,
    _ => action,
  };
  await expectVisible(tester, find.text(name));
  await tester.tap(find.text(name).last);
  await expectVisible(tester, find.text(actionLabel));
  await tester.tap(find.text(actionLabel).last);
  if (action == 'switch') {
    await waitFor(
      tester,
      () => $router.path == '/library',
      description: 'switching the open library to the selected record',
    );
  }
}

Future<void> expectVisible(
  WidgetTester tester,
  Finder finder, {
  String description = 'the required widget to appear',
}) async {
  await waitFor(
    tester,
    () => finder.evaluate().isNotEmpty,
    description: description,
  );
  expect(finder, findsWidgets);
}

Future<void> expectAbsent(WidgetTester tester, Finder finder) async {
  await waitFor(
    tester,
    () => finder.evaluate().isEmpty,
    description: 'the removed widget to disappear',
  );
  expect(finder, findsNothing);
}

Future<void> waitFor(
  WidgetTester tester,
  bool Function() condition, {
  required String description,
}) async {
  for (var attempt = 0; attempt < 200; attempt++) {
    if (condition()) return;
    await tester.pump(const Duration(milliseconds: 250));
  }
  fail('Timed out waiting for $description');
}

Future<void> pumpFor(WidgetTester tester, Duration duration) async {
  var elapsed = Duration.zero;
  while (elapsed < duration) {
    final step = duration - elapsed < const Duration(milliseconds: 100)
        ? duration - elapsed
        : const Duration(milliseconds: 100);
    await tester.pump(step);
    elapsed += step;
  }
}
