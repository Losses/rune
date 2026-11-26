import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/library_open_state.dart';
import '../../providers/status.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../utils/context_menu/offline_selection.dart';
import '../../utils/l10n.dart';
import '../../utils/playing_item.dart';
import '../../widgets/capability_gate.dart';
import '../../widgets/playback_controller/like_button.dart';
import '../../widgets/playback_controller/next_button.dart';
import '../../widgets/playback_controller/play_pause_button.dart';
import '../../widgets/playback_controller/previous_button.dart';

RequestCapabilities snapshot({
  RequestCapability play = const RequestCapabilityAvailable(),
  RequestCapability pause = const RequestCapabilityAvailable(),
  RequestCapability next = const RequestCapabilityAvailable(),
  RequestCapability previous = const RequestCapabilityAvailable(),
  RequestCapability setLiked = const RequestCapabilityAvailable(),
  RequestCapability setDownloadRule = const RequestCapabilityAvailable(),
}) {
  const capability = RequestCapabilityAvailable();
  return RequestCapabilities(
    recordId: null,
    generation: Uint64(BigInt.zero),
    attachRendererRequest: capability,
    detachRendererRequest: capability,
    listRenderersRequest: capability,
    handoffRequest: capability,
    handoffLoadRequest: capability,
    takeBackHandoffBlockRequest: capability,
    getDeviceGrantsRequest: capability,
    setDeviceGrantsRequest: capability,
    listControllersRequest: capability,
    detachControllerRequest: capability,
    listLocalLibrariesRequest: capability,
    listLibraryRecordsRequest: capability,
    removeLibraryRecordRequest: capability,
    syncShareLibraryRequest: capability,
    listPeerLibrariesRequest: capability,
    addRemoteLibraryRequest: capability,
    getSyncSettingsRequest: capability,
    setSyncSettingsRequest: capability,
    setDownloadRuleRequest: capability,
    removeDownloadRuleRequest: capability,
    testLibraryInitializedRequest: capability,
    closeLibraryRequest: capability,
    cancelTaskRequest: capability,
    scanAudioLibraryRequest: capability,
    analyzeAudioLibraryRequest: capability,
    deduplicateAudioLibraryRequest: capability,
    volumeRequest: capability,
    loadRequest: capability,
    playRequest: capability,
    pauseRequest: capability,
    pauseReportingRequest: capability,
    nextRequest: capability,
    previousRequest: capability,
    switchRequest: capability,
    seekRequest: capability,
    removeRequest: capability,
    setPlaybackModeRequest: capability,
    movePlaylistItemRequest: capability,
    setRealtimeFFTEnabledRequest: capability,
    setAdaptiveSwitchingEnabledRequest: capability,
    setMixWithOthersRequest: capability,
    startVisualizerRequest: capability,
    stopVisualizerRequest: capability,
    resizeVisualizerRequest: capability,
    setVisualizerPresetDirsRequest: capability,
    setVisualizerAccentRequest: capability,
    setVisualizerVisibleRequest: capability,
    loadVisualizerPresetRequest: capability,
    nextVisualizerPresetRequest: capability,
    previousVisualizerPresetRequest: capability,
    randomVisualizerPresetRequest: capability,
    importVisualizerSceneRequest: capability,
    removeVisualizerSceneRequest: capability,
    setVisualizerScenesRequest: capability,
    setVisualizerLogRequest: capability,
    setVisualizerLabRequest: capability,
    visualizerLabCommandRequest: capability,
    visualizerLabRequest: capability,
    visualizerPlanRequest: capability,
    visualizerSupportRequest: capability,
    visualizerStatusRequest: capability,
    listVisualizerPresetsRequest: capability,
    listVisualizerCreditsRequest: capability,
    visualizerTextureShownRequest: capability,
    sfxPlayRequest: capability,
    ifAnalyzeExistsRequest: capability,
    getAnalyzeCountRequest: capability,
    fetchMediaFilesRequest: capability,
    fetchMediaFileByIdsRequest: capability,
    fetchParsedMediaFileRequest: capability,
    searchMediaFileSummaryRequest: capability,
    getMediaFilesCountRequest: capability,
    getLyricByTrackIdRequest: capability,
    fetchCollectionGroupSummaryRequest: capability,
    fetchCollectionGroupsRequest: capability,
    fetchCollectionByIdsRequest: capability,
    searchCollectionSummaryRequest: capability,
    getCoverArtIdsByMixQueriesRequest: capability,
    getPrimaryColorByTrackIdRequest: capability,
    fetchAllPlaylistsRequest: capability,
    createPlaylistRequest: capability,
    createM3u8PlaylistRequest: capability,
    updatePlaylistRequest: capability,
    removePlaylistRequest: capability,
    addItemToPlaylistRequest: capability,
    reorderPlaylistItemPositionRequest: capability,
    fetchPlaylistOccurrencesRequest: capability,
    getPlaylistByIdRequest: capability,
    fetchAllMixesRequest: capability,
    createMixRequest: capability,
    updateMixRequest: capability,
    removeMixRequest: capability,
    addItemToMixRequest: capability,
    getMixByIdRequest: capability,
    mixQueryRequest: capability,
    fetchMixQueriesRequest: capability,
    resolveMixQueryRequest: capability,
    loadResolvedItemsRequest: capability,
    castResolvedItemsRequest: capability,
    castPreflightRequest: capability,
    refreshSessionRequest: capability,
    sessionSnapshotRequest: capability,
    playlistRevisionRequest: capability,
    remotePlaylistRevisionRequest: capability,
    setLikedRequest: capability,
    getLikedRequest: capability,
    complexQueryRequest: capability,
    searchForRequest: capability,
    fetchDirectoryTreeRequest: capability,
    authenticateSingleServiceRequest: capability,
    authenticateMultipleServiceRequest: capability,
    logoutSingleServiceRequest: capability,
    listLogRequest: capability,
    clearLogRequest: capability,
    removeLogRequest: capability,
    systemInfoRequest: capability,
    registerLicenseRequest: capability,
    validateLicenseRequest: capability,
    startListeningRequest: capability,
    stopListeningRequest: capability,
    getDiscoveredDeviceRequest: capability,
    listClientsRequest: capability,
    listPairedServersRequest: capability,
    getSslCertificateFingerprintRequest: capability,
    getOpenLibraryRequest: capability,
    addTrustedServerRequest: capability,
    removeTrustedClientRequest: capability,
    updateClientStatusRequest: capability,
    editHostsRequest: capability,
    removeTrustedServerRequest: capability,
    serverAvailabilityTestRequest: capability,
    registerDeviceOnServerRequest: capability,
    checkDeviceOnServerRequest: capability,
    connectRequest: capability,
    fetchServerCertificateRequest: capability,
    fetchRemoteFileRequest: capability,
    removeItemFromPlaylistRequest: capability,
    registerLocalLibraryRequest: capability,
    registerRemoteControlRequest: capability,
    openLibraryRequest: capability,
    registerSubsonicLibraryRequest: capability,
    replaceSubsonicCredentialRequest: capability,
    registerOpenDalLibraryRequest: capability,
    detectOpenDalShareDataRequest: capability,
    initializeNodeRequest: capability,
    runFsSelfTestRequest: capability,
    listFileDialogPlacesRequest: capability,
    resolveFileDialogPathRequest: capability,
    listFileDialogFolderRequest: capability,
    fetchFileDialogThumbnailRequest: capability,
    createFileDialogFolderRequest: capability,
    renameFileDialogEntryRequest: capability,
    fetchArtistPageRequest: capability,
    fetchArtistSectionAlbumsRequest: capability,
    registerServiceLibraryRequest: capability,
    signInServiceLibraryRequest: capability,
    signOutServiceLibraryRequest: capability,
    setOnlineLyricsRequest: capability,
  ).copyWith(
    playRequest: play,
    pauseRequest: pause,
    nextRequest: next,
    previousRequest: previous,
    setLikedRequest: setLiked,
    setDownloadRuleRequest: setDownloadRule,
  );
}

class _LibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  _LibraryOpenStateProvider({this.openRecord});

  @override
  bool get isReadOnlyReplica => false;

  @override
  bool get isRemoteControl => false;

  @override
  final LibraryRecordSummary? openRecord;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Widget playbackControls(
  RequestCapabilitiesProvider provider, {
  PlaybackStatusProvider? statusProvider,
}) => MultiProvider(
  providers: [
    ChangeNotifierProvider<RequestCapabilitiesProvider>.value(value: provider),
    if (statusProvider == null)
      ChangeNotifierProvider(create: (_) => PlaybackStatusProvider())
    else
      ChangeNotifierProvider<PlaybackStatusProvider>.value(
        value: statusProvider,
      ),
  ],
  child: FluentApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Builder(
      builder: (context) {
        final status = context.watch<PlaybackStatusProvider>();
        status.playbackStatus.ready = true;
        return ScaffoldPage(
          content: Row(
            children: [
              CapabilityGate(
                capability: playPauseCapabilityFor(context),
                child: PlayPauseButton(
                  key: const Key('play-pause'),
                  disabled: status.notReady,
                  state: status.playbackStatus.state,
                  shadows: null,
                ),
              ),
              CapabilityGate(
                capability: capabilityForRequest(context, (c) => c.nextRequest),
                child: const NextButton(
                  key: Key('next'),
                  disabled: false,
                  shadows: null,
                ),
              ),
              CapabilityGate(
                capability: capabilityForRequest(
                  context,
                  (c) => c.previousRequest,
                ),
                child: const PreviousButton(
                  key: Key('previous'),
                  disabled: false,
                  shadows: null,
                ),
              ),
            ],
          ),
        );
      },
    ),
  ),
);

/// A page that shows what [read] tells of the open library, with [provider] as
/// its capabilities when there is one.
Widget capabilityProbe(
  RequestCapabilitiesProvider? provider,
  String Function(BuildContext context) read,
) {
  final app = FluentApp(
    home: Builder(builder: (context) => Text(read(context))),
  );
  return provider == null
      ? app
      : ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
          value: provider,
          child: app,
        );
}

void main() {
  group('collection creation', () {
    String creatable(BuildContext context) => [
      for (final type in [CollectionType.mix, CollectionType.playlist])
        '${type.name}: ${canCreateCollection(context, type)}',
    ].join(', ');

    testWidgets('a mix may be created only when the capability allows it', (
      tester,
    ) async {
      final provider = RequestCapabilitiesProvider(
        changes: const Stream.empty(),
        initial: snapshot().copyWith(
          createMixRequest: const RequestCapabilityHidden(),
        ),
      );
      addTearDown(provider.dispose);
      await tester.pumpWidget(capabilityProbe(provider, creatable));
      expect(find.text('mix: false, playlist: true'), findsOneWidget);
    });

    testWidgets('a mix may be created without a provider or a snapshot', (
      tester,
    ) async {
      await tester.pumpWidget(capabilityProbe(null, creatable));
      expect(find.text('mix: true, playlist: true'), findsOneWidget);

      final provider = RequestCapabilitiesProvider(
        changes: const Stream.empty(),
      );
      addTearDown(provider.dispose);
      await tester.pumpWidget(capabilityProbe(provider, creatable));
      expect(provider.snapshot, isNull);
      expect(find.text('mix: true, playlist: true'), findsOneWidget);
    });

    testWidgets('an empty page follows a snapshot that comes later', (
      tester,
    ) async {
      final changes = StreamController<RequestCapabilities>();
      addTearDown(changes.close);
      final provider = RequestCapabilitiesProvider(changes: changes.stream);
      addTearDown(provider.dispose);
      String detail(BuildContext context) => [
        for (final type in [
          CollectionType.mix,
          CollectionType.playlist,
          CollectionType.album,
        ])
          '${type.name}: ${emptyPageShowsDetail(context, type)}',
      ].join(', ');
      await tester.pumpWidget(capabilityProbe(provider, detail));
      expect(
        find.text('mix: true, playlist: true, album: true'),
        findsOneWidget,
      );

      changes.add(
        snapshot().copyWith(createMixRequest: const RequestCapabilityHidden()),
      );
      await tester.pump();
      expect(
        find.text('mix: false, playlist: true, album: true'),
        findsOneWidget,
      );
    });

    testWidgets('an M3U8 playlist may be imported only when available', (
      tester,
    ) async {
      String importable(BuildContext context) =>
          'import: ${canImportM3u8Playlist(context)}';
      await tester.pumpWidget(capabilityProbe(null, importable));
      expect(find.text('import: true'), findsOneWidget);

      final provider = RequestCapabilitiesProvider(
        changes: const Stream.empty(),
        initial: snapshot().copyWith(
          createM3u8PlaylistRequest: const RequestCapabilityHidden(),
        ),
      );
      addTearDown(provider.dispose);
      await tester.pumpWidget(capabilityProbe(provider, importable));
      expect(find.text('import: false'), findsOneWidget);
    });

    testWidgets(
      'a snapshot of another record leaves both to the open library',
      (tester) async {
        // This device plays on a renderer whose library keeps no mixes of its
        // own and takes no playlist file.
        final provider = RequestCapabilitiesProvider(
          changes: const Stream.empty(),
          initial: snapshot().copyWith(
            recordId: () => 'renderer-record',
            createMixRequest: const RequestCapabilityHidden(),
            createM3u8PlaylistRequest: const RequestCapabilityHidden(),
          ),
        );
        addTearDown(provider.dispose);
        await tester.pumpWidget(
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => _LibraryOpenStateProvider(
              openRecord: const LibraryRecordSummary(
                shareSync: false,
                recordId: 'local-record',
                alias: 'Music',
                kind: LibraryRecordKindSummaryLocal(
                  libraryId: 'library-id',
                  path: '/music',
                ),
              ),
            ),
            child: capabilityProbe(
              provider,
              (context) =>
                  '${creatable(context)}, '
                  'import: ${canImportM3u8Playlist(context)}',
            ),
          ),
        );
        expect(
          find.text('mix: true, playlist: true, import: true'),
          findsOneWidget,
        );
      },
    );
  });

  group('the artist page', () {
    const local = LibraryRecordSummary(
      shareSync: false,
      recordId: 'local-record',
      alias: 'Music',
      kind: LibraryRecordKindSummaryLocal(
        libraryId: 'library-id',
        path: '/music',
      ),
    );

    Future<String> gate(
      WidgetTester tester, {
      String? recordId,
      bool built = true,
      bool withOpenRecord = true,
    }) async {
      final provider = RequestCapabilitiesProvider(
        changes: const Stream.empty(),
        initial: snapshot().copyWith(
          recordId: () => recordId,
          fetchArtistPageRequest: built
              ? const RequestCapabilityAvailable()
              : const RequestCapabilityHidden(),
        ),
      );
      addTearDown(provider.dispose);
      await tester.pumpWidget(
        ChangeNotifierProvider<LibraryOpenStateProvider>(
          create: (_) => _LibraryOpenStateProvider(
            openRecord: withOpenRecord ? local : null,
          ),
          child: capabilityProbe(
            provider,
            (context) => 'page: ${canOpenArtistPage(context)}',
          ),
        ),
      );
      return tester.widget<Text>(find.byType(Text)).data!;
    }

    testWidgets('is built where the snapshot of the open library says so', (
      tester,
    ) async {
      expect(await gate(tester, recordId: 'local-record'), 'page: true');
      expect(
        await gate(tester, recordId: 'local-record', built: false),
        'page: false',
      );
    });

    testWidgets('is not built for a snapshot of another record', (
      tester,
    ) async {
      // This device plays on a renderer whose library builds artist pages.
      expect(await gate(tester, recordId: 'renderer-record'), 'page: false');
    });

    testWidgets('is not built without a capability provider', (tester) async {
      await tester.pumpWidget(
        capabilityProbe(null, (context) => '${canOpenArtistPage(context)}'),
      );
      expect(find.text('false'), findsOneWidget);
    });
  });

  testWidgets('every capability reason has its own text', (tester) async {
    final texts = <CapabilityReason, String>{};
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('en'),
        home: Builder(
          builder: (context) {
            for (final reason in CapabilityReason.values) {
              texts[reason] = capabilityReasonText(context, reason);
            }
            return const SizedBox.shrink();
          },
        ),
      ),
    );

    expect(texts, {
      CapabilityReason.notImplemented:
          'This device does not support this control.',
      CapabilityReason.ownerOnly: 'Only the device owner can do this.',
      CapabilityReason.peerTooOld:
          'Update Rune on the other device to use this feature.',
      CapabilityReason.accountRoleMissing:
          'Your account is not allowed to do this.',
      CapabilityReason.notAllowed: 'This device is not allowed to do that.',
      CapabilityReason.deviceOnly:
          'This library stays on this device, so no other device can play it.',
    });
  });

  testWidgets('hidden play and pause remove only the play-pause control', (
    tester,
  ) async {
    final provider = RequestCapabilitiesProvider(
      initial: snapshot(
        play: const RequestCapabilityHidden(),
        pause: const RequestCapabilityHidden(),
      ),
    );
    addTearDown(provider.dispose);

    await tester.pumpWidget(playbackControls(provider));

    expect(find.byType(PlayPauseButton), findsNothing);
    expect(find.byType(NextButton), findsOneWidget);
    expect(find.byType(PreviousButton), findsOneWidget);
  });

  testWidgets('disabled next is hidden', (tester) async {
    final provider = RequestCapabilitiesProvider(
      initial: snapshot(
        next: const RequestCapabilityDisabled(
          reason: CapabilityReason.ownerOnly,
        ),
      ),
    );
    addTearDown(provider.dispose);

    await tester.pumpWidget(playbackControls(provider));

    expect(find.byType(NextButton), findsNothing);
  });

  testWidgets('controls are available before the first snapshot', (
    tester,
  ) async {
    final provider = RequestCapabilitiesProvider(changes: const Stream.empty());
    addTearDown(provider.dispose);

    await tester.pumpWidget(playbackControls(provider));

    expect(provider.snapshot, isNull);
    expect(provider.pauseRequest, isA<RequestCapabilityAvailable>());
    expect(provider.nextRequest, isA<RequestCapabilityAvailable>());
    expect(provider.previousRequest, isA<RequestCapabilityAvailable>());
    expect(
      find.byWidgetPredicate(
        (widget) =>
            widget is Tooltip &&
            widget.message ==
                S
                    .of(tester.element(find.byType(NextButton)))
                    .capabilityOwnerOnly,
      ),
      findsNothing,
    );
  });

  testWidgets('like action follows the set-liked capability', (tester) async {
    final changes = StreamController<RequestCapabilities>();
    final provider = RequestCapabilitiesProvider(
      changes: changes.stream,
      initial: snapshot(
        setLiked: const RequestCapabilityDisabled(
          reason: CapabilityReason.ownerOnly,
        ),
      ),
    );
    addTearDown(provider.dispose);
    addTearDown(changes.close);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: provider,
          ),
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => _LibraryOpenStateProvider(),
          ),
        ],
        child: FluentApp(
          home: LikeButton(
            item: const PlayingItem.libraryItem(
              libraryId: 'library-b',
              fileUuid: '7f8b99b0-5a5a-4ce7-b6db-764a11edce4b',
            ),
            fetchLikedStatus: (_) async => false,
          ),
        ),
      ),
    );
    expect(find.byIcon(Symbols.favorite), findsNothing);

    changes.add(snapshot(setLiked: const RequestCapabilityAvailable()));
    await tester.pump();
    expect(find.byIcon(Symbols.favorite), findsOneWidget);
  });

  testWidgets('offline selection follows the set-download-rule capability', (
    tester,
  ) async {
    final changes = StreamController<RequestCapabilities>();
    final provider = RequestCapabilitiesProvider(
      changes: changes.stream,
      initial: snapshot(setDownloadRule: const RequestCapabilityHidden()),
    );
    addTearDown(provider.dispose);
    addTearDown(changes.close);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: provider,
          ),
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => _LibraryOpenStateProvider(
              openRecord: const LibraryRecordSummary(
                shareSync: false,
                recordId: 'local-record',
                alias: 'Music',
                kind: LibraryRecordKindSummaryLocal(
                  libraryId: 'library-id',
                  path: '/music',
                ),
              ),
            ),
          ),
        ],
        child: const SizedBox(key: Key('menu-host')),
      ),
    );
    bool offered() => supportsOfflineSelection(
      tester.element(find.byKey(const Key('menu-host'))),
    );
    expect(offered(), isFalse);

    changes.add(snapshot());
    await tester.pump();
    expect(offered(), isTrue);
  });

  testWidgets('PlaybackStatusProvider maps playback states to button icons', (
    tester,
  ) async {
    final messages = StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
      sync: true,
    );
    final statusProvider = PlaybackStatusProvider(
      statusStream: messages.stream,
    );
    addTearDown(statusProvider.dispose);
    addTearDown(messages.close);
    final capabilities = RequestCapabilitiesProvider();
    addTearDown(capabilities.dispose);

    await tester.pumpWidget(
      playbackControls(capabilities, statusProvider: statusProvider),
    );
    messages.add(
      RustSignalPack(_playbackStatus(PlaybackState.playing), Uint8List(0)),
    );
    await tester.pump();
    await tester.pump();
    expect(find.byIcon(Symbols.pause), findsOneWidget);
    messages.add(
      RustSignalPack(_playbackStatus(PlaybackState.paused), Uint8List(0)),
    );
    await tester.pumpWidget(
      playbackControls(capabilities, statusProvider: statusProvider),
    );
    expect(
      tester.widget<PlayPauseButton>(find.byKey(const Key('play-pause'))).state,
      PlaybackState.paused,
    );
    expect(find.byIcon(Symbols.pause), findsNothing);
    expect(find.byIcon(Symbols.play_arrow), findsOneWidget);
  });
}

PlaybackStatus _playbackStatus(PlaybackState state) => PlaybackStatus(
  sessionId: 'subsonic-library',
  state: state,
  progressSeconds: 0,
  progressPercentage: 0,
  duration: 10,
  playbackMode: PlaybackMode.sequential,
  ready: true,
  libPath: '',
);
