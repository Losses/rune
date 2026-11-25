import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/artist/artist_page.dart';
import '../../screens/artist/artist_section_albums_provider.dart';
import '../../screens/collection/collection.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../utils/api/fetch_artist_section_albums.dart';
import '../../utils/query_list.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../widgets/band_screen/band_screen.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/collection_item.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/turntile/turntile_screen.dart';

const _queries = [MixQuery(operator: 'lib::artist_section', parameter: '7:x')];
final _queryList = QueryList.fromMixQuery(_queries);

ArtistSection _section({
  ArtistSectionKind kind = ArtistSectionKind.albums,
  String title = '',
  ArtistSectionOpens opens = ArtistSectionOpens.songs,
  Album? album,
}) => ArtistSection(
  kind: kind,
  title: title,
  opens: opens,
  queries: _queries,
  album: album,
  covers: const [],
  count: 1,
);

Collection _album(int id) => Collection(
  id: id,
  name: 'Album $id',
  queries: const [],
  collectionType: CollectionType.album,
  coverArtMap: const {},
  readonly: true,
);

Future<AppLocalizations> _s() =>
    AppLocalizations.delegate.load(const Locale('en'));

/// The library's groups of the albums from [cursor], at most [size], of a
/// section of [total] albums whose album at index `i` is `100 + i`, titled by
/// [titleOf] of its index, as a library groups them: a group a page cuts goes
/// on in the next page under the same title.
List<ArtistSectionAlbumGroup> _groups(
  int cursor,
  int size,
  int total,
  String Function(int index) titleOf,
) {
  final groups = <ArtistSectionAlbumGroup>[];
  for (var i = cursor; i < cursor + size && i < total; i++) {
    final title = titleOf(i);
    if (groups.isNotEmpty && groups.last.groupTitle == title) {
      groups.last.albumIds.add(100 + i);
    } else {
      groups.add(
        ArtistSectionAlbumGroup(groupTitle: title, albumIds: [100 + i]),
      );
    }
  }
  return groups;
}

/// Ten albums a year, the newest year first.
String _tenAYear(int index) => '${2030 - index ~/ 10}';

ArtistSectionAlbumsProvider _provider({
  required List<(int, int)> asked,
  int total = 30,
  String Function(int index) titleOf = _tenAYear,
}) => ArtistSectionAlbumsProvider(
  queries: _queryList,
  title: 'Albums',
  contentChanged: const Stream.empty(),
  fetchAlbumGroups: (queries, cursor, size) async {
    asked.add((cursor, size));
    return _groups(cursor, size, total, titleOf);
  },
  fetchAlbums: (type, ids) async => ids.map(_album).toList(),
);

Future<void> _pump(
  WidgetTester tester,
  ArtistSectionAlbumsProvider provider,
  Size size,
) async {
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
      ],
      child: FluentApp(
        builder: (context, child) => RevealEffectContext(child: child!),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: FlipAnimationManager(
          child: CollectionPage(
            collectionType: CollectionType.album,
            createProvider: () => provider,
          ),
        ),
      ),
    ),
  );
  await tester.pump(const Duration(seconds: 1));
  for (var i = 0; i < 5; i++) {
    await tester.pump(const Duration(milliseconds: 500));
  }
}

void main() {
  group('the section label', () {
    test('is the library title when it gives one', () async {
      final s = await _s();
      expect(artistSectionName(s, _section(title: 'Top Hits')), 'Top Hits');
    });

    test('falls back to the name of the kind', () async {
      final s = await _s();
      String name(ArtistSectionKind kind) =>
          artistSectionName(s, _section(kind: kind));
      expect(name(ArtistSectionKind.singles), 'Singles & EPs');
      expect(name(ArtistSectionKind.latestRelease), 'Latest Release');
      expect(name(ArtistSectionKind.essentialAlbums), 'Essential Albums');
      expect(name(ArtistSectionKind.albums), 'Albums');
    });
  });

  group('opening a section', () {
    test('songs open the songs page', () async {
      final target = artistSectionLinks(await _s(), [
        _section(title: 'T'),
      ], 'Artist').single;
      expect(target.route, '/artists/section');
      final parameter = target.arguments as ArtistSectionParameter;
      expect(parameter.queries, _queryList);
      expect(parameter.title, 'T');
      expect(parameter.artistName, 'Artist');
    });

    test('an album opens that album', () async {
      final target = artistSectionLinks(await _s(), [
        _section(
          opens: ArtistSectionOpens.album,
          album: Album(id: 9, name: 'Nine'),
        ),
      ], 'Artist').single;
      expect(target.route, '/albums/detail');
      final parameter = target.arguments as QueryTracksParameter;
      expect(parameter.id, 9);
      expect(parameter.title, 'Nine');
    });

    test(
      'albums open the albums page with queries, title and artist',
      () async {
        final target = artistSectionLinks(await _s(), [
          _section(title: 'T', opens: ArtistSectionOpens.albums),
        ], 'Artist').single;
        expect(target.route, '/artists/albums');
        final parameter = target.arguments as ArtistSectionParameter;
        expect(parameter.queries, _queryList);
        expect(parameter.title, 'T');
        expect(parameter.artistName, 'Artist');
      },
    );

    test('a page of a section gets the sections that open a list and its '
        'place among them', () async {
      final links = artistSectionLinks(await _s(), [
        _section(
          kind: ArtistSectionKind.latestRelease,
          opens: ArtistSectionOpens.album,
          album: Album(id: 9, name: 'Nine'),
        ),
        _section(opens: ArtistSectionOpens.albums),
        _section(kind: ArtistSectionKind.singles),
      ], 'Artist');

      expect(links.map((x) => x.title), [
        'Latest Release',
        'Albums',
        'Singles & EPs',
      ]);
      expect(links.map((x) => x.route), [
        '/albums/detail',
        '/artists/albums',
        '/artists/section',
      ]);
      expect((links[0].arguments as QueryTracksParameter).id, 9);
      for (final index in [1, 2]) {
        final parameter = links[index].arguments as ArtistSectionParameter;
        expect(parameter.sections.map((x) => x.title), [
          'Albums',
          'Singles & EPs',
        ]);
        expect(parameter.sections.map((x) => x.route), [
          '/artists/albums',
          '/artists/section',
        ]);
        expect(parameter.index, index - 1);
        expect(parameter.title, links[index].title);
        expect(parameter.artistName, 'Artist');
        // Another section of the row opens with the same row, at its place.
        final other = parameter.at(2 - index);
        expect(other.title, links[3 - index].title);
        expect(other.queries, parameter.sections[2 - index].queries);
        expect(other.sections, same(parameter.sections));
        expect(other.index, 2 - index);
      }
    });
  });

  group('the request', () {
    test(
      'names the section and pages, and the reply of its call answers',
      () async {
        final responses =
            StreamController<
              RustSignalPack<FetchArtistSectionAlbumsResponse>
            >.broadcast();
        final failures = StreamController<RequestFailedResponse>.broadcast();
        final requests = <FetchArtistSectionAlbumsRequest>[];
        final groups = fetchArtistSectionAlbumsWithStreams(
          _queries,
          24,
          24,
          responses: responses.stream,
          failures: failures.stream,
          sendRequest: requests.add,
        );
        expect(requests.single.queries.single.parameter, '7:x');
        expect(requests.single.cursor, 24);
        expect(requests.single.pageSize, 24);
        responses.add(
          RustSignalPack(
            FetchArtistSectionAlbumsResponse(
              callId: requests.single.callId,
              groups: [
                ArtistSectionAlbumGroup(groupTitle: '2024', albumIds: [3, 1]),
                ArtistSectionAlbumGroup(groupTitle: '#', albumIds: [2]),
              ],
            ),
            Uint8List(0),
          ),
        );
        expect(await groups, [
          ArtistSectionAlbumGroup(groupTitle: '2024', albumIds: [3, 1]),
          ArtistSectionAlbumGroup(groupTitle: '#', albumIds: [2]),
        ]);
        await responses.close();
        await failures.close();
      },
    );
  });

  group('the albums page', () {
    testWidgets('the library\'s groups are drawn under their titles as the '
        'library\'s own pages draw theirs', (tester) async {
      final asked = <(int, int)>[];
      final provider = _provider(
        asked: asked,
        total: 5,
        titleOf: (index) => ['2024', '2024', '2021', '#', '#'][index],
      );
      await _pump(tester, provider, const Size(1280, 720));
      expect(find.byType(StartScreen), findsOneWidget);
      expect(provider.items.map((group) => group.groupTitle), [
        '2024',
        '2021',
        '#',
      ]);
      expect(provider.items.map((group) => group.items.map((x) => x.id)), [
        [100, 101],
        [102],
        [103, 104],
      ]);
      final drawn = tester.widgetList<StartGroup<InternalCollection>>(
        find.byType(StartGroup<InternalCollection>),
      );
      expect(drawn.map((group) => group.groupTitle), ['2024', '2021', '#']);
      for (final title in ['2024', '2021', '#']) {
        expect(find.text(title), findsOneWidget);
      }
      expect(find.byType(CollectionItem), findsNWidgets(5));
      expect(provider.isLastPage, isTrue);
    });

    test('a group a page cuts goes on in one group on the next', () async {
      final asked = <(int, int)>[];
      final provider = _provider(asked: asked);
      addTearDown(provider.dispose);
      await provider.fetchData();
      expect(provider.isLastPage, isFalse);
      // The first page ends four albums into 2028.
      expect(provider.items.map((group) => group.groupTitle), [
        '2030',
        '2029',
        '2028',
      ]);
      expect(provider.items.last.items.length, 4);
      await provider.fetchData();
      expect(asked, [(0, 24), (24, 24)]);
      expect(provider.items.map((group) => group.groupTitle), [
        '2030',
        '2029',
        '2028',
      ]);
      expect(provider.items.map((group) => group.items.map((x) => x.id)), [
        for (var year = 0; year < 3; year++)
          [for (var i = 0; i < 10; i++) 100 + year * 10 + i],
      ]);
      expect(provider.isLastPage, isTrue);
    });

    testWidgets('a curated section\'s one untitled group is drawn under the '
        'section\'s name, in the library\'s order', (tester) async {
      final asked = <(int, int)>[];
      final provider = ArtistSectionAlbumsProvider(
        queries: _queryList,
        title: 'Essential Albums',
        contentChanged: const Stream.empty(),
        fetchAlbumGroups: (queries, cursor, size) async {
          asked.add((cursor, size));
          final ids = [for (var i = 0; i < 30; i++) 200 - i];
          return [
            ArtistSectionAlbumGroup(
              groupTitle: '',
              albumIds: ids.skip(cursor).take(size).toList(),
            ),
          ];
        },
        fetchAlbums: (type, ids) async => ids.map(_album).toList(),
      );
      await _pump(tester, provider, const Size(1280, 720));
      // 30 albums are two pages, the first of which the screen has loaded.
      for (var i = 0; i < 2 && !provider.isLastPage; i++) {
        await provider.fetchData();
      }
      expect(provider.isLastPage, isTrue);
      // The group titles of the year picker are asked for besides the pages.
      expect(asked.where((request) => request.$2 == 24).toSet(), {
        (0, 24),
        (24, 24),
      });
      expect(provider.items.map((group) => group.groupTitle), [
        'Essential Albums',
      ]);
      expect(provider.items.single.items.map((x) => x.id), [
        for (var i = 0; i < 30; i++) 200 - i,
      ]);
      await tester.pump(const Duration(milliseconds: 500));
      expect(find.text('Essential Albums'), findsOneWidget);
    });

    test('an album the library cannot name is left out of its group', () async {
      final provider = ArtistSectionAlbumsProvider(
        queries: _queryList,
        title: 'Albums',
        contentChanged: const Stream.empty(),
        fetchAlbumGroups: (queries, cursor, size) async => [
          ArtistSectionAlbumGroup(groupTitle: '2024', albumIds: [1, 2]),
          ArtistSectionAlbumGroup(groupTitle: '2023', albumIds: [3]),
        ],
        fetchAlbums: (type, ids) async =>
            ids.where((id) => id != 2 && id != 3).map(_album).toList(),
      );
      addTearDown(provider.dispose);
      await provider.fetchData();
      expect(provider.items.map((group) => group.groupTitle), ['2024']);
      expect(provider.items.single.items.map((x) => x.id), [1]);
    });

    test('does not touch the albums page cache', () async {
      addTearDown(CollectionCache().clearAll);
      final sentinel = [
        Group<InternalCollection>(groupTitle: 'sentinel', items: []),
      ];
      CollectionCache().cachePage(CollectionType.album, 0, sentinel);
      final provider = _provider(asked: []);
      addTearDown(provider.dispose);
      await provider.fetchData();
      expect(
        CollectionCache().getPage(CollectionType.album, 0),
        same(sentinel),
      );
      await provider.reloadData();
      expect(
        CollectionCache().getPage(CollectionType.album, 0),
        same(sentinel),
      );
      expect(provider.items.first.items, isNotEmpty);
    });

    test('lists the titles of all its groups for the year picker', () async {
      final asked = <(int, int)>[];
      final provider = _provider(asked: asked);
      addTearDown(provider.dispose);
      final summary = await provider.summary;
      expect(summary.map((group) => group.groupTitle), [
        '2030',
        '2029',
        '2028',
      ]);
      // The titles come from one request that covers every album.
      expect(asked.single.$1, 0);
      expect(asked.single.$2, greaterThanOrEqualTo(30));
    });

    test('titles the summary of an untitled group by the section', () async {
      final provider = ArtistSectionAlbumsProvider(
        queries: _queryList,
        title: 'Essential Albums',
        contentChanged: const Stream.empty(),
        fetchAlbumGroups: (queries, cursor, size) async => [
          ArtistSectionAlbumGroup(groupTitle: '', albumIds: [1, 2]),
        ],
        fetchAlbums: (type, ids) async => ids.map(_album).toList(),
      );
      addTearDown(provider.dispose);
      expect((await provider.summary).map((group) => group.groupTitle), [
        'Essential Albums',
      ]);
    });

    testWidgets('a click on a year opens the picker with the years', (
      tester,
    ) async {
      final provider = _provider(
        asked: [],
        total: 5,
        titleOf: (index) => ['2024', '2024', '2021', '#', '#'][index],
      );
      await _pump(tester, provider, const Size(1280, 720));
      await tester.tap(find.text('2021'));
      await tester.pump(const Duration(milliseconds: 500));
      await tester.pump(const Duration(milliseconds: 500));
      final picker = find.byType(ContentDialog);
      expect(picker, findsOneWidget);
      expect(
        find.descendant(of: picker, matching: find.byType(Button)),
        findsNWidgets(4),
      );
      for (final title in ['2024', '2021', '#']) {
        expect(
          find.descendant(of: picker, matching: find.text(title)),
          findsOneWidget,
        );
      }
    });

    for (final (name, size, expected) in [
      ('zune', const Size(320, 700), TurntileScreen),
      ('dock', const Size(120, 700), BandScreen),
      ('band', const Size(640, 130), BandScreen),
    ]) {
      testWidgets('a $name window draws the screen the albums page does', (
        tester,
      ) async {
        await _pump(tester, _provider(asked: [], total: 3), size);
        expect(find.byType(StartScreen), findsNothing);
        expect(find.byType(expected), findsOneWidget);
      });
    }
  });
}
