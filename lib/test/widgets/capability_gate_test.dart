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
    setLibraryBackgroundSyncRequest: capability,
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
    startBroadcastRequest: capability,
    stopBroadcastRequest: capability,
    startListeningRequest: capability,
    stopListeningRequest: capability,
    getDiscoveredDeviceRequest: capability,
    startServerRequest: capability,
    stopServerRequest: capability,
    listClientsRequest: capability,
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
  ).copyWith(
    playRequest: play,
    pauseRequest: pause,
    nextRequest: next,
    previousRequest: previous,
    setLikedRequest: setLiked,
  );
}

class _LibraryOpenStateProvider extends ChangeNotifier
    implements LibraryOpenStateProvider {
  @override
  bool get isReadOnlyReplica => false;

  @override
  LibraryRecordSummary? get openRecord => null;

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

void main() {
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
