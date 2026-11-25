import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
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
  Stream<void>? contentChanged,
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
            contentChanged: contentChanged,
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

class _OpenLibrary extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'record',
    alias: 'Music',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'library', path: '/music'),
  );

  @override
  bool get hasCatalog => true;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

const _secondPageCursor = PlaylistOccurrenceCursor(
  position: 19,
  occurrenceId: '19@0123456789abcdef',
);

void main() {
  // The large and zune lists. A band or dock window shows an empty list as an
  // icon, and a belt window is too short to fit the Scan Library button.
  for (final (layout, size) in const [
    ('large', Size(2400, 600)),
    ('zune', Size(300, 700)),
  ]) {
    testWidgets(
      'a scan from the empty $layout list lists the tracks it found',
      (tester) async {
        tester.view.physicalSize = size;
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final states = StreamController<LibraryTaskStateChanged>.broadcast(
          sync: true,
        );
        final scans = <ScanAudioLibraryRequest>[];
        final manager = LibraryManagerProvider(
          sendScanRequest: scans.add,
          libraryTaskStates: states.stream,
        );
        addTearDown(states.close);
        addTearDown(manager.dispose);
        var indexed = const <InternalMediaFile>[];
        await tester.pumpWidget(
          MultiProvider(
            providers: [
              ChangeNotifierProvider<LibraryOpenStateProvider>(
                create: (_) => _OpenLibrary(),
              ),
              ChangeNotifierProvider<LibraryManagerProvider>.value(
                value: manager,
              ),
            ],
            child: _list(
              (_, cursor, _) async =>
                  cursor == 0 ? indexed : const <InternalMediaFile>[],
            ),
          ),
        );
        await tester.pump();
        await tester.pump();
        expect(find.text('No tracks found'), findsOneWidget);

        await tester.tap(find.text('Scan Library'));
        await tester.pump();
        expect(scans, hasLength(1));
        indexed = [_track(0)];
        states.add(
          const LibraryTaskStateChanged(
            state: LibraryTaskStateScanEnded(
              recordId: 'record',
              runId: 'run',
              progress: 1,
              total: 1,
              phase: ScanTaskPhase.indexFiles,
              outcome: ScanOutcomeFinished(),
            ),
          ),
        );
        for (var i = 0; i < 5; i++) {
          await tester.pump(const Duration(milliseconds: 100));
        }
        expect(find.text('Track 0'), findsOneWidget);
        expect(find.text('No tracks found'), findsNothing);
      },
    );
  }

  testWidgets('first page timeout shows the error text', (tester) async {
    await tester.pumpWidget(
      _list((_, _, _) async => throw TimeoutException('no answer')),
    );
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('TimeoutException'), findsOneWidget);
  });

  testWidgets('a later page timing out shows the dialog once', (tester) async {
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
    expect(find.textContaining('TimeoutException'), findsOneWidget);
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text('Unknown Error'), findsWidgets);
  });

  testWidgets('a later page failing again keeps one dialog', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(2400, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final changes = StreamController<void>.broadcast(sync: true);
    addTearDown(changes.close);
    final cursors = <int>[];
    await tester.pumpWidget(
      _list((_, cursor, _) async {
        cursors.add(cursor);
        if (cursor == 0) return _page(0);
        throw TimeoutException('no answer');
      }, contentChanged: changes.stream),
    );
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(find.byType(ContentDialog), findsOneWidget);
    final failures = cursors.where((c) => c == 20).length;

    // The library changed, so the failed page is requested and fails again.
    changes.add(null);
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(cursors.where((c) => c == 20).length, greaterThan(failures));
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text('Track 0'), findsOneWidget);
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
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text('Unknown Error'), findsWidgets);
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
