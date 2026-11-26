import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../providers/request_capabilities.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../widgets/capability_gate_test.dart' show snapshot;
import 'context_menu_test_harness.dart';

const _roaming = 'Start Roaming';
const _playOn = 'Play On';
const _playlistFile = 'Playlist File';

const _targetKey = Key('target');

/// How many times a track menu asked for the playlists.
var _playlistLoads = 0;

const _track = FetchParsedMediaFileResponse(
  file: MediaFile(
    id: 12,
    libraryId: 'library-1',
    track: 'track-12',
    path: '',
    artist: 'Artist',
    album: 'Album',
    title: 'Song',
    duration: 180,
    audioAvailability: AudioAvailability.remoteOnly,
    audioOrigin: AudioOrigin.remote,
  ),
  artists: [Artist(id: 3, name: 'Artist')],
  album: Album(id: 4, name: 'Album'),
);

/// The capabilities of [record] as its library declares them: with [library]'s
/// answers, and a snapshot of the record open on this device unless [recordId]
/// names another one.
RequestCapabilitiesProvider _declared(
  LibraryRecordSummary record,
  RequestCapabilities library, {
  String? recordId,
}) => RequestCapabilitiesProvider(
  changes: const Stream.empty(),
  initial: library.copyWith(recordId: () => recordId ?? record.recordId),
);

/// What a Subsonic library declares: no analysis, no files, no mixes of its
/// own, and another device can still play it.
final _subsonic = snapshot().copyWith(
  getAnalyzeCountRequest: const RequestCapabilityHidden(),
  fetchDirectoryTreeRequest: const RequestCapabilityHidden(),
  fetchAllMixesRequest: const RequestCapabilityHidden(),
  addItemToMixRequest: const RequestCapabilityHidden(),
);

/// What a library of files declares: every control works.
final _files = snapshot();

/// A library that another device cannot play.
final _keptHere = _subsonic.copyWith(
  castResolvedItemsRequest: const RequestCapabilityHidden(),
);

const _remoteControl = LibraryRecordSummary(
  shareSync: false,
  recordId: 'remote-record',
  alias: 'Living Room',
  kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
);

/// A target that opens its menu with the real menu code, as a list row does.
class _MenuTarget extends StatefulWidget {
  const _MenuTarget.collection() : track = false;
  const _MenuTarget.track() : track = true;

  final bool track;

  @override
  State<_MenuTarget> createState() => _MenuTargetState();
}

class _MenuTargetState extends State<_MenuTarget> {
  final _controller = RouterAwareFlyoutController();
  final _attachKey = GlobalKey();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _open(Offset position) {
    if (widget.track) {
      openTrackItemContextMenu(
        position,
        context,
        _attachKey,
        _controller,
        null,
        _track.file.id,
        null,
        null,
        loadPlaylists: () async {
          _playlistLoads++;
          return const [];
        },
        loadMixes: () async => const [],
        loadMediaFile: (_) async => _track,
        fetchLikedStatus: (_) async => false,
      );
    } else {
      openCollectionItemContextMenu(
        position,
        context,
        _attachKey,
        _controller,
        CollectionType.album,
        4,
        'Album',
        null,
        true,
        () async => const [],
        () async {
          _playlistLoads++;
          return const [];
        },
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: _attachKey,
      contextController: _controller,
      onContextMenu: _open,
      onMiddleClick: (_) {},
      child: const SizedBox(
        key: _targetKey,
        width: 120,
        height: 40,
        child: ColoredBox(color: Color(0xFF000000)),
      ),
    );
  }
}

/// Builds the menus of a band screen in the harness's window, whose providers
/// name the open library, and keeps the labels of their buttons in [entries].
class _BandMenus extends StatelessWidget {
  const _BandMenus(this.entries);

  final Set<String> entries;

  @override
  Widget build(BuildContext context) {
    final menus = [
      buildBandScreenCollectionItemContextMenu(
        context,
        CollectionType.album,
        4,
        true,
      ),
      buildBandScreenTrackItemContextMenu(context, _track, true),
    ];
    entries
      ..clear()
      ..addAll({
        for (final menu in menus)
          for (final button
              in ((menu.child as Container).child as CommandBar).primaryItems
                  .whereType<CommandBarButton>())
            if (button.icon case Tooltip(:final message?)) message,
      });
    return const SizedBox(key: _targetKey, width: 120, height: 40);
  }
}

/// The entries the tests look for that the open menus show.
Set<String> _shown() => {
  for (final label in [
    'Start Playing',
    _roaming,
    _playOn,
    _playlistFile,
    'Cover Art Wall',
  ])
    if (find.text(label).evaluate().isNotEmpty) label,
};

/// The entries of the collection and track menus of a band screen.
Future<Set<String>> _bandEntries(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
}) async {
  final entries = <String>{};
  await openContextMenu(
    tester,
    record: record,
    capabilities: capabilities,
    target: _BandMenus(entries),
    targetKey: _targetKey,
  );
  return entries;
}

/// Opens the large menu of a collection or a track of [record].
Future<void> _openMenu(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
  required _MenuTarget target,
}) async {
  await openContextMenu(
    tester,
    record: record,
    capabilities: capabilities,
    target: target,
    targetKey: _targetKey,
  );
  expect(find.byType(ContentDialog), findsNothing);
  expect(find.byType(MenuFlyout), findsOneWidget);
}

/// Closes the open menus, so the next window of the test starts without them.
Future<void> _closeMenus(WidgetTester tester) async {
  await tester.tapAt(Offset.zero);
  await tester.pumpAndSettle();
  expect(find.byType(MenuFlyout), findsNothing);
}

/// The entries of the large menu of a collection or a track of [record].
Future<Set<String>> _menuEntries(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
  required _MenuTarget target,
}) async {
  await _openMenu(
    tester,
    record: record,
    capabilities: capabilities,
    target: target,
  );
  final shown = _shown();
  await _closeMenus(tester);
  return shown;
}

/// The entries of the export submenu of a collection of [record].
Future<Set<String>> _exportEntries(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
}) async {
  await _openMenu(
    tester,
    record: record,
    capabilities: capabilities,
    target: const _MenuTarget.collection(),
  );
  await tester.tap(find.text('Export As'));
  await tester.pumpAndSettle();
  final shown = _shown();
  await _closeMenus(tester);
  return shown;
}

final _desktop = Platform.isMacOS || Platform.isWindows || Platform.isLinux;

void main() {
  group('a Subsonic library', () {
    testWidgets('leaves roaming out of a collection menu and keeps Play On', (
      tester,
    ) async {
      final entries = await _menuEntries(
        tester,
        record: subsonicRecord,
        capabilities: _declared(subsonicRecord, _subsonic),
        target: const _MenuTarget.collection(),
      );
      expect(entries, containsAll(['Start Playing', _playOn]));
      expect(entries, isNot(contains(_roaming)));
    });

    testWidgets('leaves the playlist file out of the export menu', (
      tester,
    ) async {
      final entries = await _exportEntries(
        tester,
        record: subsonicRecord,
        capabilities: _declared(subsonicRecord, _subsonic),
      );
      expect(entries, isNot(contains(_playlistFile)));
      // The cover wall needs no files.
      expect(entries, contains('Cover Art Wall'));
    }, skip: !_desktop);

    testWidgets('leaves roaming out of a track menu and keeps Play On', (
      tester,
    ) async {
      final entries = await _menuEntries(
        tester,
        record: subsonicRecord,
        capabilities: _declared(subsonicRecord, _subsonic),
        target: const _MenuTarget.track(),
      );
      expect(entries, containsAll(['Start Playing', _playOn]));
      expect(entries, isNot(contains(_roaming)));
    });

    testWidgets('leaves roaming out of the band menus and keeps Play On', (
      tester,
    ) async {
      final entries = await _bandEntries(
        tester,
        record: subsonicRecord,
        capabilities: _declared(subsonicRecord, _subsonic),
      );
      expect(entries, containsAll(['Start Playing', _playOn]));
      expect(entries, isNot(contains(_roaming)));
    });
  });

  group('a library of files', () {
    testWidgets('keeps roaming and Play On in every menu', (tester) async {
      for (final target in const [
        _MenuTarget.collection(),
        _MenuTarget.track(),
      ]) {
        expect(
          await _menuEntries(
            tester,
            record: folderRecord,
            capabilities: _declared(folderRecord, _files),
            target: target,
          ),
          containsAll([_roaming, _playOn]),
        );
      }
      expect(
        await _bandEntries(
          tester,
          record: folderRecord,
          capabilities: _declared(folderRecord, _files),
        ),
        containsAll([_roaming, _playOn]),
      );
    });

    testWidgets('keeps the playlist file without a capability provider', (
      tester,
    ) async {
      expect(
        await _exportEntries(tester, record: folderRecord),
        contains(_playlistFile),
      );
    }, skip: !_desktop);
  });

  group('a snapshot', () {
    testWidgets('of another record is ignored while playing on a renderer', (
      tester,
    ) async {
      // The renderer has a library that keeps no files open.
      final renderer = _declared(
        folderRecord,
        _keptHere,
        recordId: 'renderer-record',
      );
      expect(
        await _menuEntries(
          tester,
          record: folderRecord,
          capabilities: renderer,
          target: const _MenuTarget.collection(),
        ),
        containsAll([_roaming, _playOn]),
      );
    });

    testWidgets('of another record leaves the open library its own answers', (
      tester,
    ) async {
      // The open library's snapshot came first, then the one of the library of
      // files on the renderer this device plays on.
      RequestCapabilitiesProvider renderer() => RequestCapabilitiesProvider(
        changes: Stream.value(
          _files.copyWith(recordId: () => 'renderer-record'),
        ),
        initial: _subsonic.copyWith(recordId: () => subsonicRecord.recordId),
      );
      final entries = await _menuEntries(
        tester,
        record: subsonicRecord,
        capabilities: renderer(),
        target: const _MenuTarget.collection(),
      );
      expect(entries, contains(_playOn));
      expect(entries, isNot(contains(_roaming)));
      if (!_desktop) return;
      expect(
        await _exportEntries(
          tester,
          record: subsonicRecord,
          capabilities: renderer(),
        ),
        isNot(contains(_playlistFile)),
      );
    });

    testWidgets(
      'of another record leaves the menus the open library\'s playlists',
      (tester) async {
        // The open library lists playlists, and the renderer this device plays
        // on sends a snapshot of a library that lists none.
        RequestCapabilitiesProvider renderer() => RequestCapabilitiesProvider(
          changes: Stream.value(
            _files.copyWith(
              recordId: () => 'renderer-record',
              fetchAllPlaylistsRequest: const RequestCapabilityHidden(),
            ),
          ),
          initial: _files.copyWith(recordId: () => folderRecord.recordId),
        );
        for (final target in const [
          _MenuTarget.track(),
          _MenuTarget.collection(),
        ]) {
          _playlistLoads = 0;
          await _menuEntries(
            tester,
            record: folderRecord,
            capabilities: renderer(),
            target: target,
          );
          expect(_playlistLoads, 1);
        }
      },
    );

    testWidgets('of the controlled library counts while remote controlling', (
      tester,
    ) async {
      Future<Set<String>> entries(RequestCapabilities library) => _menuEntries(
        tester,
        record: _remoteControl,
        capabilities: _declared(
          _remoteControl,
          library,
          recordId: 'owner-record',
        ),
        target: const _MenuTarget.collection(),
      );
      expect(await entries(_subsonic), isNot(contains(_roaming)));
      expect(await entries(_files), contains(_roaming));
    });

    testWidgets(
      'that no other device can play the library leaves out Play On',
      (tester) async {
        final large = await _menuEntries(
          tester,
          record: subsonicRecord,
          capabilities: _declared(subsonicRecord, _keptHere),
          target: const _MenuTarget.track(),
        );
        expect(large, contains('Start Playing'));
        expect(large, isNot(contains(_playOn)));
        final band = await _bandEntries(
          tester,
          record: subsonicRecord,
          capabilities: _declared(subsonicRecord, _keptHere),
        );
        expect(band, isNot(contains(_playOn)));
      },
    );
  });
}
