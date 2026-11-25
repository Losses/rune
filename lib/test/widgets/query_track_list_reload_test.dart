import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/query_tracks/query_tracks_list.dart';
import '../../utils/api/fetch_playlist_occurrences.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/query_list.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) => DeviceType.tv;
  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      false;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Screen extends ChangeNotifier implements ScreenSizeProvider {
  @override
  Size get screenSize => const Size(1200, 800);
  @override
  Size get layoutSize => const Size(1200, 800);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

List<InternalMediaFile> _tracks(String prefix, int from, int count) => [
  for (var i = from; i < from + count; i++)
    InternalMediaFile(
      id: i,
      path: '',
      artist: '',
      album: '',
      title: '$prefix-$i',
      duration: 0,
      coverArtPath: '',
      trackNumber: null,
    ),
];

const _unavailable = RequestFailure(
  'MixQueryRequest',
  RequestErrorKind.unavailable,
  'offline',
);

// Lets the loads and the frames they schedule run, and the animation timers
// they start end.
Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 5; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

void main() {
  late StreamController<void> contentChanged;
  final s = lookupAppLocalizations(const Locale('en'));

  setUp(() {
    contentChanged = StreamController<void>.broadcast(sync: true);
  });

  tearDown(() => contentChanged.close());

  Future<void> pumpList(
    WidgetTester tester,
    QueryList queries, {
    Future<List<InternalMediaFile>> Function(QueryList, int, int)? loadMixPage,
    Future<PlaylistOccurrencePage> Function(
      int,
      PlaylistOccurrenceCursor?,
      int,
    )?
    loadPlaylistPage,
  }) async {
    tester.view.physicalSize = const Size(1200, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final layout = StartScreenLayoutManager();
    addTearDown(layout.dispose);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<ResponsiveProvider>.value(
            value: _Responsive(),
          ),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: _Screen()),
          ChangeNotifierProvider<StartScreenLayoutManager>.value(value: layout),
        ],
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          builder: (context, child) => RevealEffectContext(child: child!),
          home: PageContentFrame(
            top: false,
            child: QueryTrackListView(
              layoutManager: layout,
              queries: queries,
              mode: null,
              contentChanged: contentChanged.stream,
              loadMixPage: loadMixPage ?? queryMixTracks,
              loadPlaylistPage: loadPlaylistPage ?? fetchPlaylistOccurrences,
            ),
          ),
        ),
      ),
    );
    await _settle(tester);
  }

  testWidgets('a failed refresh keeps the tracks shown and shows the error', (
    tester,
  ) async {
    Future<List<InternalMediaFile>> Function(int cursor) load =
        (cursor) async => _tracks('old', cursor, 5);
    await pumpList(
      tester,
      const QueryList([('lib::artist', '1')]),
      loadMixPage: (queries, cursor, size) => load(cursor),
    );
    expect(find.text('old-0'), findsOneWidget);
    expect(find.text('old-4'), findsOneWidget);

    for (final Object failure in [
      TimeoutException('no answer'),
      _unavailable,
    ]) {
      load = (cursor) async => throw failure;
      contentChanged.add(null);
      await _settle(tester);
      expect(find.text('old-0'), findsOneWidget, reason: '$failure');
      expect(find.text('old-4'), findsOneWidget, reason: '$failure');
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.byType(ContentDialog), findsOneWidget, reason: '$failure');
      expect(find.text(s.unknownError), findsOneWidget);
      expect(find.text(requestErrorText(s, failure)), findsOneWidget);

      // Another refresh failing while the error is open adds no second one.
      contentChanged.add(null);
      await _settle(tester);
      expect(find.byType(ContentDialog), findsOneWidget, reason: '$failure');

      await tester.tap(find.text(s.close));
      await _settle(tester);
      expect(find.byType(ContentDialog), findsNothing);
    }

    // A later change tries again.
    load = (cursor) async => _tracks('new', cursor, 5);
    contentChanged.add(null);
    await _settle(tester);
    expect(find.text('old-0'), findsNothing);
    expect(find.text('new-0'), findsOneWidget);
  });

  testWidgets(
    'only the list of the page shown reports a failed refresh, not the lists '
    'of the pages stacked under it',
    (tester) async {
      Future<List<InternalMediaFile>> Function(int cursor) below =
          (cursor) async => _tracks('below', cursor, 5);
      await pumpList(
        tester,
        const QueryList([('lib::artist', '1')]),
        loadMixPage: (queries, cursor, size) => below(cursor),
      );

      Future<List<InternalMediaFile>> Function(int cursor) above =
          (cursor) async => _tracks('above', cursor, 5);
      final layout = StartScreenLayoutManager();
      addTearDown(layout.dispose);
      final navigator = tester.state<NavigatorState>(
        find.byType(Navigator).first,
      );
      navigator.push(
        PageRouteBuilder<void>(
          pageBuilder: (context, _, _) => PageContentFrame(
            top: false,
            child: QueryTrackListView(
              layoutManager: layout,
              queries: const QueryList([('lib::artist', '2')]),
              mode: null,
              contentChanged: contentChanged.stream,
              loadMixPage: (queries, cursor, size) => above(cursor),
              loadPlaylistPage: fetchPlaylistOccurrences,
            ),
          ),
        ),
      );
      await _settle(tester);
      expect(find.text('above-0'), findsOneWidget);

      final timeout = TimeoutException('no answer');
      below = (cursor) async => throw _unavailable;
      above = (cursor) async => throw timeout;
      contentChanged.add(null);
      await _settle(tester);

      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text(requestErrorText(s, timeout)), findsOneWidget);
      expect(find.text(requestErrorText(s, _unavailable)), findsNothing);

      await tester.tap(find.text(s.close));
      await _settle(tester);
      navigator.pop();
      await _settle(tester);
      expect(find.text('below-0'), findsOneWidget);
      expect(find.byType(ContentDialog), findsNothing);
    },
  );

  testWidgets(
    'a failed refresh keeps the playlist cursors of the pages still to load',
    (tester) async {
      const page20 = PlaylistOccurrenceCursor(
        position: 20,
        occurrenceId: 'old-20',
      );
      final afters = <String?>[];
      var refreshFails = false;
      final heldPage20 = Completer<PlaylistOccurrencePage>();
      await pumpList(
        tester,
        const QueryList([('lib::playlist', '7')]),
        loadPlaylistPage: (playlistId, after, size) async {
          afters.add(after?.occurrenceId);
          if (after == null) {
            if (refreshFails) throw TimeoutException('no answer');
            return PlaylistOccurrencePage(_tracks('old', 0, 20), page20);
          }
          // The first request for the second page is still out when the
          // library changes.
          if (afters.where((x) => x == 'old-20').length == 1) {
            return heldPage20.future;
          }
          return PlaylistOccurrencePage(_tracks('old', 20, 5), null);
        },
      );
      expect(afters, [null, 'old-20']);
      expect(find.text('old-19'), findsOneWidget);

      refreshFails = true;
      contentChanged.add(null);
      await _settle(tester);

      // The refresh failed on its first page. The second page loads again
      // after the old list's own cursor.
      expect(afters, [null, 'old-20', null, 'old-20']);
      expect(find.text('old-0'), findsOneWidget);
      expect(find.text('old-24'), findsOneWidget);
      expect(find.byType(ContentDialog), findsOneWidget);
    },
  );

  testWidgets('a failed refresh with no tracks shown shows the failure', (
    tester,
  ) async {
    // The first load is still out when the library changes.
    Future<List<InternalMediaFile>> Function(int cursor) load = (cursor) =>
        Completer<List<InternalMediaFile>>().future;
    await pumpList(
      tester,
      const QueryList([('lib::artist', '1')]),
      loadMixPage: (queries, cursor, size) => load(cursor),
    );
    expect(find.byType(ProgressRing), findsOneWidget);

    load = (cursor) async => throw _unavailable;
    contentChanged.add(null);
    await _settle(tester);
    expect(find.text(s.requestUnavailable), findsOneWidget);
    expect(find.byType(ProgressRing), findsNothing);
    expect(find.byType(ContentDialog), findsNothing);
  });

  testWidgets('a burst of content changes makes at most two reload passes', (
    tester,
  ) async {
    var firstPageLoads = 0;
    Completer<List<InternalMediaFile>>? held;
    await pumpList(
      tester,
      const QueryList([('lib::artist', '1')]),
      loadMixPage: (queries, cursor, size) {
        if (cursor != 0) return Future.value(_tracks('t', cursor, 5));
        firstPageLoads++;
        return held?.future ?? Future.value(_tracks('t', 0, 5));
      },
    );
    expect(firstPageLoads, 1);

    held = Completer<List<InternalMediaFile>>();
    for (var i = 0; i < 5; i++) {
      contentChanged.add(null);
    }
    await _settle(tester);
    expect(firstPageLoads, 2);

    held.complete(_tracks('t', 0, 5));
    held = null;
    await _settle(tester);
    expect(firstPageLoads, 3);
  });
}
