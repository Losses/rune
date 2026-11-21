import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../../providers/library_home.dart';
import '../../screens/library_home/large_screen_library_home_list.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

void main() {
  testWidgets(
    'the library home says nothing about its data before its entries are read',
    (tester) async {
      final stored = Completer<String?>();
      final home = LibraryHomeProvider(readStored: () => stored.future);
      addTearDown(home.dispose);
      final layout = StartScreenLayoutManager();
      addTearDown(layout.dispose);

      await tester.pumpWidget(
        ChangeNotifierProvider<LibraryHomeProvider>.value(
          value: home,
          child: FluentApp(
            locale: const Locale('en'),
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: ScaffoldPage(
              content: LargeScreenLibraryHomeListView(layoutManager: layout),
            ),
          ),
        ),
      );
      await tester.pump();

      expect(home.loaded, isFalse);
      expect(find.text('No data available'), findsNothing);
    },
  );

  test('the entries count as read once reading them ends', () async {
    final home = LibraryHomeProvider(readStored: () async => null);
    await pumpEventQueue();
    expect(home.loaded, isTrue);
    expect(home.entries, isNotEmpty);
    home.dispose();
  });
}
