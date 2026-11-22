import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/query_tracks/query_tracks_list.dart';
import 'package:rune/utils/api/fetch_playlist_occurrences.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/l10n.dart';
import 'package:rune/utils/query_list.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/track_list/utils/internal_media_file.dart';

InternalMediaFile _track(int i, {String title = 'Track'}) => InternalMediaFile(
  id: i,
  path: '/music/$i.flac',
  artist: 'Artist',
  album: 'Album',
  title: '$title $i',
  duration: 60,
  coverArtPath: '',
  trackNumber: i + 1,
);

List<InternalMediaFile> _page(int cursor) => [
  for (var i = cursor; i < cursor + 20; i++) _track(i),
];

Widget _list(
  QueryTrackPageLoader loader, {
  QueryList queries = const QueryList([('lib::album', '1')]),
  PlaylistPageLoader playlistLoader = fetchPlaylistOccurrences,
}) => MultiProvider(
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
            queries: queries,
            mode: null,
            loadMixPage: loader,
            loadPlaylistPage: playlistLoader,
          ),
        ),
      ),
    ),
  ),
);

Widget _playlistList(PlaylistPageLoader loader) => _list(
  (_, _, _) async => fail('a playlist loads no mix pages'),
  queries: const QueryList([('lib::playlist', '7')]),
  playlistLoader: loader,
);

const _secondPageCursor = PlaylistOccurrenceCursor(
  position: 19,
  occurrenceId: '19@0123456789abcdef',
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

  testWidgets(
    'a later playlist page that conflicts lists the playlist again from its first page',
    (tester) async {
      tester.view.physicalSize = const Size(2400, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      const conflict = RequestFailure(
        'FetchPlaylistOccurrencesRequest',
        RequestErrorKind.conflict,
        'The playlist changed since it was listed',
      );
      final afters = <String?>[];
      await tester.pumpWidget(
        _playlistList((_, after, _) async {
          afters.add(after?.occurrenceId);
          switch (afters.length) {
            case 1:
              return PlaylistOccurrencePage(_page(0), _secondPageCursor);
            case 2:
              throw conflict;
            case 3:
              return PlaylistOccurrencePage([
                for (var i = 0; i < 3; i++) _track(i, title: 'New'),
              ], null);
          }
          fail('unexpected page request');
        }),
      );
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(afters, [null, '19@0123456789abcdef', null]);
      expect(find.text('New 0'), findsOneWidget);
      expect(find.text('Track 0'), findsNothing);
      final s = S.of(tester.element(find.byType(QueryTrackListView)));
      expect(find.text(requestErrorText(s, conflict)), findsNothing);
    },
  );

  testWidgets(
    'a later playlist page failing for another reason is requested only once',
    (tester) async {
      tester.view.physicalSize = const Size(2400, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final afters = <String?>[];
      await tester.pumpWidget(
        _playlistList((_, after, _) async {
          afters.add(after?.occurrenceId);
          if (after == null) {
            return PlaylistOccurrencePage(_page(0), _secondPageCursor);
          }
          throw const RequestFailure(
            'FetchPlaylistOccurrencesRequest',
            RequestErrorKind.internal,
            'broken',
          );
        }),
      );
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(afters, [null, '19@0123456789abcdef']);
      expect(find.text('Track 0'), findsOneWidget);
    },
  );
}
