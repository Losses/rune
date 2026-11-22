import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/context_menu_wrapper.dart';

const _subsonicRecord = LibraryRecordSummary(
  recordId: 'subsonic-record',
  alias: 'Server',
  kind: LibraryRecordKindSummarySubsonic(
    endpoint: 'http://127.0.0.1:4533',
    account: 'listener',
  ),
);

const _folderRecord = LibraryRecordSummary(
  recordId: 'folder-record',
  alias: 'Music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

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

/// The answer a Subsonic library gives to the mix requests, which it does not
/// implement.
const _mixesUnsupported = RequestFailure(
  'FetchAllMixesRequest',
  RequestErrorKind.unavailable,
  'FetchAllMixesRequest is unsupported by this target: not implemented',
);

/// The capabilities of a library that keeps no mixes, such as a Subsonic one.
class _NoMixCapabilities extends RequestCapabilitiesProvider {
  _NoMixCapabilities() : super(changes: const Stream.empty());

  @override
  RequestCapability get fetchAllMixesRequest => const RequestCapabilityHidden();

  @override
  RequestCapability get addItemToMixRequest => const RequestCapabilityHidden();
}

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
        key: Key('track'),
        width: 120,
        height: 40,
        child: ColoredBox(color: Color(0xFF000000)),
      ),
    );
  }
}

Future<void> _openMenu(
  WidgetTester tester, {
  required LibraryRecordSummary record,
  RequestCapabilitiesProvider? capabilities,
  required Future<List<Mix>> Function() loadMixes,
}) async {
  // A desktop window, so the menu loads its data before it opens.
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(1920, 1080);
  addTearDown(tester.view.reset);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [record],
      invalidRecords: const [],
      open: OpenRecord(
        recordId: record.recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      ),
      lastOpenRecord: null,
    ),
  );
  addTearDown(registry.dispose);
  final openState = LibraryOpenStateProvider(null, registry);
  addTearDown(openState.dispose);
  await openState.initialOpen;
  if (capabilities != null) addTearDown(capabilities.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ChangeNotifierProvider<LibraryOpenStateProvider>.value(
          value: openState,
        ),
        if (capabilities != null)
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: capabilities,
          ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Center(child: _TrackRow(loadMixes: loadMixes)),
      ),
    ),
  );
  // Lets the screen size provider save the window size it read.
  await tester.pump(const Duration(seconds: 1));

  final gesture = await tester.startGesture(
    tester.getCenter(find.byKey(const Key('track'))),
    kind: PointerDeviceKind.mouse,
    buttons: kSecondaryMouseButton,
  );
  await gesture.up();
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a track of a library without mixes opens its menu', (
    tester,
  ) async {
    var mixLoads = 0;
    await _openMenu(
      tester,
      record: _subsonicRecord,
      capabilities: _NoMixCapabilities(),
      loadMixes: () async {
        mixLoads += 1;
        throw _mixesUnsupported;
      },
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
    await _openMenu(
      tester,
      record: _folderRecord,
      capabilities: null,
      loadMixes: () async {
        mixLoads += 1;
        return const [
          Mix(id: 5, name: 'Evening', group: '', locked: false, mode: null),
        ];
      },
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
