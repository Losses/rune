import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_manager.dart';
import '../../providers/library_open_state.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../screens/query_tracks/query_tracks_list.dart';
import '../../screens/tracks/track_list.dart';
import '../../utils/query_list.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/no_items.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../widgets/turntile/turntile_screen.dart';

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  _Responsive(this.device);

  final DeviceType device;

  @override
  DeviceType get currentDeviceType => device;
  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) => device;
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

List<InternalMediaFile> _tracks(int from, int count) => [
  for (var i = from; i < from + count; i++)
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

/// A page of a collection: three groups of twelve albums.
List<Group<InternalCollection>> _groups(int page) => [
  for (var group = 0; group < 3; group++)
    Group<InternalCollection>(
      groupTitle: 'group-$page-$group',
      items: [
        for (var i = 0; i < 12; i++)
          InternalCollection(
            id: page * 100 + group * 12 + i,
            name: 'album-$page-$group-$i',
            queries: const QueryList([]),
            collectionType: CollectionType.album,
            coverArtMap: const {},
            readonly: false,
          ),
      ],
    ),
];

class _LocalLibrary extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  bool get hasCatalog => true;

  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    shareSync: false,
    recordId: 'local-record',
    alias: 'Music',
    kind: LibraryRecordKindSummaryLocal(
      libraryId: 'library-id',
      path: '/music',
    ),
  );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// What the empty state of a list needs: its scan button reads the open library.
List<ChangeNotifierProvider> _emptyStateProviders() => [
  ChangeNotifierProvider<LibraryManagerProvider>(
    create: (_) => LibraryManagerProvider(sendScanRequest: (_) {}),
  ),
  ChangeNotifierProvider<LibraryOpenStateProvider>(
    create: (_) => _LocalLibrary(),
  ),
];

// Lets the loads and the frames they schedule run, and the animation timers
// they start end.
Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 12; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<void> _pump(
  WidgetTester tester,
  Widget Function(StartScreenLayoutManager layout) home, {
  DeviceType device = DeviceType.tv,
  List<ChangeNotifierProvider> providers = const [],
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
          value: _Responsive(device),
        ),
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: _Screen()),
        ChangeNotifierProvider<StartScreenLayoutManager>.value(value: layout),
        ...providers,
      ],
      child: FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        builder: (context, child) => RevealEffectContext(child: child!),
        home: PageContentFrame(top: false, child: home(layout)),
      ),
    ),
  );
}

ScrollPosition _position(WidgetTester tester) =>
    tester.state<ScrollableState>(find.byType(Scrollable).first).position;

void main() {
  group('a list of the tracks of a query', () {
    // Pages after the first wait for the test to let them through.
    late Map<int, Completer<List<InternalMediaFile>>> waiting;
    late List<int> asked;

    setUp(() {
      waiting = {};
      asked = [];
    });

    Future<void> pumpList(WidgetTester tester, DeviceType device) => _pump(
      tester,
      device: device,
      (layout) => QueryTrackListView(
        layoutManager: layout,
        queries: const QueryList([('lib::artist', '1')]),
        mode: null,
        contentChanged: const Stream.empty(),
        loadMixPage: (queries, cursor, size) {
          asked.add(cursor);
          if (cursor == 0) return Future.value(_tracks(0, size));
          return (waiting[cursor] ??= Completer()).future;
        },
      ),
    );

    for (final device in [DeviceType.tv, DeviceType.zune, DeviceType.dock]) {
      testWidgets(
        'leaves tracks that have not loaded blank on ${device.name}',
        (tester) async {
          await pumpList(tester, device);
          await _settle(tester);

          // The second page is asked for and has not come.
          expect(asked, [0, 20]);
          expect(find.byType(ProgressRing), findsNothing);
        },
      );
    }

    testWidgets('shows the ring in the middle until its first tracks come', (
      tester,
    ) async {
      final first = Completer<List<InternalMediaFile>>();
      await _pump(
        tester,
        (layout) => QueryTrackListView(
          layoutManager: layout,
          queries: const QueryList([('lib::artist', '1')]),
          mode: null,
          contentChanged: const Stream.empty(),
          loadMixPage: (queries, cursor, size) => first.future,
        ),
      );
      await tester.pump();
      expect(find.byType(ProgressRing), findsOneWidget);

      first.complete(_tracks(0, 5));
      await _settle(tester);
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.text('track-0'), findsOneWidget);
    });

    testWidgets('asks for the pages ahead of the tracks in view', (
      tester,
    ) async {
      // A small list shows about a dozen tracks of a page of twenty.
      await pumpList(tester, DeviceType.zune);
      await _settle(tester);
      expect(asked, [0, 20]);

      // Each page that comes lets the next be asked for, up to three pages
      // ahead, without a scroll.
      waiting[20]!.complete(_tracks(20, 20));
      await _settle(tester);
      waiting[40]!.complete(_tracks(40, 20));
      await _settle(tester);
      waiting[60]!.complete(_tracks(60, 20));
      await _settle(tester);
      expect(asked, [0, 20, 40, 60]);
      // The list is at its start, three pages from the end of what has come.
      final position = _position(tester);
      expect(position.pixels, 0);
      expect(position.extentAfter, greaterThan(position.viewportDimension));

      // A scroll that stays far from that end asks for the page after it.
      position.jumpTo(400);
      await _settle(tester);
      expect(asked, [0, 20, 40, 60, 80]);
      expect(position.extentAfter, greaterThan(position.viewportDimension));
    });
  });

  group('the list of all tracks', () {
    testWidgets('asks for the page after the one in view, and leaves what '
        'has not loaded blank', (tester) async {
      final asked = <int>[];
      final count = Completer<int>();
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => count.future,
          loadPage: (cursor, size) {
            asked.add(cursor);
            if (cursor == 0) return Future.value(_tracks(0, size));
            return Completer<List<InternalMediaFile>>().future;
          },
        ),
        device: DeviceType.zune,
      );
      await tester.pump();
      // The ring in the middle shows until the list knows its length.
      expect(find.byType(ProgressRing), findsOneWidget);

      count.complete(1000);
      await _settle(tester);
      expect(find.text('track-0'), findsOneWidget);
      expect(asked, [0, 100]);
      expect(find.byType(ProgressRing), findsNothing);

      // Far down the list nothing has loaded, and nothing is drawn there.
      _position(tester).jumpTo(64.0 * 500);
      await _settle(tester);
      expect(asked, containsAll([500, 600]));
      expect(find.byType(ProgressRing), findsNothing);
    });
  });

  group('the list of all tracks before its first page', () {
    testWidgets('keeps the ring until the first page comes', (tester) async {
      final first = Completer<List<InternalMediaFile>>();
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.value(1000),
          loadPage: (cursor, size) => first.future,
        ),
        device: DeviceType.zune,
      );
      await _settle(tester);
      expect(find.byType(ProgressRing), findsOneWidget);

      first.complete(_tracks(0, 100));
      await _settle(tester);
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.text('track-0'), findsOneWidget);
    });
    testWidgets('leaves the ring once an empty first page has come after a '
        'failed count', (tester) async {
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.error(StateError('offline')),
          loadPage: (cursor, size) => Future.value(<InternalMediaFile>[]),
        ),
        device: DeviceType.zune,
        providers: _emptyStateProviders(),
      );
      await _settle(tester);
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.byType(NoItems), findsOneWidget);

      // Lets the animation timers of the pages that came end.
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 5));
    });

    testWidgets('shows the ring again after a restart until the new first '
        'page settles', (tester) async {
      var round = 0;
      final count = Completer<int>();
      final second = Completer<List<InternalMediaFile>>();
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () {
            round++;
            return round == 1 ? Future.value(5) : count.future;
          },
          loadPage: (cursor, size) {
            if (round == 1) return Future.value(<InternalMediaFile>[]);
            return second.future;
          },
        ),
        device: DeviceType.zune,
        providers: _emptyStateProviders(),
      );
      await _settle(tester);

      // The content change arrives the way Rust sends it.
      assignRustSignal['LibraryContentChanged']!(
        const LibraryContentChanged(path: '').bincodeSerialize(),
        Uint8List(0),
      );
      await tester.pump();
      await tester.pump();
      // The count of the restart has not come, and nothing is on screen.
      expect(find.byType(ProgressRing), findsOneWidget);

      count.complete(1000);
      await _settle(tester);
      expect(find.byType(ProgressRing), findsOneWidget);

      second.complete(_tracks(0, 100));
      await _settle(tester);
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.text('track-0'), findsOneWidget);
    });
  });

  group('the list of all tracks when a page fails', () {
    testWidgets('does not ask for it again until the list restarts', (
      tester,
    ) async {
      final asked = <int>[];
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.value(1000),
          loadPage: (cursor, size) {
            asked.add(cursor);
            if (cursor == 0) return Future.value(_tracks(0, size));
            return Future.error(StateError('offline'));
          },
        ),
        device: DeviceType.zune,
      );
      await _settle(tester);
      expect(find.text('track-0'), findsOneWidget);

      // The page ahead failed once, and the rebuilds after it leave it be.
      expect(asked, [0, 100]);
      await _settle(tester);
      expect(asked, [0, 100]);
    });

    testWidgets('asks for it once more when its own places are built, and '
        'not again while it keeps failing', (tester) async {
      final asked = <int>[];
      var failures = 1;
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.value(1000),
          loadPage: (cursor, size) {
            asked.add(cursor);
            if (cursor == 100 && failures > 0) {
              failures--;
              return Future.error(StateError('offline'));
            }
            return Future.value(_tracks(cursor, size));
          },
        ),
        device: DeviceType.zune,
      );
      await _settle(tester);
      expect(asked, [0, 100]);

      // Its places come into view after the connection is back.
      _position(tester).jumpTo(64.0 * 100);
      await _settle(tester);
      expect(asked.where((c) => c == 100), hasLength(2));
      expect(find.textContaining('track-1'), findsWidgets);
    });

    testWidgets('is asked for only once more while it keeps failing', (
      tester,
    ) async {
      final asked = <int>[];
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.value(1000),
          loadPage: (cursor, size) {
            asked.add(cursor);
            if (cursor == 0) return Future.value(_tracks(0, size));
            return Future.error(StateError('offline'));
          },
        ),
        device: DeviceType.zune,
      );
      await _settle(tester);
      _position(tester).jumpTo(64.0 * 100);
      await _settle(tester);
      await _settle(tester);
      expect(asked.where((c) => c == 100), hasLength(2));
    });
  });

  group('the list of all tracks when the count fails', () {
    testWidgets('takes the end of the list from the short page', (
      tester,
    ) async {
      final asked = <int>[];
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.error(StateError('offline')),
          loadPage: (cursor, size) {
            asked.add(cursor);
            return Future.value(cursor == 0 ? _tracks(0, size) : []);
          },
        ),
        device: DeviceType.zune,
      );
      await _settle(tester);
      await _settle(tester);
      // The empty page ahead is asked for once, not on every rebuild.
      expect(asked, [0, 100]);

      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 5));
    });
  });

  group('the list of all tracks when the count fails and the first page is '
      'empty', () {
    testWidgets('ends at zero and asks for the first page once', (
      tester,
    ) async {
      final asked = <int>[];
      await _pump(
        tester,
        (layout) => TrackListView(
          layoutManager: layout,
          loadCount: () => Future.error(StateError('offline')),
          loadPage: (cursor, size) {
            asked.add(cursor);
            return Future.value(<InternalMediaFile>[]);
          },
        ),
        device: DeviceType.zune,
        providers: _emptyStateProviders(),
      );
      await _settle(tester);
      await _settle(tester);
      expect(asked, [0]);
      expect(find.byType(ProgressRing), findsNothing);
      expect(find.byType(NoItems), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 5));
    });
  });

  group('a collection page', () {
    late Map<int, Completer<(List<Group<InternalCollection>>, bool)>> waiting;
    late List<int> asked;

    setUp(() {
      CollectionCache().clearAll();
      waiting = {};
      asked = [];
    });

    Future<CollectionDataProvider> pumpPage(
      WidgetTester tester, {
      Future<List<Group<InternalCollection>>>? summary,
    }) async {
      final data = CollectionDataProvider(
        collectionType: CollectionType.album,
        contentChanged: const Stream.empty(),
        loadSummary: (_) => summary ?? Future.value([]),
        loadPage: (type, size, cursor) {
          asked.add(cursor);
          final wait = waiting[cursor];
          if (wait != null) return wait.future;
          return Future.value((_groups(cursor), false));
        },
      );
      addTearDown(data.dispose);
      await _pump(
        tester,
        (layout) => const TurntileScreen(),
        device: DeviceType.zune,
        providers: [
          ChangeNotifierProvider<CollectionDataProvider>.value(value: data),
        ],
      );
      return data;
    }

    testWidgets('shows the ring in the middle until its first data comes', (
      tester,
    ) async {
      final summary = Completer<List<Group<InternalCollection>>>();
      await pumpPage(tester, summary: summary.future);
      await tester.pump();
      expect(find.byType(ProgressRing), findsOneWidget);

      waiting[0] = Completer();
      summary.complete([]);
      await _settle(tester);
      // The first page is on its way, and the list has nothing yet.
      expect(asked, [0]);
      expect(find.byType(ProgressRing), findsOneWidget);

      waiting[0]!.complete((_groups(0), false));
      waiting[1] = Completer();
      await _settle(tester);
      expect(find.text('album-0-0-0'), findsOneWidget);
      expect(find.byType(ProgressRing), findsNothing);
    });

    testWidgets('draws nothing at its end while the next page loads', (
      tester,
    ) async {
      waiting[1] = Completer();
      final data = await pumpPage(tester);
      await _settle(tester);
      expect(asked, [0, 1]);
      expect(data.isLoading, isTrue);

      final position = _position(tester);
      position.jumpTo(position.maxScrollExtent);
      await _settle(tester);
      expect(position.extentAfter, 0);
      expect(data.isLoading, isTrue);
      expect(find.byType(ProgressRing), findsNothing);
    });

    testWidgets('asks for the next page before its end comes into view', (
      tester,
    ) async {
      await pumpPage(tester);
      await _settle(tester);

      // Pages are asked for until they reach well beyond the window.
      final position = _position(tester);
      final viewport = position.viewportDimension;
      expect(asked.length, greaterThan(1));
      expect(position.extentAfter, greaterThan(viewport));

      // A scroll that stays far from the end asks for more.
      final before = asked.length;
      position.jumpTo(position.pixels + viewport);
      await _settle(tester);
      expect(asked.length, greaterThan(before));
      expect(position.extentAfter, greaterThan(viewport));
    });
  });
}
