import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_open_state.dart';
import '../../providers/playing_item_capabilities.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/dialogs/add_to_playlist/add_to_playlist_dialog.dart';
import '../../utils/playing_item.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../widgets/capability_gate_test.dart' show snapshot;

const like = 'Liked';
const addToPlaylist = 'Add to Playlist';
const addToMix = 'Add to Mix';

const parsed = FetchParsedMediaFileResponse(
  file: MediaFile(
    id: 7,
    libraryId: 'library',
    track: 't',
    path: '',
    artist: 'Artist One',
    album: 'Dawn Album',
    title: 'First Light',
    duration: 200,
    audioAvailability: AudioAvailability.local,
    audioOrigin: AudioOrigin.host,
  ),
  artists: [Artist(id: 1, name: 'Artist One')],
  album: Album(id: 2, name: 'Dawn Album'),
);

/// The track of [parsed] as the item its like is kept for.
const track = PlayingItem.subsonicTrack(libraryId: 'library', trackId: 't');

const roadTrip = Playlist(id: 3, name: 'Road Trip', group: 'Mood');
const nightDrive = Playlist(id: 4, name: 'Night Drive', group: 'Mood');

const morning = Mix(id: 5, name: 'Morning', group: 'Mood', locked: false);
const evening = Mix(id: 6, name: 'Evening', group: 'Mood', locked: false);
const recommended = Mix(id: 8, name: 'For You', group: 'Rune', locked: true);

/// A replica the owner shares for reading only.
const readOnlyReplica = LibraryRecordSummary(
  shareSync: false,
  recordId: 'record',
  alias: 'Library',
  kind: LibraryRecordKindSummaryReplica(
    libraryId: 'library',
    ownerFingerprint: 'owner',
  ),
  syncAccess: LibraryAccessSummary.read,
);

/// Shows [home] in a desktop window of a library with [capabilities].
Future<void> pumpIn(
  WidgetTester tester,
  Widget home, {
  RequestCapabilities? capabilities,
  PlayingItemCapabilities? playing,
  bool replica = false,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(1920, 1080);
  addTearDown(tester.view.reset);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final library = RequestCapabilitiesProvider(
    changes: const Stream.empty(),
    initial: capabilities ?? snapshot(),
  );
  addTearDown(library.dispose);
  final item = PlayingItemCapabilitiesProvider(
    changes: const Stream.empty(),
    initial: playing,
  );
  addTearDown(item.dispose);
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
        ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
          value: library,
        ),
        ChangeNotifierProvider<PlayingItemCapabilitiesProvider>.value(
          value: item,
        ),
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _Library(replica: replica),
        ),
      ],
      child: FluentApp(
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: home,
      ),
    ),
  );
}

/// An item whose menu [open] shows, right-clicked.
Future<void> rightClick(
  WidgetTester tester,
  void Function(
    Offset position,
    BuildContext context,
    GlobalKey attachKey,
    RouterAwareFlyoutController controller,
  )
  open, {
  RequestCapabilities? capabilities,
  PlayingItemCapabilities? playing,
  bool replica = false,
}) async {
  final controller = RouterAwareFlyoutController();
  addTearDown(controller.dispose);
  final attachKey = GlobalKey();
  await pumpIn(
    tester,
    Builder(
      builder: (context) => Center(
        child: ContextMenuWrapper(
          contextAttachKey: attachKey,
          contextController: controller,
          onContextMenu: (position) =>
              open(position, context, attachKey, controller),
          onMiddleClick: (_) {},
          child: const SizedBox(
            key: Key('item'),
            width: 120,
            height: 40,
            child: ColoredBox(color: Color(0xFF000000)),
          ),
        ),
      ),
    ),
    capabilities: capabilities,
    playing: playing,
    replica: replica,
  );
  final gesture = await tester.startGesture(
    tester.getCenter(find.byKey(const Key('item'))),
    kind: PointerDeviceKind.mouse,
    buttons: kSecondaryMouseButton,
  );
  await gesture.up();
  await tester.pumpAndSettle();
}

/// What a track menu asked of the likes.
class Likes {
  Likes(this.liked);

  bool liked;
  final read = <PlayingItem>[];
  final written = <(PlayingItem, bool)>[];
}

/// Opens the menu of the track [parsed].
Future<Likes> openTrackMenu(
  WidgetTester tester, {
  bool liked = false,
  RequestCapabilities? capabilities,
  PlayingItemCapabilities? playing,
  bool replica = false,
}) async {
  final likes = Likes(liked);
  await rightClick(
    tester,
    (position, context, attachKey, controller) => openTrackItemContextMenu(
      position,
      context,
      attachKey,
      controller,
      null,
      parsed.file.id,
      null,
      null,
      loadPlaylists: () async => const [roadTrip],
      loadMixes: () async => const [morning],
      loadMediaFile: (_) async => parsed,
      fetchLikedStatus: (item) async {
        likes.read.add(item);
        return likes.liked;
      },
      setLikedStatus: (item, liked) async {
        likes.written.add((item, liked));
        return likes.liked = liked;
      },
    ),
    capabilities: capabilities,
    playing: playing,
    replica: replica,
  );
  return likes;
}

/// What the adds of a collection menu sent.
class Adds {
  final toPlaylist = <(int, int)>[];
  final toMix = <(int, String, String)>[];
}

/// Opens the menu of the collection [type] 1, whose songs are 11, 12 and 13
/// and, for a mix, whose queries are an artist and an album. The other
/// playlists are empty.
Future<Adds> openCollectionMenu(
  WidgetTester tester,
  CollectionType type, {
  int id = 1,
  bool replica = false,
}) async {
  final adds = Adds();
  await rightClick(tester, (position, context, attachKey, controller) {
    final box = attachKey.currentContext!.findRenderObject() as RenderBox;
    controller.showFlyout(
      position: box.localToGlobal(
        position,
        ancestor: Navigator.of(context).context.findRenderObject(),
      ),
      builder: (context) => buildLargeScreenCollectionItemContextMenu(
        context,
        type,
        id,
        'Collection',
        const [morning, evening, recommended],
        null,
        null,
        const [],
        const [roadTrip, nightDrive],
        (type, id) async => type == CollectionType.playlist && id != 1
            ? const []
            : const [11, 12, 13],
        (playlistId, fileId) async => adds.toPlaylist.add((playlistId, fileId)),
        (type, id) async => type == CollectionType.mix
            ? const [('lib::artist', '21'), ('lib::album', '22')]
            : [('lib::${type.name}', '$id')],
        (mixId, operator, parameter) async =>
            adds.toMix.add((mixId, operator, parameter)),
      ),
    );
  }, replica: replica);
  return adds;
}

/// Opens the submenu [label] of an open menu.
Future<void> openSubmenu(WidgetTester tester, String label) async {
  await tester.tap(find.text(label));
  await tester.pumpAndSettle();
}

/// The heart of the Like entry of an open menu.
Icon heart(WidgetTester tester) =>
    tester.widget<Icon>(find.byIcon(Symbols.favorite));

void main() {
  group('Like on a track', () {
    testWidgets('shows the heart of the playback bar, empty until liked', (
      tester,
    ) async {
      final likes = await openTrackMenu(tester);

      expect(find.text(like), findsOneWidget);
      expect(heart(tester).fill, 0);
      expect(likes.read, [track]);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('is filled for a liked track, and takes the like back', (
      tester,
    ) async {
      final likes = await openTrackMenu(tester, liked: true);

      expect(heart(tester).fill, 1);
      await tester.tap(find.text(like));
      await tester.pumpAndSettle();

      expect(likes.written, [(track, false)]);
      expect(find.byType(MenuFlyout), findsNothing);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('likes a track that is not liked', (tester) async {
      final likes = await openTrackMenu(tester);

      await tester.tap(find.text(like));
      await tester.pumpAndSettle();

      expect(likes.written, [(track, true)]);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('is left out where the open library sets no likes', (
      tester,
    ) async {
      final likes = await openTrackMenu(
        tester,
        capabilities: snapshot(setLiked: const RequestCapabilityHidden()),
      );

      expect(find.byType(MenuFlyout), findsOneWidget);
      expect(find.text(like), findsNothing);
      expect(likes.read, isEmpty);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('is left out where the library of the track sets none', (
      tester,
    ) async {
      final likes = await openTrackMenu(
        tester,
        playing: PlayingItemCapabilities(
          item: track.toRequest(),
          setLikedRequest: const RequestCapabilityDisabled(
            reason: CapabilityReason.notImplemented,
          ),
          getLikedRequest: const RequestCapabilityAvailable(),
          getLyricByTrackIdRequest: const RequestCapabilityAvailable(),
          getPrimaryColorByTrackIdRequest: const RequestCapabilityAvailable(),
        ),
      );

      expect(find.byType(MenuFlyout), findsOneWidget);
      expect(find.text(like), findsNothing);
      expect(likes.read, isEmpty);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('shows no like where the library of the track reads none', (
      tester,
    ) async {
      final likes = await openTrackMenu(
        tester,
        liked: true,
        playing: PlayingItemCapabilities(
          item: track.toRequest(),
          setLikedRequest: const RequestCapabilityAvailable(),
          getLikedRequest: const RequestCapabilityHidden(),
          getLyricByTrackIdRequest: const RequestCapabilityAvailable(),
          getPrimaryColorByTrackIdRequest: const RequestCapabilityAvailable(),
        ),
      );

      expect(heart(tester).fill, 0);
      expect(likes.read, isEmpty);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('is a button of the band menu only where it is offered', (
      tester,
    ) async {
      late List<String> offered;
      late List<String> withheld;
      await pumpIn(
        tester,
        Builder(
          builder: (context) {
            offered = _band(
              buildBandScreenTrackItemContextMenu(
                context,
                parsed,
                true,
                liked: false,
              ),
            );
            withheld = _band(
              buildBandScreenTrackItemContextMenu(context, parsed, true),
            );
            return const SizedBox.shrink();
          },
        ),
      );

      expect(offered, contains(like));
      expect(withheld, isNot(contains(like)));
      await tester.pump(const Duration(seconds: 1));
    });
  });

  for (final failing in ['playlists', 'mixes', 'file']) {
    testWidgets('a track menu whose $failing fail to load shows the failure', (
      tester,
    ) async {
      const failure = RequestFailure(
        'Load',
        RequestErrorKind.unavailable,
        'unavailable',
      );
      await rightClick(
        tester,
        (position, context, attachKey, controller) => openTrackItemContextMenu(
          position,
          context,
          attachKey,
          controller,
          null,
          parsed.file.id,
          null,
          null,
          loadPlaylists: () async =>
              failing == 'playlists' ? throw failure : const [roadTrip],
          loadMixes: () async =>
              failing == 'mixes' ? throw failure : const [morning],
          loadMediaFile: (_) async =>
              failing == 'file' ? throw failure : parsed,
          fetchLikedStatus: (_) async => false,
        ),
      );

      expect(find.byType(MenuFlyout), findsNothing);
      expect(find.byType(ContentDialog), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    });
  }

  group('Add to Playlist on a collection', () {
    for (final type in [
      CollectionType.album,
      CollectionType.artist,
      CollectionType.playlist,
      CollectionType.mix,
    ]) {
      testWidgets('adds every song of ${type.name} 1, in order', (
        tester,
      ) async {
        final adds = await openCollectionMenu(tester, type);

        await openSubmenu(tester, addToPlaylist);
        expect(find.text('New Playlist'), findsOneWidget);
        await tester.tap(find.text(roadTrip.name));
        await tester.pumpAndSettle();

        expect(adds.toPlaylist, [(3, 11), (3, 12), (3, 13)]);
        // The dialog that showed the progress closed by itself.
        expect(find.byType(AddToPlaylistDialog), findsNothing);
        await tester.pump(const Duration(seconds: 1));
      });
    }

    testWidgets('does not offer a playlist itself', (tester) async {
      await openCollectionMenu(
        tester,
        CollectionType.playlist,
        id: roadTrip.id,
      );

      await openSubmenu(tester, addToPlaylist);
      expect(find.text(roadTrip.name), findsNothing);
      expect(find.text(nightDrive.name), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    });
  });

  group('Add to Mix on a mix', () {
    testWidgets('copies each query of the mix', (tester) async {
      final adds = await openCollectionMenu(tester, CollectionType.mix);

      await openSubmenu(tester, addToMix);
      expect(find.text('New Mix'), findsOneWidget);
      await tester.tap(find.text(morning.name));
      await tester.pumpAndSettle();

      expect(adds.toMix, [(5, 'lib::artist', '21'), (5, 'lib::album', '22')]);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('offers neither the mix itself nor a locked mix', (
      tester,
    ) async {
      await openCollectionMenu(tester, CollectionType.mix, id: morning.id);

      await openSubmenu(tester, addToMix);
      expect(find.text(morning.name), findsNothing);
      expect(find.text(recommended.name), findsNothing);
      expect(find.text(evening.name), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('takes a locked mix as the one to copy', (tester) async {
      final adds = await openCollectionMenu(
        tester,
        CollectionType.mix,
        id: recommended.id,
      );

      await openSubmenu(tester, addToMix);
      await tester.tap(find.text(evening.name));
      await tester.pumpAndSettle();

      expect(adds.toMix, hasLength(2));
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('an album is still added as itself', (tester) async {
      final adds = await openCollectionMenu(tester, CollectionType.album);

      await openSubmenu(tester, addToMix);
      await tester.tap(find.text(morning.name));
      await tester.pumpAndSettle();

      expect(adds.toMix, [(5, 'lib::album', '1')]);
      await tester.pump(const Duration(seconds: 1));
    });
  });

  group('a read-only replica', () {
    testWidgets('has no Like and no adds in the track menu', (tester) async {
      final likes = await openTrackMenu(tester, replica: true);

      expect(find.byType(MenuFlyout), findsOneWidget);
      expect(find.text(like), findsNothing);
      expect(find.text(addToPlaylist), findsNothing);
      expect(find.text(addToMix), findsNothing);
      expect(likes.read, isEmpty);
      await tester.pump(const Duration(seconds: 1));
    });

    for (final type in [CollectionType.album, CollectionType.mix]) {
      testWidgets('has no adds in the menu of ${type.name}', (tester) async {
        await openCollectionMenu(tester, type, replica: true);

        expect(find.byType(MenuFlyout), findsOneWidget);
        expect(find.text(addToPlaylist), findsNothing);
        expect(find.text(addToMix), findsNothing);
        await tester.pump(const Duration(seconds: 1));
      });
    }
  });
}

/// The labels of the buttons of a band menu.
List<String> _band(FlyoutContent content) {
  final bar = (content.child as Container).child as CommandBar;
  return bar.primaryItems
      .whereType<CommandBarButton>()
      .map((button) => (button.icon as Tooltip).message ?? '')
      .toList();
}

class _Library extends ChangeNotifier implements LibraryOpenStateProvider {
  _Library({required this.replica});

  final bool replica;

  @override
  bool get isRemoteControl => false;

  @override
  String? get catalogLibraryId => null;

  @override
  bool get isReadOnlyReplica => replica;

  @override
  LibraryRecordSummary? get openRecord => replica
      ? readOnlyReplica
      : const LibraryRecordSummary(
          shareSync: false,
          recordId: 'record',
          alias: 'Library',
          kind: LibraryRecordKindSummaryLocal(
            libraryId: 'library',
            path: '/music',
          ),
        );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
