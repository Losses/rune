import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/query_tracks/query_tracks.dart';
import '../../screens/query_tracks/query_tracks_list.dart';
import '../../utils/query_list.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/constants/navigation_bar_height.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

final _queries = QueryList([('lib::artist_section', '7:x')]);

void main() {
  testWidgets('the tracks page of a section asks for its query and starts '
      'its list below the navigation bar, with no title of its own', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1200, 800);
    addTearDown(tester.view.reset);
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    addTearDown(sizes.dispose);
    final asked = <QueryList>[];
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
          ChangeNotifierProvider<ResponsiveProvider>(
            create: (_) => ResponsiveProvider(sizes, () async => null),
          ),
        ],
        child: FluentApp(
          builder: (context, child) => RevealEffectContext(child: child!),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: QueryTracksPage(
            queries: _queries,
            title: null,
            mode: null,
            loadMixPage: (queries, cursor, size) async {
              asked.add(queries);
              if (cursor > 0) return [];
              return [
                InternalMediaFile(
                  id: 1,
                  path: '',
                  artist: '',
                  album: '',
                  title: 'A song',
                  duration: 0,
                  coverArtPath: '',
                  trackNumber: null,
                ),
              ];
            },
            contentChanged: const Stream.empty(),
          ),
        ),
      ),
    );
    await tester.pump(const Duration(seconds: 1));
    await tester.pump(const Duration(seconds: 1));

    expect(asked, [_queries]);
    expect(find.text('A song'), findsOneWidget);
    // Every text is in the list.
    final list = find.byType(QueryTrackListView);
    expect(
      find.byType(Text).evaluate().length,
      find.descendant(of: list, matching: find.byType(Text)).evaluate().length,
    );
    expect(
      tester.getTopLeft(find.byType(QueryTrackListView)).dy,
      fullNavigationBarHeight,
    );
  });
}
