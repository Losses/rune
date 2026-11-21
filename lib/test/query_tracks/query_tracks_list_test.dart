import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/query_tracks/query_tracks_list.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/query_list.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/track_list/utils/internal_media_file.dart';

InternalMediaFile _track(int i) => InternalMediaFile(
  id: i,
  path: '/music/$i.flac',
  artist: 'Artist',
  album: 'Album',
  title: 'Track $i',
  duration: 60,
  coverArtPath: '',
  trackNumber: i + 1,
);

List<InternalMediaFile> _page(int cursor) => [
  for (var i = cursor; i < cursor + 20; i++) _track(i),
];

Widget _list(QueryTrackPageLoader loader) => MultiProvider(
  providers: [
    ChangeNotifierProvider(create: (_) => StartScreenLayoutManager()),
    ChangeNotifierProvider(
      create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
    ),
    ChangeNotifierProvider(
      create: (context) => ResponsiveProvider(
        context.read<ScreenSizeProvider>(),
        () async => null,
      ),
    ),
  ],
  child: FluentApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Builder(
      builder: (context) => RevealEffectContext(
        child: PageContentFrame(
          child: QueryTrackListView(
            layoutManager: context.read<StartScreenLayoutManager>(),
            queries: const QueryList([('lib::album', '1')]),
            mode: null,
            loadMixPage: loader,
          ),
        ),
      ),
    ),
  ),
);

void main() {
  testWidgets('first page timeout shows the error text', (tester) async {
    await tester.pumpWidget(
      _list((_, _, _) async => throw TimeoutException('no answer')),
    );
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('TimeoutException'), findsOneWidget);
  });

  testWidgets('a later page timing out keeps the loaded tracks', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(2400, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final cursors = <int>[];
    await tester.pumpWidget(
      _list((_, cursor, _) async {
        cursors.add(cursor);
        if (cursor == 0) return _page(0);
        throw TimeoutException('no answer');
      }),
    );
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(cursors, contains(20));
    expect(find.text('Track 0'), findsOneWidget);
    expect(find.textContaining('TimeoutException'), findsNothing);
  });

  testWidgets('a later page failing at once is requested only once', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(2400, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final cursors = <int>[];
    await tester.pumpWidget(
      _list((_, cursor, _) async {
        cursors.add(cursor);
        if (cursor == 0) return _page(0);
        throw const RequestFailure(
          'MixQueryRequest',
          RequestErrorKind.internal,
          'broken',
        );
      }),
    );
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(cursors.where((c) => c == 20), hasLength(1));
    expect(find.text('Track 0'), findsOneWidget);
  });
}
