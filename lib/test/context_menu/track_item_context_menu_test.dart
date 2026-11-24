import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';
import 'context_menu_test_harness.dart';

const _trackKey = Key('track');

const _track = FetchParsedMediaFileResponse(
  file: MediaFile(
    id: 12,
    libraryId: 'library-1',
    track: 'track-12',
    path: 'Album/Song.flac',
    artist: 'Artist',
    album: 'Album',
    title: 'Song',
    duration: 180,
    audioAvailability: AudioAvailability.local,
    audioOrigin: AudioOrigin.host,
  ),
  artists: [Artist(id: 3, name: 'Artist')],
  album: Album(id: 4, name: 'Album'),
);

class _TrackRow extends StatefulWidget {
  const _TrackRow({required this.loadMixes});

  final Future<List<Mix>> Function() loadMixes;

  @override
  State<_TrackRow> createState() => _TrackRowState();
}

class _TrackRowState extends State<_TrackRow> {
  final _controller = RouterAwareFlyoutController();
  final _attachKey = GlobalKey();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: _attachKey,
      contextController: _controller,
      onContextMenu: (position) => openTrackItemContextMenu(
        position,
        context,
        _attachKey,
        _controller,
        null,
        _track.file.id,
        null,
        null,
        loadPlaylists: () async => const [
          Playlist(id: 7, name: 'Road Trip', group: ''),
        ],
        loadMixes: widget.loadMixes,
        loadMediaFile: (fileId) async {
          expect(fileId, _track.file.id);
          return _track;
        },
      ),
      onMiddleClick: (_) {},
      child: const SizedBox(
        key: _trackKey,
        width: 120,
        height: 40,
        child: ColoredBox(color: Color(0xFF000000)),
      ),
    );
  }
}

void main() {
  testWidgets('a track of a library without mixes opens its menu', (
    tester,
  ) async {
    var mixLoads = 0;
    await openContextMenu(
      tester,
      record: subsonicRecord,
      capabilities: NoMixCapabilities(),
      target: _TrackRow(
        loadMixes: () async {
          mixLoads += 1;
          throw mixesUnsupported;
        },
      ),
      targetKey: _trackKey,
    );

    expect(find.byType(ContentDialog), findsNothing);
    expect(find.byType(MenuFlyout), findsOneWidget);
    expect(find.text('Start Playing'), findsOneWidget);
    expect(find.text('Add to Playlist'), findsOneWidget);
    expect(find.text('Add to Mix'), findsNothing);
    expect(mixLoads, 0);
  });

  testWidgets('a track of a library that keeps mixes offers them', (
    tester,
  ) async {
    var mixLoads = 0;
    await openContextMenu(
      tester,
      record: folderRecord,
      capabilities: null,
      target: _TrackRow(
        loadMixes: () async {
          mixLoads += 1;
          return const [
            Mix(id: 5, name: 'Evening', group: '', locked: false, mode: null),
          ];
        },
      ),
      targetKey: _trackKey,
    );

    expect(find.byType(MenuFlyout), findsOneWidget);
    expect(find.text('Add to Playlist'), findsOneWidget);
    expect(find.text('Add to Mix'), findsOneWidget);
    expect(mixLoads, 1);

    await tester.tap(find.text('Add to Mix'));
    await tester.pumpAndSettle();
    expect(find.text('Evening'), findsOneWidget);
  });
}
