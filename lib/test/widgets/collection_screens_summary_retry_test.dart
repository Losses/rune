import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/collection/utils/collection_data_provider.dart';
import 'package:rune/utils/query_list.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/band_screen/band_screen.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/widgets/start_screen/start_screen.dart';
import 'package:rune/widgets/start_screen/utils/group.dart';
import 'package:rune/widgets/start_screen/utils/internal_collection.dart';
import 'package:rune/widgets/turntile/turntile_screen.dart';

Future<List<Group<InternalCollection>>> failLater() async {
  await Future<void>.delayed(const Duration(milliseconds: 10));
  throw 'summary failed';
}

void main() {
  late StreamController<void> contentChanged;
  late int summaryCalls;
  late Completer<List<Group<InternalCollection>>> retriedSummary;

  setUp(() {
    CollectionCache().clearAll();
    contentChanged = StreamController<void>.broadcast(sync: true);
    summaryCalls = 0;
    retriedSummary = Completer();
  });

  tearDown(() => contentChanged.close());

  CollectionDataProvider makeProvider() => CollectionDataProvider(
    collectionType: CollectionType.album,
    contentChanged: contentChanged.stream,
    // The first summary succeeds, the second fails, and the third waits.
    loadSummary: (_) {
      summaryCalls++;
      if (summaryCalls == 1) return Future.value([]);
      if (summaryCalls == 2) return failLater();
      return retriedSummary.future;
    },
    loadPage: (type, size, cursor) async => (
      [
        Group<InternalCollection>(
          groupTitle: 'Group $cursor',
          items: [
            InternalCollection(
              id: cursor,
              name: 'Collection $cursor',
              queries: QueryList([('lib::album', '$cursor')]),
              collectionType: CollectionType.album,
              coverArtMap: const {},
              readonly: true,
            ),
          ],
        ),
      ],
      true,
    ),
  );

  Future<void> pumpScreen(
    WidgetTester tester,
    CollectionDataProvider data,
    Widget screen,
  ) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<CollectionDataProvider>.value(value: data),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
          ChangeNotifierProvider<ResponsiveProvider>(
            create: (_) => ResponsiveProvider(sizes, () async => null),
          ),
        ],
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: RevealEffectContext(
            child: ScaffoldPage(content: PageContentFrame(child: screen)),
          ),
        ),
      ),
    );
    await tester.pump(const Duration(seconds: 1));
  }

  final screens = <String, Widget>{
    'band': const BandScreen(),
    'start': const StartScreen(),
    'turntile': const TurntileScreen(),
  };

  for (final entry in screens.entries) {
    testWidgets(
      'the ${entry.key} screen keeps its items while the summary is retried '
      'after a failure',
      (tester) async {
        final data = makeProvider();
        addTearDown(data.dispose);
        await pumpScreen(tester, data, entry.value);
        await data.fetchData();
        await tester.pump(const Duration(seconds: 1));
        expect(data.items, isNotEmpty);

        // The reload fails at its summary and leaves the items on screen.
        contentChanged.add(null);
        // The frame that shows the new summary comes before it fails.
        await tester.pump();
        await tester.pump(const Duration(seconds: 1));
        expect(data.items, isNotEmpty);
        expect(find.byType(ProgressRing), findsNothing);

        // The next reload waits for its summary while the items stay.
        contentChanged.add(null);
        await tester.pump();
        expect(data.items, isNotEmpty);
        expect(find.byType(ProgressRing), findsNothing);
      },
    );
  }
}
