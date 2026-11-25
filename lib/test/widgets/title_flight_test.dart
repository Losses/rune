import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../main.dart' show cafeMode;
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../screens/artist/artist_page.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/history.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_transition_parameter.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/navigation_bar/flip_text.dart';
import '../../widgets/navigation_bar/flip_text_animation.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/parent_link.dart';
import '../../widgets/router/no_effect_page_route.dart';
import '../../widgets/router/router_animation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

/// The page of [settings]: the real artist page for an artist, a stand-in for
/// the others, all inside the app's real page route and page transition.
Route<void> _route(RouteSettings settings) {
  late final NoEffectPageRoute<void> route;
  route = NoEffectPageRoute<void>(
    settings: settings,
    builder: (context) {
      final arguments = settings.arguments;
      final parameters = arguments is RouterTransitionParameter
          ? arguments.parameters
          : null;
      final Widget page = switch ((settings.name, parameters)) {
        ('/artists/detail', QueryTracksParameter(:final id, :final title)) =>
          ArtistPage(
            artistId: id,
            name: title,
            fetch: (id, _) async =>
                FetchArtistPageResponse(artistId: id, sections: [], tracks: []),
            contentChanged: const Stream.empty(),
          ),
        _ => Center(child: Text('${settings.name}')),
      };
      return RouterAnimation(page, route: route);
    },
  );
  return route;
}

/// The app's navigator of pages under the real navigation bar, as
/// `header_return_regression_test` has it, with the real page route.
Future<void> _pumpApp(WidgetTester tester) async {
  tester.view.physicalSize = const Size(1200, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  cafeMode = false;
  $router.update('/library', null);
  $history.reset();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
        ChangeNotifierProvider<RouterPathProvider>.value(value: $router),
        ChangeNotifierProvider<LinuxCustomWindowControlsProvider>.value(
          value: LinuxCustomWindowControlsProvider(),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
        navigatorObservers: [routerAnimationObserver],
        onGenerateRoute: _route,
        initialRoute: '/library',
        builder: (context, child) => Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (context) => RevealEffectContext(
                // The bar and the pages are built by one widget that hears the
                // router, as RuneRouterFrameImplementation builds them.
                child: FlipAnimationManager(
                  child: ListenableBuilder(
                    listenable: $router,
                    builder: (context, _) => Column(
                      children: [
                        SizedBox(
                          height: 200,
                          child: NavigationBar(path: $router.path),
                        ),
                        Expanded(child: child!),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    ),
  );
  await tester.pump(const Duration(seconds: 2));
}

ArtistSection _section(ArtistSectionKind kind, ArtistSectionOpens opens) =>
    ArtistSection(
      kind: kind,
      title: '',
      opens: opens,
      queries: [
        MixQuery(operator: 'lib::artist_section', parameter: kind.name),
      ],
      album: opens == ArtistSectionOpens.album
          ? const Album(id: 9, name: 'Nine')
          : null,
      covers: const [],
      count: 1,
    );

/// An artist's links as its page makes them: Latest Release, which opens one
/// album, Albums, which opens its albums, and Appears On, which opens songs.
List<ArtistSectionLink> _links([String artist = 'EGOIST']) =>
    artistSectionLinks(lookupAppLocalizations(const Locale('en')), [
      _section(ArtistSectionKind.latestRelease, ArtistSectionOpens.album),
      _section(ArtistSectionKind.albums, ArtistSectionOpens.albums),
      _section(ArtistSectionKind.appearsOn, ArtistSectionOpens.songs),
    ], artist);

/// Goes somewhere and returns the flight of a title that starts, if any,
/// once it is drawn, then lets the pages and the flight settle.
Future<FlipTextAnimation?> _flightOf(
  WidgetTester tester,
  void Function() go,
) async {
  go();
  await tester.pump();
  await tester.pump();
  final flights = tester
      .widgetList<FlipTextAnimation>(find.byType(FlipTextAnimation))
      .toList();
  expect(flights.length, lessThanOrEqualTo(1));
  await tester.pumpAndSettle();
  return flights.firstOrNull;
}

Finder _flipText(String flipKey, {bool skipOffstage = true}) =>
    find.byWidgetPredicate(
      (widget) => widget is FlipText && widget.flipKey == flipKey,
      skipOffstage: skipOffstage,
    );

/// Where the text [flipKey] is drawn now that nothing moves, in the
/// coordinates the flight is drawn in.
Offset _restingAt(WidgetTester tester, String flipKey) => tester.getTopLeft(
  find.descendant(of: _flipText(flipKey), matching: find.byType(Transform)),
);

const _name = 'child:/artists/detail';
const _artistTitle = 'title:/artists/detail';

Future<void> _openArtist(WidgetTester tester, int id, String name) async {
  $push('/artists/detail', arguments: QueryTracksParameter(id, name));
  await tester.pumpAndSettle();
}

ParentLink _title(WidgetTester tester) =>
    tester.widget<ParentLink>(find.byType(ParentLink));

void _tapTitle(WidgetTester tester) => _title(tester).onPressed();

/// Whether the text [flipKey], or the one of them reading [text], shows, on
/// a page shown or under one.
bool _visible(WidgetTester tester, String flipKey, {String? text}) => tester
    .widget<Visibility>(
      find
          .descendant(
            of: find.byWidgetPredicate(
              (widget) =>
                  widget is FlipText &&
                  widget.flipKey == flipKey &&
                  (text == null || widget.text == text),
              skipOffstage: false,
            ),
            matching: find.byType(Visibility, skipOffstage: false),
            skipOffstage: false,
          )
          .first,
    )
    .visible;

/// Opens [link] from the artist page shown and closes it again with the
/// title, and checks that the name flies from where it rests to where the
/// title rests, and back.
Future<void> _upAndDown(
  WidgetTester tester,
  ArtistSectionLink link,
  String artist,
) async {
  final name = _restingAt(tester, _name);

  final up = await _flightOf(
    tester,
    () => $push(link.route, arguments: link.arguments),
  );
  expect($router.path, link.route);
  expect(_title(tester).text, artist);
  expect(_title(tester).titleFlipKey, _artistTitle);
  final title = _restingAt(tester, _artistTitle);
  expect(up!.text, artist);
  expect(up.fromPositions.position, name);
  expect(up.toPositions.position, title);
  // Up into the big title.
  expect(title.dy, lessThan(name.dy));
  expect(up.fromStyles.scale, closeTo(14 * 1.2 / 17, 0.0001));
  expect(up.toStyles.scale, 5);

  final down = await _flightOf(tester, () => _tapTitle(tester));
  expect($router.path, '/artists/detail');
  expect(down!.text, artist);
  expect(down.fromPositions.position, title);
  expect(down.toPositions.position, name);
  expect(_restingAt(tester, _name), name);
  expect(_visible(tester, _name), isTrue);
}

void main() {
  testWidgets('the artist page takes the title of the artists and gives it '
      'back, as a page of the library does', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    const child = 'child:/artists';
    const title = 'title:/artists';
    final atLink = _restingAt(tester, child);

    final up = await _flightOf(
      tester,
      () => $push(
        '/artists/detail',
        arguments: QueryTracksParameter(7, 'EGOIST'),
      ),
    );
    expect(up!.text, 'Artists');
    expect(up.fromPositions.position, atLink);
    expect(up.toPositions.position, _restingAt(tester, title));
    expect(_title(tester).text, 'Artists');

    // The name is drawn under the header as an album's is, and is the text
    // that flies when a page that names the artist opens.
    expect(find.text('EGOIST'), findsOneWidget);
    expect(
      find.descendant(of: find.byType(ArtistPage), matching: _flipText(_name)),
      findsOneWidget,
    );

    final atTitle = _restingAt(tester, title);
    final down = await _flightOf(tester, () => _tapTitle(tester));
    expect($router.path, '/artists');
    expect(down!.fromPositions.position, atTitle);
    expect(down.toPositions.position, atLink);
    expect(_title(tester).text, 'Library');
  });

  testWidgets('the name flies from and back to where it rests between the '
      'artist page and the page of a section of albums', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    await _upAndDown(tester, _links()[1], 'EGOIST');
  });

  testWidgets('the name flies from and back to where it rests between the '
      'artist page and the page of a section of songs', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    await _upAndDown(tester, _links()[2], 'EGOIST');
  });

  testWidgets('the name flies from and back to where it rests between the '
      'artist page and an album it opens', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    await _upAndDown(tester, _links()[0], 'EGOIST');
  });

  testWidgets('the name flies the same a second time, and after the page of '
      'another section was shown in place of the one opened', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    final name = _restingAt(tester, _name);
    await _upAndDown(tester, _links()[1], 'EGOIST');
    await _upAndDown(tester, _links()[0], 'EGOIST');

    // Into the albums, from there to the songs as a press on the link of the
    // row does, and back with the title.
    final albums = _links()[1];
    final songs = _links()[2];
    $push(albums.route, arguments: albums.arguments);
    await tester.pumpAndSettle();
    expect(
      await _flightOf(
        tester,
        () => $replace(songs.route, arguments: songs.arguments),
      ),
      isNull,
    );
    expect($router.path, '/artists/section');
    final title = _restingAt(tester, _artistTitle);
    final down = await _flightOf(tester, () => _tapTitle(tester));
    expect($router.path, '/artists/detail');
    expect(down!.fromPositions.position, title);
    expect(down.toPositions.position, name);
  });

  testWidgets('the section flies between its link and the title over an '
      'album it opens', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    final name = _restingAt(tester, _name);
    final link = _links()[1];
    $push(link.route, arguments: link.arguments);
    await tester.pumpAndSettle();
    const child = 'child:/artists/albums';
    const title = 'title:/artists/albums';
    final atLink = _restingAt(tester, child);

    final up = await _flightOf(
      tester,
      () => $push('/albums/detail', arguments: QueryTracksParameter(3, 'X')),
    );
    expect(up!.text, 'Albums');
    expect(up.fromPositions.position, atLink);
    expect(up.toPositions.position, _restingAt(tester, title));
    expect(_title(tester).text, 'Albums');
    expect(_title(tester).titleFlipKey, title);

    final atTitle = _restingAt(tester, title);
    final down = await _flightOf(tester, () => _tapTitle(tester));
    expect(down!.text, 'Albums');
    expect(down.fromPositions.position, atTitle);
    expect(down.toPositions.position, _restingAt(tester, child));
    expect(_restingAt(tester, child), atLink);
    expect($router.path, '/artists/albums');
    expect(_title(tester).text, 'EGOIST');

    // And from the section back to the artist page its name comes down.
    final artistTitle = _restingAt(tester, _artistTitle);
    final back = await _flightOf(tester, () => _tapTitle(tester));
    expect($router.path, '/artists/detail');
    expect(back!.text, 'EGOIST');
    expect(back.fromPositions.position, artistTitle);
    expect(back.toPositions.position, name);
  });

  testWidgets('from search the name flies as from the artists', (tester) async {
    await _pumpApp(tester);
    $push('/search');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    expect(_title(tester).text, 'Search');
    await _upAndDown(tester, _links()[1], 'EGOIST');
    await _upAndDown(tester, _links()[0], 'EGOIST');
    expect(_title(tester).text, 'Search');
  });

  testWidgets('another artist opened over an album and closed again flies '
      'nothing, and back on the first its own name flies', (tester) async {
    await _pumpApp(tester);
    $push('/artists');
    await tester.pumpAndSettle();
    await _openArtist(tester, 7, 'EGOIST');
    final name = _restingAt(tester, _name);
    final link = _links()[0];
    $push(link.route, arguments: link.arguments);
    await tester.pumpAndSettle();

    // Another artist from the album, as a track's Go to Artist does, is no
    // return to the artist the title names, and closing it opens nothing.
    expect(
      await _flightOf(
        tester,
        () => $push(
          '/artists/detail',
          arguments: QueryTracksParameter(8, 'Other'),
        ),
      ),
      isNull,
    );
    expect(_title(tester).text, 'Artists');
    expect(_visible(tester, _name, text: 'Other'), isTrue);

    // Its own name flies to its own title and back.
    await _upAndDown(tester, _links('Other')[1], 'Other');

    expect(await _flightOf(tester, $pop), isNull);
    expect($router.path, '/albums/detail');
    expect(_title(tester).text, 'EGOIST');

    final title = _restingAt(tester, _artistTitle);
    final down = await _flightOf(tester, () => _tapTitle(tester));
    expect($router.path, '/artists/detail');
    expect(_title(tester).text, 'Artists');
    expect(down!.text, 'EGOIST');
    expect(down.fromPositions.position, title);
    expect(down.toPositions.position, name);
    expect(_restingAt(tester, _name), name);
    expect(_visible(tester, _name), isTrue);

    final section = _links()[1];
    $push(section.route, arguments: section.arguments);
    await tester.pump();
    await tester.pump();
    final up = tester.widget<FlipTextAnimation>(find.byType(FlipTextAnimation));
    expect(up.text, 'EGOIST');
    expect(up.fromPositions.position, name);
    expect(up.toPositions.position, title);
    // The page's own name is what flies, so it is hidden meanwhile.
    expect(_visible(tester, _name, text: 'EGOIST'), isFalse);
    await tester.pumpAndSettle();
    expect(_visible(tester, _name, text: 'EGOIST'), isTrue);
  });

  testWidgets('a text hidden for a flight shows again after it, though '
      'another took its key meanwhile', (tester) async {
    final another = ValueNotifier(false);
    addTearDown(another.dispose);
    await tester.pumpWidget(
      FluentApp(
        home: FlipAnimationManager(
          child: ValueListenableBuilder(
            valueListenable: another,
            builder: (context, another, _) => Stack(
              children: [
                const Positioned(
                  left: 10,
                  top: 300,
                  child: FlipText(flipKey: 'k', text: 'First'),
                ),
                const Positioned(
                  left: 10,
                  top: 10,
                  child: FlipText(flipKey: 't', text: 'Title', scale: 5),
                ),
                if (another)
                  const Positioned(
                    left: 10,
                    top: 500,
                    child: FlipText(flipKey: 'k', text: 'Another'),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    bool shows(String text) => tester
        .widget<Visibility>(
          find.descendant(
            of: find.widgetWithText(FlipText, text),
            matching: find.byType(Visibility),
          ),
        )
        .visible;

    await tester.pump();
    tester
        .state<FlipAnimationManagerState>(find.byType(FlipAnimationManager))
        .flipAnimation('k', 't');
    await tester.pump();
    await tester.pump();
    expect(find.byType(FlipTextAnimation), findsOneWidget);
    expect(shows('First'), isFalse);

    another.value = true;
    await tester.pumpAndSettle();
    expect(find.byType(FlipTextAnimation), findsNothing);
    expect(shows('First'), isTrue);
  });

  testWidgets('a text gone from the header is no place to fly from', (
    tester,
  ) async {
    await _pumpApp(tester);
    // A page outside the navigation has a link to search in its row.
    $push('/lyrics');
    await tester.pumpAndSettle();
    expect(_flipText('child:/search'), findsOneWidget);
    $push('/search');
    await tester.pumpAndSettle();
    expect(_flipText('child:/search'), findsNothing);

    expect(
      await _flightOf(
        tester,
        () => $push(
          '/artists/detail',
          arguments: QueryTracksParameter(7, 'EGOIST'),
        ),
      ),
      isNull,
    );
  });
}
