import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/status.dart';
import '../../utils/l10n.dart';
import '../../widgets/capability_gate.dart';
import '../../widgets/playback_controller/next_button.dart';
import '../../widgets/playback_controller/play_pause_button.dart';
import '../../widgets/playback_controller/previous_button.dart';

RequestCapabilities snapshot({
  RequestCapability play = const RequestCapabilityAvailable(),
  RequestCapability pause = const RequestCapabilityAvailable(),
  RequestCapability next = const RequestCapabilityAvailable(),
  RequestCapability previous = const RequestCapabilityAvailable(),
}) {
  const capability = RequestCapabilityAvailable();
  return RequestCapabilities(
    recordId: null,
    generation: Uint64(BigInt.zero),
    attachRendererRequest: capability,
    detachRendererRequest: capability,
    listRenderersRequest: capability,
    handoffRequest: capability,
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
    operatePlaybackWithMixQueryRequest: capability,
    castPreflightRequest: capability,
    castLoadRequest: capability,
    refreshSessionRequest: capability,
    sessionSnapshotRequest: capability,
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
  ).copyWith(
    playRequest: play,
    pauseRequest: pause,
    nextRequest: next,
    previousRequest: previous,
  );
}

Widget playbackControls(RequestCapabilitiesProvider provider) => MultiProvider(
  providers: [
    ChangeNotifierProvider<RequestCapabilitiesProvider>.value(value: provider),
    ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
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

  testWidgets('disabled next stays present and exposes the localized reason', (
    tester,
  ) async {
    final provider = RequestCapabilitiesProvider(
      initial: snapshot(
        next: const RequestCapabilityDisabled(
          reason: CapabilityReason.ownerOnly,
        ),
      ),
    );
    addTearDown(provider.dispose);

    await tester.pumpWidget(playbackControls(provider));

    expect(find.byType(NextButton), findsOneWidget);
    final disabledOpacity = find
        .ancestor(of: find.byType(NextButton), matching: find.byType(Opacity))
        .first;
    expect(tester.widget<Opacity>(disabledOpacity).opacity, 0.45);
    expect(
      tester
          .widget<Tooltip>(
            find
                .ancestor(
                  of: find.byType(NextButton),
                  matching: find.byType(Tooltip),
                )
                .first,
          )
          .message,
      S.of(tester.element(find.byType(NextButton))).capabilityOwnerOnly,
    );
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
}
