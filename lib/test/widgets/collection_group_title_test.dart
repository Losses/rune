import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../screens/collection/utils/group_title.dart';
import '../../utils/api/fetch_collection_group_summary_title.dart';
import '../../widgets/start_screen/utils/group.dart';

void main() {
  Future<Map<String, String>> titlesIn(
    WidgetTester tester,
    Locale locale,
  ) async {
    final titles = <String, String>{};
    await tester.pumpWidget(
      FluentApp(
        locale: locale,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) {
            String of(String title, {bool reserved = false}) =>
                collectionGroupTitle(
                  context,
                  Group<void>(
                    groupTitle: title,
                    items: const [],
                    reserved: reserved,
                  ),
                );
            titles
              ..['untitled'] = of(untitledGroupKey, reserved: true)
              ..['recent'] = of(recentlyPlayedGroupKey, reserved: true)
              // A group the library fills under a title of its service.
              ..['service'] = of('Made for You', reserved: true)
              // A group of the user's own may have no title, or any title,
              // also one that spells a key.
              ..['empty'] = of('')
              ..['own recent'] = of(recentlyPlayedGroupKey)
              ..['own untitled'] = of(untitledGroupKey)
              ..['english recent'] = of('Recently Played');
            return const SizedBox();
          },
        ),
      ),
    );
    return titles;
  }

  testWidgets(
    'a group a library fills under a key is named in the app language',
    (tester) async {
      expect(await titlesIn(tester, const Locale('en')), {
        'untitled': 'For You',
        'recent': 'Recently Played',
        'service': 'Made for You',
        'empty': '',
        'own recent': recentlyPlayedGroupKey,
        'own untitled': untitledGroupKey,
        'english recent': 'Recently Played',
      });
      final german = await titlesIn(tester, const Locale('de'));
      expect(german['untitled'], 'Für Sie');
      expect(german['recent'], 'Zuletzt gespielt');
      // What no key names is shown as it is, in any language.
      expect(german['service'], 'Made for You');
      expect(german['own recent'], recentlyPlayedGroupKey);
    },
  );

  test('pickers leave out the groups a library fills itself', () {
    CollectionGroupSummary group(String title, {bool reserved = false}) =>
        CollectionGroupSummary(groupTitle: title, count: 1, reserved: reserved);
    expect(
      pickableGroupTitles([
        group('Favorite'),
        group('Apple Music', reserved: true),
        group(recentlyPlayedGroupKey, reserved: true),
        group('Mood'),
      ]),
      ['Favorite', 'Mood'],
    );
  });
}
