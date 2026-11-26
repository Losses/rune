import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../main.dart' show cafeMode;
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../screens/artist/artist_detail.dart';
import '../../screens/artist/artist_page.dart';
import '../../screens/query_tracks/query_tracks.dart';
import '../../screens/query_tracks/query_tracks_list.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/query_list.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/history.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/artist_page/release_spans.dart';
import '../../widgets/artist_page/release_tile.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/navigation_bar/flip_text.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/no_items.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/managed_start_screen_item.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/start_group_implementation.dart';
import '../../widgets/tile/flip_tile.dart';
import '../../widgets/tile/rolling_cover_circles.dart';
import '../../widgets/tile_group/tile_grid.dart';
import '../../widgets/track_list/band_screen_track_list.dart';
import '../../widgets/track_list/large_screen_track_list.dart';
import '../../widgets/track_list/small_screen_track_list.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../widgets/turntile/turntile_group.dart';
import '../../widgets/turntile/turntile_normal_layout.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/start_screen/stacked_layout.dart';
import '../../widgets/track_list/large_screen_track_list_item.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

import '../widgets/capability_gate_test.dart' show snapshot;

const _albums = [MixQuery(operator: 'lib::album', parameter: '1')];
const _singles = [MixQuery(operator: 'lib::album', parameter: '2')];
const _tracks = [MixQuery(operator: 'lib::artist', parameter: '7')];
const _trackQueries = QueryList([('lib::artist', '7')]);

/// The paths of [count] covers.
List<String> _paths(int count) => [for (var i = 0; i < count; i += 1) '$i.jpg'];

/// A page of [sections], or else of two, the first with [firstCovers].
FetchArtistPageResponse _page({
  List<ArtistSection>? sections,
  List<MixQuery> tracks = _tracks,
  List<String> firstCovers = const [],
}) => FetchArtistPageResponse(
  artistId: 7,
  sections:
      sections ??
      [
        ArtistSection(
          kind: ArtistSectionKind.albums,
          title: '',
          opens: ArtistSectionOpens.songs,
          queries: _albums,
          covers: firstCovers,
          count: 3,
        ),
        const ArtistSection(
          kind: ArtistSectionKind.singles,
          title: '',
          opens: ArtistSectionOpens.songs,
          queries: _singles,
          covers: [],
          count: 2,
        ),
      ],
  tracks: tracks,
);

/// A window of [size] showing [child] with the providers a page needs.
Future<void> _pump(
  WidgetTester tester,
  Widget child, {
  Size size = const Size(1280, 720),
  RequestCapabilitiesProvider? capabilities,

  /// The test font draws every glyph a full square, so a title is wider than
  /// the tiles under it. A small title lets the tiles decide the width.
  bool smallTitles = false,

  /// Gets the pages pushed over [child], which is then under the app's
  /// navigator.
  List<RouteSettings>? pushed,

  /// The language the user picked. Without one the app follows the system.
  Locale? locale,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  addTearDown(sizes.dispose);
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
        if (capabilities != null)
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: capabilities,
          ),
      ],
      child: FluentApp(
        theme: smallTitles
            ? FluentThemeData(
                typography:
                    Typography.fromBrightness(
                      brightness: Brightness.light,
                    ).merge(
                      const Typography.raw(titleLarge: TextStyle(fontSize: 2)),
                    ),
              )
            : null,
        builder: (context, child) => RevealEffectContext(child: child!),
        locale: locale,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        navigatorKey: pushed == null
            ? null
            : runeWithNavigationBarAndPlaybackControllorNavigatorKey,
        onGenerateRoute: pushed == null
            ? null
            : (settings) {
                pushed.add(settings);
                return PageRouteBuilder<void>(
                  settings: settings,
                  transitionDuration: Duration.zero,
                  pageBuilder: (context, _, _) => const SizedBox(),
                );
              },
        home: child,
      ),
    ),
  );
  // The size provider saves the size after a second.
  await tester.pump(const Duration(seconds: 1));
}

RequestCapabilitiesProvider _capabilities({required bool artistPage}) {
  final provider = RequestCapabilitiesProvider(
    changes: const Stream.empty(),
    initial: snapshot().copyWith(
      fetchArtistPageRequest: artistPage
          ? const RequestCapabilityAvailable()
          : const RequestCapabilityHidden(),
    ),
  );
  addTearDown(provider.dispose);
  return provider;
}

ArtistPage _artistPage({
  Future<FetchArtistPageResponse> Function(int, String)? fetch,
  QueryTrackPageLoader? loadTracks,
  Stream<void>? contentChanged,
  math.Random? random,
}) => ArtistPage(
  artistId: 7,
  name: 'Hiroyuki Sawano',
  fetch: fetch ?? (_, _) async => _page(),
  loadTracks: loadTracks ?? queryMixTracks,
  contentChanged: contentChanged,
  random: random,
);

/// The release group of the large layout and of a zune window.
final _startGroup = find.byType(StartGroup<int>);
final _turntileGroup = find.byType(TurntileGroup<int>);

/// The grid a release group lays its tiles on.
TileGrid _grid(WidgetTester tester, Finder group) => tester.widget<TileGrid>(
  find.descendant(of: group, matching: find.byType(TileGrid)),
);

List<String> _covers(WidgetTester tester) =>
    tester.widget<ReleaseTile>(find.byType(ReleaseTile)).covers;

/// Removes the page and lets the image cache clear itself, which it does a
/// moment after the last tile with covers is gone.
Future<void> _removeCovers(WidgetTester tester) async {
  await tester.pumpWidget(const SizedBox());
  await tester.pump(const Duration(milliseconds: 200));
}

void main() {
  group('the gate', () {
    testWidgets('without the capability the page is the artist tracks', (
      tester,
    ) async {
      await _pump(
        tester,
        Builder(
          builder: (context) =>
              artistDetailPage(context, QueryTracksParameter(7, 'Sawano')),
        ),
        capabilities: _capabilities(artistPage: false),
      );
      expect(find.byType(QueryTracksPage), findsOneWidget);
      expect(find.byType(ArtistPage), findsNothing);
      final page = tester.widget<QueryTracksPage>(find.byType(QueryTracksPage));
      expect(page.queries, QueryList([('lib::artist', '7')]));
      expect(page.title, 'Sawano');
    });

    testWidgets('with the capability the page is the artist page', (
      tester,
    ) async {
      await _pump(
        tester,
        Builder(
          builder: (context) =>
              artistDetailPage(context, QueryTracksParameter(7, 'Sawano')),
        ),
        capabilities: _capabilities(artistPage: true),
      );
      expect(find.byType(ArtistPage), findsOneWidget);
      expect(find.byType(QueryTracksPage), findsNothing);
      final page = tester.widget<ArtistPage>(find.byType(ArtistPage));
      expect(page.artistId, 7);
      expect(page.name, 'Sawano');
    });
  });

  group('the page', () {
    testWidgets('shows the sections as tiles and hands Tracks to the list', (
      tester,
    ) async {
      await _pump(tester, _artistPage());
      await tester.pump();

      expect(find.byType(ReleaseTile), findsNWidgets(2));
      expect(find.text('Albums'), findsOneWidget);
      expect(find.text('Singles & EPs'), findsOneWidget);
      expect(find.text('Releases'), findsOneWidget);
      expect(find.text('Tracks'), findsOneWidget);
      expect(find.text('Hiroyuki Sawano'), findsOneWidget);
      // Both headings are drawn by the widget the library home draws its
      // group titles with, and no other text of the page is a heading.
      final headings = tester
          .widgetList<StartGroupStackedLayout>(
            find.byType(StartGroupStackedLayout),
          )
          .toList();
      expect(headings.map((x) => x.groupTitle), ['Releases', 'Tracks']);
      expect(headings.map((x) => x.onTitleTap), everyElement(isNull));
      final titleLarge = FluentTheme.of(
        tester.element(find.text('Releases')),
      ).typography.titleLarge;
      for (final heading in ['Releases', 'Tracks']) {
        expect(tester.widget<Text>(find.text(heading)).style, titleLarge);
      }
      final list = tester.widget<QueryTrackListView>(
        find.byType(QueryTrackListView),
      );
      expect(list.queries, QueryList([('lib::artist', '7')]));
    });

    testWidgets(
      'a release tile has the shell and the menu of a collection tile',
      (tester) async {
        await _pump(tester, _artistPage());
        await tester.pump();

        expect(
          find.descendant(
            of: find.byType(ReleaseTile),
            matching: find.byType(ContextMenuWrapper),
          ),
          findsNWidgets(2),
        );
        final gesture = await tester.startGesture(
          tester.getCenter(find.byType(ReleaseTile).first),
          kind: PointerDeviceKind.mouse,
          buttons: kSecondaryMouseButton,
        );
        await gesture.up();
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));

        expect(find.text('Start Playing'), findsOneWidget);
        expect(find.text('Add to Queue'), findsOneWidget);
      },
    );

    testWidgets('waits behind the loading ring', (tester) async {
      final answer = Completer<FetchArtistPageResponse>();
      await _pump(tester, _artistPage(fetch: (_, _) => answer.future));
      await tester.pump();
      expect(find.byType(ProgressRing), findsOneWidget);
      expect(_startGroup, findsNothing);

      answer.complete(_page());
      await tester.pump();
      expect(_startGroup, findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('leaves out empty tracks', (tester) async {
      await _pump(
        tester,
        _artistPage(fetch: (_, _) async => _page(tracks: [])),
      );
      await tester.pump();
      expect(_startGroup, findsOneWidget);
      expect(find.byType(QueryTrackListView), findsNothing);
      expect(find.text('Tracks'), findsNothing);
    });

    testWidgets('leaves out empty sections', (tester) async {
      await _pump(
        tester,
        _artistPage(fetch: (_, _) async => _page(sections: [])),
      );
      await tester.pump();
      expect(_startGroup, findsNothing);
      expect(find.text('Releases'), findsNothing);
      expect(find.byType(QueryTrackListView), findsOneWidget);
    });

    testWidgets('a press on a section opens its page', (tester) async {
      cafeMode = false;
      $router.update('/library', null);
      $history.reset();
      final pushed = <RouteSettings>[];
      await _pump(tester, _artistPage(), pushed: pushed);
      await tester.pump();

      // A tile answers a press after a short delay.
      await tester.tap(find.text('Singles & EPs'));
      await tester.pump(const Duration(milliseconds: 200));
      expect(pushed.single.name, '/artists/section');
      final parameter =
          (pushed.single.arguments as RouterTransitionParameter).parameters
              as ArtistSectionParameter;
      expect(parameter.queries, QueryList([('lib::album', '2')]));
      expect(parameter.title, 'Singles & EPs');
      expect(parameter.artistName, 'Hiroyuki Sawano');
      expect(parameter.sections.map((x) => x.title), [
        'Albums',
        'Singles & EPs',
      ]);
      expect(parameter.index, 1);
    });

    testWidgets('more than nine sections are all drawn', (tester) async {
      await _pump(
        tester,
        _artistPage(
          fetch: (_, _) async => _page(
            sections: [
              for (var i = 0; i < 12; i++)
                ArtistSection(
                  kind: ArtistSectionKind.albums,
                  title: 'Section $i',
                  opens: ArtistSectionOpens.songs,
                  queries: _albums,
                  covers: const [],
                  count: 1,
                ),
            ],
          ),
        ),
        size: const Size(2200, 1200),
      );
      await tester.pump();
      expect(find.byType(ReleaseTile), findsNWidgets(12));
      // Every tile is a square, named as a collection tile of the home.
      for (final span in _grid(tester, _startGroup).spans) {
        expect(span.columns, span.rows);
      }
      expect(
        find.descendant(
          of: find.byType(ReleaseTile),
          matching: find.byType(NamedTileCover),
        ),
        findsNWidgets(12),
      );
      expect(_grid(tester, _startGroup).spans.skip(9).map((x) => x.columns), [
        1,
        1,
        1,
      ]);
    });

    group('the arrangements of the release tiles', () {
      // Each section has covers for every arrangement of a tile of one
      // cell, and for a large tile.
      FetchArtistPageResponse four({int covers = 20}) => _page(
        sections: [
          for (var i = 0; i < 4; i += 1)
            ArtistSection(
              kind: ArtistSectionKind.albums,
              title: '',
              opens: ArtistSectionOpens.songs,
              queries: _albums,
              covers: _paths(covers),
              count: 1,
            ),
        ],
      );
      List<ReleaseTile> tiles(WidgetTester tester) =>
          tester.widgetList<ReleaseTile>(find.byType(ReleaseTile)).toList();
      List<CircleArrangementKind> arrangements(WidgetTester tester) => [
        for (final tile in tiles(tester)) tile.arrangement,
      ];

      testWidgets('differ on every tile of one cell, and stay through '
          'rebuilds and window sizes', (tester) async {
        final key = GlobalKey();
        Future<void> pump(Size size) async {
          await _pump(
            tester,
            ArtistPage(
              key: key,
              artistId: 7,
              name: 'Hiroyuki Sawano',
              fetch: (_, _) async => four(),
              random: math.Random(1),
            ),
            size: size,
          );
          await tester.pump();
        }

        await pump(const Size(2200, 1200));
        final first = arrangements(tester);
        expect(first, hasLength(4));
        // The tiles of one cell differ; the large one may match one of them.
        expect(first.skip(1).toSet(), hasLength(3));
        for (final tile in tiles(tester)) {
          if (tile.cells == 1) {
            expect(tile.arrangement, isNot(CircleArrangementKind.bigAndSmall));
          }
        }
        expect(tiles(tester).first.cells, 2);

        // The same page, built again and in a window of another size.
        await pump(const Size(2200, 1200));
        expect(arrangements(tester), first);
        await pump(const Size(1280, 720));
        expect(arrangements(tester), first);
        // A zune window has the same tiles in a list.
        await pump(const Size(320, 700));
        expect(arrangements(tester).skip(1), first.skip(1));
        await pump(const Size(2200, 1200));
        expect(arrangements(tester), first);
        await _removeCovers(tester);
      });

      testWidgets('are those the covers of a section can fill, on tiles of '
          'one cell when the first section has few', (tester) async {
        for (var seed = 0; seed < 4; seed += 1) {
          await _pump(
            tester,
            _artistPage(
              fetch: (_, _) async => four(covers: 5),
              random: math.Random(seed),
            ),
            size: const Size(2200, 1200),
          );
          await tester.pump();
          for (final tile in tiles(tester)) {
            expect(tile.cells, 1);
            expect(
              CircleArrangement.of(tile.arrangement, 1).inSight,
              lessThanOrEqualTo(5),
            );
          }
          await _removeCovers(tester);
        }
      });

      testWidgets('are picked anew by another visit of the page', (
        tester,
      ) async {
        final picked = <String>{};
        for (var seed = 0; seed < 8; seed += 1) {
          await _pump(
            tester,
            _artistPage(
              fetch: (_, _) async => four(),
              random: math.Random(seed),
            ),
            size: const Size(2200, 1200),
          );
          await tester.pump();
          picked.add(arrangements(tester).join());
          await _removeCovers(tester);
        }
        expect(picked.length, greaterThan(1));
      });
    });

    testWidgets('shows the failure of the request as the track list does', (
      tester,
    ) async {
      const failure = RequestFailure(
        'FetchArtistPageRequest',
        RequestErrorKind.unavailable,
        'no answer',
      );
      await _pump(tester, _artistPage(fetch: (_, _) => Future.error(failure)));
      await tester.pump();
      expect(find.byType(ProgressRing), findsNothing);
      expect(_startGroup, findsNothing);
      expect(
        find.text(
          requestErrorText(lookupAppLocalizations(const Locale('en')), failure),
        ),
        findsOneWidget,
      );
    });

    testWidgets('an answer with nothing says so as the home does, and in a '
        'list window as an empty track list does', (tester) async {
      Future<FetchArtistPageResponse> nothing(int _, String _) async =>
          _page(sections: [], tracks: []);
      await _pump(tester, _artistPage(fetch: nothing));
      await tester.pump();
      expect(
        find.text(lookupAppLocalizations(const Locale('en')).noDataAvailable),
        findsOneWidget,
      );
      expect(find.byType(NoItems), findsNothing);

      await _pump(
        tester,
        _artistPage(fetch: nothing),
        size: const Size(640, 130),
      );
      await tester.pump();
      expect(find.byType(BandScreenTrackList), findsOneWidget);
      expect(find.byType(NoItems), findsOneWidget);
    });
  });

  group('the entrance', () {
    Future<void> answer(WidgetTester tester, Size size) async {
      final answer = Completer<FetchArtistPageResponse>();
      await _pump(
        tester,
        _artistPage(fetch: (_, _) => answer.future),
        size: size,
      );
      // No tracks, so nothing but the page itself starts the entrance.
      answer.complete(_page(tracks: []));
      await tester.pump();
    }

    testWidgets('release tiles enter as the tiles of the home do', (
      tester,
    ) async {
      await answer(tester, const Size(1280, 720));
      expect(find.byType(ReleaseTile), findsNothing);
      for (var i = 0; i < 10; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(
        find.ancestor(
          of: find.byType(ReleaseTile),
          matching: find.byType(ManagedStartScreenItem),
        ),
        findsNWidgets(2),
      );
    });

    for (final (name, size) in [
      ('zune', const Size(320, 700)),
      ('belt', const Size(900, 200)),
      ('band', const Size(640, 130)),
      ('dock', const Size(120, 700)),
    ]) {
      testWidgets('release tiles of a $name window enter as its tiles do', (
        tester,
      ) async {
        await answer(tester, size);
        final items = find.ancestor(
          of: find.byType(ReleaseTile),
          matching: find.byType(ManagedTurntileScreenItem),
        );
        double opacity() => tester
            .widget<AnimatedOpacity>(
              find
                  .descendant(
                    of: items.first,
                    matching: find.byType(AnimatedOpacity),
                  )
                  .first,
            )
            .opacity;
        expect(items, findsNWidgets(2));
        expect(opacity(), 0);
        for (var i = 0; i < 10; i++) {
          await tester.pump(const Duration(milliseconds: 100));
        }
        expect(opacity(), 1);
      });
    }
  });

  group('an answer that is too late', () {
    testWidgets('starts no entrance when the page is gone', (tester) async {
      final answer = Completer<FetchArtistPageResponse>();
      await _pump(tester, _artistPage(fetch: (_, _) => answer.future));
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      answer.complete(_page(tracks: []));
      await tester.pump();
      // A pending timer would fail the test at its end.
    });

    testWidgets('of another artist does not play the entrance again', (
      tester,
    ) async {
      final answers = <int, Completer<FetchArtistPageResponse>>{
        7: Completer<FetchArtistPageResponse>(),
        8: Completer<FetchArtistPageResponse>(),
      };
      final artist = ValueNotifier(7);
      addTearDown(artist.dispose);
      await _pump(
        tester,
        ValueListenableBuilder<int>(
          valueListenable: artist,
          builder: (context, id, _) => ArtistPage(
            artistId: id,
            name: 'Artist $id',
            fetch: (id, _) => answers[id]!.future,
          ),
        ),
      );
      await tester.pump();
      artist.value = 8;
      await tester.pump();
      answers[8]!.complete(_page(tracks: []));
      await tester.pump();
      for (var i = 0; i < 10; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      answers[7]!.complete(_page(tracks: []));
      await tester.pump();
      // A pending timer would fail the test at its end.
    });
  });

  group('the quiet refresh', () {
    FetchArtistPageResponse withCovers(List<String> covers) => _page(
      sections: [
        ArtistSection(
          kind: ArtistSectionKind.albums,
          title: '',
          opens: ArtistSectionOpens.songs,
          queries: _albums,
          covers: covers,
          count: 3,
        ),
      ],
    );

    Future<StreamController<void>> pumpWithEvents(
      WidgetTester tester,
      Future<FetchArtistPageResponse> Function(int, String) fetch,
    ) async {
      final events = StreamController<void>.broadcast();
      addTearDown(events.close);
      await _pump(
        tester,
        _artistPage(fetch: fetch, contentChanged: events.stream),
      );
      await tester.pump();
      return events;
    }

    testWidgets(
      'an event asks once more, keeps the ring away and fills covers',
      (tester) async {
        final answers = [Completer<FetchArtistPageResponse>()];
        var calls = 0;
        final events = await pumpWithEvents(tester, (_, _) {
          calls++;
          return calls == 1
              ? Future.value(withCovers([]))
              : answers.last.future;
        });
        expect(calls, 1);
        expect(_covers(tester), isEmpty);
        // The track list may have a ring of its own. The page's must not return.
        final rings = find.byType(ProgressRing).evaluate().length;

        events.add(null);
        await tester.pump();
        expect(calls, 2);
        expect(find.byType(ProgressRing), findsNWidgets(rings));
        expect(_startGroup, findsOneWidget);

        answers.last.complete(withCovers(['a.jpg', 'b.jpg']));
        await tester.pump();
        expect(calls, 2);
        expect(find.byType(ProgressRing), findsNWidgets(rings));
        expect(_covers(tester), ['a.jpg', 'b.jpg']);
        await _removeCovers(tester);
      },
    );

    testWidgets('a failed refresh keeps the page and is logged', (
      tester,
    ) async {
      final logged = <String>[];
      final previous = debugPrint;
      debugPrint = (message, {wrapWidth}) {
        if (message != null) logged.add(message);
      };

      var calls = 0;
      final events = await pumpWithEvents(tester, (_, _) {
        calls++;
        return calls == 1
            ? Future.value(withCovers(['a.jpg']))
            : Future.error(StateError('gone'));
      });

      events.add(null);
      await tester.pump();
      await tester.pump();
      debugPrint = previous;
      expect(calls, 2);
      expect(_covers(tester), ['a.jpg']);
      expect(find.textContaining('gone'), findsNothing);
      expect(logged.where((m) => m.contains('gone')), hasLength(1));
      await _removeCovers(tester);
    });

    testWidgets('events during a request make at most one more after it', (
      tester,
    ) async {
      final pending = <Completer<FetchArtistPageResponse>>[];
      var calls = 0;
      final events = await pumpWithEvents(tester, (_, _) {
        calls++;
        if (calls == 1) return Future.value(withCovers([]));
        final answer = Completer<FetchArtistPageResponse>();
        pending.add(answer);
        return answer.future;
      });

      events.add(null);
      await tester.pump();
      expect(calls, 2);
      events
        ..add(null)
        ..add(null)
        ..add(null);
      await tester.pump();
      expect(calls, 2);

      pending[0].complete(withCovers(['a.jpg']));
      await tester.pump();
      expect(calls, 3);
      expect(_covers(tester), ['a.jpg']);

      pending[1].complete(withCovers(['a.jpg', 'b.jpg']));
      await tester.pump();
      expect(calls, 3);
      expect(_covers(tester), ['a.jpg', 'b.jpg']);
      await _removeCovers(tester);
    });

    testWidgets('stops listening when the page is removed', (tester) async {
      var calls = 0;
      final events = await pumpWithEvents(tester, (_, _) {
        calls++;
        return Future.value(withCovers([]));
      });
      expect(events.hasListener, isTrue);

      await tester.pumpWidget(const SizedBox());
      expect(events.hasListener, isFalse);
      events.add(null);
      await tester.pump();
      expect(calls, 1);
    });
  });

  group('the language', () {
    Future<List<String>> askedIn(WidgetTester tester, Locale? locale) async {
      final asked = <String>[];
      await _pump(
        tester,
        _artistPage(
          fetch: (_, language) async {
            asked.add(language);
            return _page();
          },
        ),
        locale: locale,
      );
      await tester.pump();
      return asked;
    }

    testWidgets('the page is asked in the language the app shows', (
      tester,
    ) async {
      const picked = [
        (Locale.fromSubtags(languageCode: 'zh', scriptCode: 'Hans'), 'zh-Hans'),
        (
          Locale.fromSubtags(
            languageCode: 'zh',
            scriptCode: 'Hant',
            countryCode: 'TW',
          ),
          'zh-Hant-TW',
        ),
        (Locale('ja'), 'ja'),
        (Locale('en'), 'en'),
      ];
      for (final (locale, tag) in picked) {
        expect(await askedIn(tester, locale), [tag], reason: '$locale');
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets('without a picked language it is the one the system gives', (
      tester,
    ) async {
      tester.platformDispatcher.localesTestValue = const [Locale('de', 'DE')];
      addTearDown(tester.platformDispatcher.clearLocalesTestValue);
      expect(await askedIn(tester, null), ['de']);
    });

    testWidgets('a new language asks again and keeps the page', (tester) async {
      final asked = <String>[];
      final page = _artistPage(
        fetch: (_, language) async {
          asked.add(language);
          return _page();
        },
      );
      await _pump(tester, page, locale: const Locale('en'));
      await tester.pump();
      expect(asked, ['en']);
      final rings = find.byType(ProgressRing).evaluate().length;

      await _pump(tester, page, locale: const Locale('ja'));
      expect(asked, ['en', 'ja']);
      expect(find.byType(ProgressRing), findsNWidgets(rings));
      expect(_startGroup, findsOneWidget);
    });
  });

  group('the window size classes', () {
    // Each class picks as the collection pages and the track list do.
    testWidgets('a zune window is one vertical list, releases first', (
      tester,
    ) async {
      await _pump(tester, _artistPage(), size: const Size(320, 700));
      await tester.pump();
      expect(find.byType(SmallScreenTrackList), findsOneWidget);
      final scrollables = find.ancestor(
        of: _turntileGroup,
        matching: find.byType(Scrollable),
      );
      expect(scrollables, findsOneWidget);
      expect(
        tester.state<ScrollableState>(scrollables).axisDirection,
        AxisDirection.down,
      );
      expect(
        find.descendant(
          of: scrollables,
          matching: find.byType(QueryTrackListView),
        ),
        findsNothing,
      );
      expect(
        tester.widget<TurntileGroup<int>>(_turntileGroup).gridLayoutVariation,
        TurntileGroupGridLayoutVariation.tile,
      );
    });

    for (final width in [200.0, 260.0, 320.0, 340.0]) {
      testWidgets('a $width wide zune window fills the columns of its tile '
          'group', (tester) async {
        // The first section has the covers of a large tile.
        await _pump(
          tester,
          _artistPage(fetch: (_, _) async => _page(firstCovers: _paths(6))),
          size: Size(width, 700),
        );
        await tester.pump();
        // The tiles are those set for the columns the group has: squares
        // that fill its rows, but for the end of the last, and reach its
        // right edge.
        final grid = _grid(tester, _turntileGroup);
        for (final span in grid.spans) {
          expect(span.columns, span.rows);
        }
        final cells = grid.spans.fold<int>(
          0,
          (sum, x) => sum + x.columns * x.rows,
        );
        expect(
          grid.layout.columns * grid.layout.rows - cells,
          lessThan(grid.layout.columns),
        );
        final right = tester
            .widgetList<ReleaseTile>(find.byType(ReleaseTile))
            .map((x) => tester.getRect(find.byWidget(x)).right)
            .reduce(math.max);
        expect(
          tester.getRect(_turntileGroup).right -
              TurnTileGroupNormalLayout.padding.right -
              right,
          lessThanOrEqualTo(grid.gapSize + 1),
        );
        await _removeCovers(tester);
      });
    }

    testWidgets('a zune window titles its groups as the turntile does', (
      tester,
    ) async {
      await _pump(tester, _artistPage(), size: const Size(320, 700));
      await tester.pump();
      expect(
        tester
            .widget<TurnTileGroupNormalLayout>(
              find.byType(TurnTileGroupNormalLayout),
            )
            .groupTitle,
        'Releases',
      );
      expect(find.byType(StartGroupStackedLayout), findsNothing);
      // The tracks are the list's own items under a title drawn as a group's.
      final releases = find.text('Releases');
      final tracks = find.text('Tracks');
      expect(
        tester.widget<Text>(tracks).style,
        tester.widget<Text>(releases).style,
      );
      expect(tester.getTopLeft(tracks).dx, tester.getTopLeft(releases).dx);
      // Under it comes what a group has between its title and its items.
      final header = find
          .ancestor(of: _turntileGroup, matching: find.byType(Column))
          .first;
      expect(
        tester.getBottomLeft(header).dy - tester.getBottomLeft(tracks).dy,
        TurnTileGroupNormalLayout.titlePadding.bottom,
      );
    });

    testWidgets('a zune window scrolls the releases away with the tracks', (
      tester,
    ) async {
      await _pump(tester, _artistPage(), size: const Size(320, 300));
      await tester.pump();
      final before = tester.getTopLeft(_turntileGroup).dy;
      await tester.drag(find.byType(Scrollable).first, const Offset(0, -80));
      await tester.pump();
      final after = _turntileGroup;
      // Scrolled off the screen, the group is not built any more.
      if (after.evaluate().isNotEmpty) {
        expect(tester.getTopLeft(after).dy, lessThan(before));
      }
    });

    for (final (name, size, horizontal) in [
      ('belt', const Size(900, 200), true),
      ('band', const Size(640, 130), true),
      ('dock', const Size(120, 700), false),
    ]) {
      testWidgets('a $name window draws the tiles of the band screen and the '
          'band tracks', (tester) async {
        await _pump(tester, _artistPage(), size: size);
        await tester.pump();
        expect(find.byType(BandScreenTrackList), findsOneWidget);
        expect(_startGroup, findsNothing);
        expect(find.byType(TurnTileGroupNormalLayout), findsNothing);
        expect(find.byType(StartGroupStackedLayout), findsNothing);
        expect(
          find.byType(BeltContainer),
          name == 'belt' ? findsOneWidget : findsNothing,
        );
        final list = find.descendant(
          of: find.byType(BandScreenTrackList),
          matching: find.byType(ListView),
        );
        final padding = getScrollContainerPadding(
          tester.element(list),
          listen: false,
        );
        final listSize = tester.getSize(list);
        final cross = horizontal
            ? listSize.height - padding.vertical
            : listSize.width - padding.horizontal;
        final tiles = find.byType(ReleaseTile);
        expect(tiles, findsNWidgets(2));
        // A square of the strip's cross size, less the padding the band
        // screen puts round the tile of an item.
        for (final tile in tiles.evaluate()) {
          expect(
            tester.getSize(find.byWidget(tile.widget)),
            Size(cross - 4, cross - 2),
          );
        }
        // Band and dock draw the covers alone, as the tiles of an album do.
        expect(
          find.text('Albums'),
          name == 'belt' ? findsOneWidget : findsNothing,
        );
        // The tracks follow the tiles along the strip.
        final first = tester.getTopLeft(tiles.first);
        final last = tester.getTopLeft(tiles.last);
        expect(horizontal ? last.dx > first.dx : last.dy > first.dy, isTrue);
      });
    }

    for (final (name, size, horizontal) in [
      ('zune', const Size(320, 700), false),
      ('belt', const Size(900, 200), true),
      ('band', const Size(640, 130), true),
      ('dock', const Size(120, 700), false),
    ]) {
      testWidgets('a $name window keeps the release tiles when the first '
          'tracks fail to load, and shows the failure after them', (
        tester,
      ) async {
        const failure = RequestFailure(
          'MixQueryRequest',
          RequestErrorKind.unavailable,
          'no answer',
        );
        final answer = Completer<List<InternalMediaFile>>();
        await _pump(
          tester,
          _artistPage(loadTracks: (_, _, _) => answer.future),
          size: size,
        );
        await tester.pump();
        final tiles = find.byType(ReleaseTile);
        expect(tiles, findsNWidgets(2));
        final built = tiles.evaluate().toList();
        final places = [
          for (var i = 0; i < 2; i += 1) tester.getRect(tiles.at(i)),
        ];

        answer.completeError(failure);
        await tester.pump();
        await tester.pump(const Duration(seconds: 1));
        // The tiles are the ones that were there, so they keep their state
        // and their entrance, where they were.
        expect(tiles.evaluate().toList(), built);
        expect([
          for (var i = 0; i < 2; i += 1) tester.getRect(tiles.at(i)),
        ], places);
        expect(
          find.text('Tracks'),
          name == 'zune' ? findsOneWidget : findsNothing,
        );
        // The failure is where the tracks would be, after the tiles along
        // the list.
        final text = find.text(
          requestErrorText(lookupAppLocalizations(const Locale('en')), failure),
        );
        expect(text, findsOneWidget);
        final last = tester.getRect(tiles.last);
        final shown = tester.getRect(text);
        expect(
          horizontal ? shown.left : shown.top,
          greaterThanOrEqualTo(horizontal ? last.right : last.bottom),
        );
      });
    }
  });

  group('the large layout', () {
    // Window sizes in the classes the home gives its large list.
    final windows = <(String, Size)>[
      ('desktop', const Size(1280, 720)),
      ('tv', const Size(2200, 1200)),
      ('tablet', const Size(768, 1024)),
      ('mobile', const Size(560, 900)),
      ('phone', const Size(390, 844)),
      ('car', const Size(1000, 320)),
      ('fish', const Size(1000, 270)),
    ];

    /// The padding the home's scroll view takes, read where the group is.
    EdgeInsets homePadding(WidgetTester tester) =>
        getScrollContainerPadding(tester.element(_startGroup), listen: false);

    /// The height the page has for its groups.
    double contentHeight(WidgetTester tester) {
      final padding = homePadding(tester);
      return tester
              .getSize(
                find
                    .ancestor(
                      of: _startGroup,
                      matching: find.byType(LayoutBuilder),
                    )
                    .first,
              )
              .height -
          padding.vertical;
    }

    for (final (name, size) in windows) {
      testWidgets('a $name window puts Releases and Tracks side by side', (
        tester,
      ) async {
        await _pump(tester, _artistPage(), size: size);
        await tester.pump();
        final releases = tester.getRect(
          find.byType(StartGroupStackedLayout).first,
        );
        final tracks = tester.getRect(
          find.byType(StartGroupStackedLayout).last,
        );
        expect(tracks.left, greaterThanOrEqualTo(releases.right));
        // Nothing lies below the release group.
        expect(
          tester.getTopLeft(find.byType(QueryTrackListView)).dy,
          lessThan(tester.getBottomLeft(_startGroup).dy),
        );
        expect(tracks.top, releases.top);
        expect(find.byType(QueryTrackListView), findsOneWidget);
      });

      testWidgets('a $name window counts the short side as the home does', (
        tester,
      ) async {
        await _pump(tester, _artistPage(), size: size);
        await tester.pump();
        final home = StartGroupImplementation.startLinkDimensionCalculator(
          contentHeight(tester),
          defaultCellSize,
          defaultGapSize,
          const [0, 1],
        ).rows;
        expect(
          _grid(tester, _startGroup).size.height,
          TileGrid.extent(
            math.min(home, maxReleaseShortSide),
            defaultCellSize,
            defaultGapSize,
          ),
        );
      });

      testWidgets('a $name window starts its first group as the home does', (
        tester,
      ) async {
        await _pump(tester, _artistPage(), size: size);
        await tester.pump();
        // The home's scroll view pads its first group by the scroll padding,
        // and its title sits at the stacked layout's own padding.
        expect(
          tester.getTopLeft(find.text('Releases')).dx,
          homePadding(tester).left + StartGroupStackedLayout.padding.left,
        );
      });
    }

    testWidgets('a desktop window is three blocks high at full size', (
      tester,
    ) async {
      await _pump(tester, _artistPage(), size: const Size(1280, 720));
      await tester.pump();
      final grid = _grid(tester, _startGroup);
      expect(
        grid.size.height,
        TileGrid.extent(3, defaultCellSize, defaultGapSize),
      );
      expect(grid.cellSize, defaultCellSize);
      expect(grid.gapSize, defaultGapSize);
    });

    testWidgets('a short window has fewer blocks high', (tester) async {
      await _pump(tester, _artistPage(), size: const Size(1000, 320));
      await tester.pump();
      expect(
        _grid(tester, _startGroup).size.height,
        lessThan(TileGrid.extent(3, defaultCellSize, defaultGapSize)),
      );
    });

    testWidgets('a narrow window keeps the cells and scrolls the row', (
      tester,
    ) async {
      // The first section has the covers of a large tile.
      await _pump(
        tester,
        _artistPage(fetch: (_, _) async => _page(firstCovers: _paths(6))),
        size: const Size(600, 844),
        smallTitles: true,
      );
      await tester.pump();
      final grid = _grid(tester, _startGroup);
      expect(grid.cellSize, defaultCellSize);
      expect(grid.gapSize, defaultGapSize);
      expect(
        find.ancestor(
          of: _startGroup,
          matching: find.byType(SingleChildScrollView),
        ),
        findsOneWidget,
      );
      final padding = homePadding(tester);
      expect(
        tester.getSize(find.byType(QueryTrackListView)).width,
        greaterThanOrEqualTo(
          padding.horizontal +
              largeTrackListRightPlus +
              largeTrackListTileWidth,
        ),
      );
      await _removeCovers(tester);
    });

    testWidgets('crossing the width where the row starts to scroll keeps the '
        'track list and the release tiles', (tester) async {
      await _pump(
        tester,
        _artistPage(fetch: (_, _) async => _page(firstCovers: _paths(6))),
        size: const Size(1280, 720),
        smallTitles: true,
      );
      await tester.pump();
      expect(
        find.ancestor(
          of: _startGroup,
          matching: find.byType(SingleChildScrollView),
        ),
        findsNothing,
      );
      final list = tester.state(find.byType(QueryTrackListView));
      final tile = tester.state(find.byType(ReleaseTile).first);

      tester.view.physicalSize = const Size(600, 844);
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(
        find.ancestor(
          of: _startGroup,
          matching: find.byType(SingleChildScrollView),
        ),
        findsOneWidget,
      );
      expect(tester.state(find.byType(QueryTrackListView)), same(list));
      expect(tester.state(find.byType(ReleaseTile).first), same(tile));

      tester.view.physicalSize = const Size(1280, 720);
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(tester.state(find.byType(QueryTrackListView)), same(list));
      await _removeCovers(tester);
    });

    testWidgets('a wide window gives the rest of the width to Tracks', (
      tester,
    ) async {
      await _pump(tester, _artistPage(), size: const Size(1280, 720));
      await tester.pump();
      expect(
        tester.getTopRight(find.byType(QueryTrackListView)).dx,
        closeTo(1280 - StartGroupStackedLayout.padding.right, 0.001),
      );
    });
  });

  group('the name', () {
    testWidgets("is drawn as an album's title is, as the text the header "
        'flips with its title', (tester) async {
      await _pump(tester, _artistPage());
      await tester.pump();
      final name = find.text('Hiroyuki Sawano');
      final title = find.ancestor(
        of: name,
        matching: find.byType(QueryTracksTitle),
      );
      expect(title, findsOneWidget);
      final flip = tester.widget<FlipText>(
        find.ancestor(of: name, matching: find.byType(FlipText)),
      );
      expect(flip.flipKey, 'child:/artists/detail');

      // An album's title beside it, to compare the two as they are drawn.
      final context = tester.element(title);
      final at = tester.getTopLeft(title);
      final drawn = tester.getRect(name).shift(-at);
      final size = tester.getSize(title);
      final color = tester.widget<Text>(name).style!.color;
      final inactive = FluentTheme.of(context).inactiveColor;
      await _pump(
        tester,
        Builder(
          builder: (context) => const Align(
            alignment: Alignment.topLeft,
            child: QueryTracksTitle(title: 'Hiroyuki Sawano'),
          ),
        ),
      );
      final plain = find.byType(QueryTracksTitle);
      final plainName = find.text('Hiroyuki Sawano');
      expect(find.byType(FlipText), findsNothing);
      final plainDrawn = tester
          .getRect(plainName)
          .shift(-tester.getTopLeft(plain));
      // The same glyphs in the same place: the 14 at 1.2 from its middle, and
      // the 17 at 16.8 / 17 from its top left moved up.
      expect(drawn.left, closeTo(plainDrawn.left, 0.001));
      expect(drawn.top, closeTo(plainDrawn.top, 0.001));
      expect(drawn.width, closeTo(plainDrawn.width, 0.001));
      expect(drawn.height, closeTo(plainDrawn.height, 0.001));
      expect(drawn.height, closeTo(14 * 1.2, 0.001));
      // The same box, so what is under the name is where it was.
      expect(size.width, closeTo(tester.getSize(plain).width, 0.001));
      expect(size.height, closeTo(tester.getSize(plain).height, 0.001));
      // The same colour.
      expect(tester.widget<Text>(plainName).style!.color, inactive);
      expect(color, inactive);
    });
  });

  group('the spacing of the groups', () {
    const releasesTitle = 'Releases';
    const tracksTitle = 'Tracks';

    testWidgets('a desktop window puts no gap between the two groups', (
      tester,
    ) async {
      await _pump(tester, _artistPage());
      await tester.pump();
      // Nothing lies between the two stacked layouts, so what separates the
      // tiles of Releases from the title of Tracks is their own paddings.
      final releases = tester.getRect(
        find.byType(StartGroupStackedLayout).first,
      );
      final tracks = tester.getRect(find.byType(StartGroupStackedLayout).last);
      expect(tracks.left, releases.right);
      expect(
        tester.getTopLeft(find.text(tracksTitle)).dx - tracks.left,
        StartGroupStackedLayout.padding.left,
      );
      expect(
        releases.right -
            tester
                .getTopRight(
                  find.descendant(
                    of: _startGroup,
                    matching: find.byType(TileGrid),
                  ),
                )
                .dx,
        greaterThanOrEqualTo(StartGroupStackedLayout.padding.right),
      );
    });

    testWidgets('tracks start under their title as release tiles do', (
      tester,
    ) async {
      final layout = StartScreenLayoutManager();
      addTearDown(layout.dispose);
      final tracks = [
        for (var i = 0; i < 6; i++)
          InternalMediaFile(
            id: i,
            path: '',
            artist: '',
            album: '',
            title: 'track-$i',
            duration: 0,
            coverArtPath: '',
            trackNumber: null,
          ),
      ];
      await _pump(
        tester,
        PageContentFrame(
          top: false,
          child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
            value: layout,
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                StartGroup<int>(
                  groupIndex: 0,
                  groupTitle: releasesTitle,
                  items: const [0, 1],
                  constraints: const BoxConstraints(maxHeight: 600),
                  groupLayoutVariation: StartGroupGroupLayoutVariation.stacked,
                  gapSize: defaultGapSize,
                  itemBuilder: (context, index) => ReleaseTile(
                    label: 'Albums',
                    covers: const [],
                    onPressed: () {},
                  ),
                  direction: Axis.horizontal,
                ),
                Expanded(
                  child: StartGroupStackedLayout(
                    groupTitle: tracksTitle,
                    child: QueryTrackListView(
                      layoutManager: layout,
                      queries: _trackQueries,
                      mode: null,
                      layout: DeviceType.tv,
                      underTitle: true,
                      loadMixPage: (queries, cursor, size) async =>
                          cursor == 0 ? tracks : [],
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      for (var i = 0; i < 5; i++) {
        await tester.pump(const Duration(milliseconds: 200));
      }

      final releaseOffset =
          tester.getTopLeft(find.byType(ReleaseTile).first).dy -
          tester.getTopLeft(find.text(releasesTitle)).dy;
      final trackOffset =
          tester.getTopLeft(find.byType(LargeScreenTrackListItem).first).dy -
          tester.getTopLeft(find.text(tracksTitle)).dy;
      expect(releaseOffset, StartGroupStackedLayout.childPadding.top);
      expect(trackOffset, releaseOffset);
      // The first column starts at the child padding, as the tiles do.
      expect(
        tester.getTopLeft(find.byType(LargeScreenTrackListItem).first).dx -
            tester.getTopLeft(find.text(tracksTitle)).dx,
        StartGroupStackedLayout.childPadding.left,
      );

      // Over the title the tiles are solid in every state, so the title does
      // not show through them.
      final tile = find.byType(LargeScreenTrackListItem).first;
      expect(
        tester.widget<LargeScreenTrackListItem>(tile).solidBackground,
        isTrue,
      );
      final fill = tester
          .widget<Button>(
            find.descendant(of: tile, matching: find.byType(Button)),
          )
          .style!
          .backgroundColor!;
      for (final states in [
        <WidgetState>{},
        {WidgetState.hovered},
        {WidgetState.pressed},
      ]) {
        expect(fill.resolve(states)!.a, 1.0);
      }
    });
  });
}
