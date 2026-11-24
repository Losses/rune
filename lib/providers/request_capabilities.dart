import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import 'library_open_state.dart';
import 'status.dart';

class RequestCapabilitiesProvider extends ChangeNotifier {
  RequestCapabilitiesProvider({
    Stream<RequestCapabilities>? changes,
    RequestCapabilities? initial,
  }) : _snapshot = initial ?? RequestCapabilities.latestRustSignal?.message {
    _subscription =
        (changes ??
                RequestCapabilities.rustSignalStream.map(
                  (event) => event.message,
                ))
            .listen(_update);
  }

  RequestCapabilities? _snapshot;
  late final StreamSubscription<RequestCapabilities> _subscription;

  RequestCapabilities? get snapshot => _snapshot;

  RequestCapability get volumeRequest =>
      _snapshot?.volumeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get playRequest =>
      _snapshot?.playRequest ?? const RequestCapabilityAvailable();
  RequestCapability get pauseRequest =>
      _snapshot?.pauseRequest ?? const RequestCapabilityAvailable();
  RequestCapability get nextRequest =>
      _snapshot?.nextRequest ?? const RequestCapabilityAvailable();
  RequestCapability get previousRequest =>
      _snapshot?.previousRequest ?? const RequestCapabilityAvailable();
  RequestCapability get switchRequest =>
      _snapshot?.switchRequest ?? const RequestCapabilityAvailable();
  RequestCapability get seekRequest =>
      _snapshot?.seekRequest ?? const RequestCapabilityAvailable();
  RequestCapability get removeRequest =>
      _snapshot?.removeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get setPlaybackModeRequest =>
      _snapshot?.setPlaybackModeRequest ?? const RequestCapabilityAvailable();
  RequestCapability get movePlaylistItemRequest =>
      _snapshot?.movePlaylistItemRequest ?? const RequestCapabilityAvailable();
  RequestCapability get setLikedRequest =>
      _snapshot?.setLikedRequest ?? const RequestCapabilityAvailable();
  RequestCapability get setDownloadRuleRequest =>
      _snapshot?.setDownloadRuleRequest ?? const RequestCapabilityAvailable();
  RequestCapability get fetchAllMixesRequest =>
      _snapshot?.fetchAllMixesRequest ?? const RequestCapabilityAvailable();
  RequestCapability get addItemToMixRequest =>
      _snapshot?.addItemToMixRequest ?? const RequestCapabilityAvailable();
  RequestCapability get complexQueryRequest =>
      _snapshot?.complexQueryRequest ?? const RequestCapabilityAvailable();
  RequestCapability get getAnalyzeCountRequest =>
      _snapshot?.getAnalyzeCountRequest ?? const RequestCapabilityAvailable();
  RequestCapability get fetchDirectoryTreeRequest =>
      _snapshot?.fetchDirectoryTreeRequest ??
      const RequestCapabilityAvailable();
  RequestCapability get castResolvedItemsRequest =>
      _snapshot?.castResolvedItemsRequest ?? const RequestCapabilityAvailable();
  RequestCapability get fetchAllPlaylistsRequest =>
      _snapshot?.fetchAllPlaylistsRequest ?? const RequestCapabilityAvailable();
  RequestCapability get createMixRequest =>
      _snapshot?.createMixRequest ?? const RequestCapabilityAvailable();
  RequestCapability get createM3u8PlaylistRequest =>
      _snapshot?.createM3u8PlaylistRequest ??
      const RequestCapabilityAvailable();

  void _update(RequestCapabilities snapshot) {
    _snapshot = snapshot;
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription.cancel();
    super.dispose();
  }
}

RequestCapability playPauseCapabilityFor(
  BuildContext context, {
  bool listen = true,
}) {
  final capabilities = requestCapabilitiesOf(context, listen: listen);
  if (capabilities == null) return const RequestCapabilityAvailable();
  if (playPauseIsHidden(capabilities)) return const RequestCapabilityHidden();
  return _playPauseCapability(context, capabilities, listen: listen);
}

RequestCapabilitiesProvider? requestCapabilitiesOf(
  BuildContext context, {
  bool listen = true,
}) => Provider.of<RequestCapabilitiesProvider?>(context, listen: listen);

RequestCapability capabilityForRequest(
  BuildContext context,
  RequestCapability Function(RequestCapabilitiesProvider provider) select, {
  bool listen = true,
}) {
  final capabilities = requestCapabilitiesOf(context, listen: listen);
  return capabilities == null
      ? const RequestCapabilityAvailable()
      : select(capabilities);
}

/// Whether the open library answers a request, so it may be asked. Without a
/// capability provider every request counts as answered.
bool requestAvailable(
  BuildContext context,
  RequestCapability Function(RequestCapabilitiesProvider provider) select, {
  bool listen = false,
}) =>
    capabilityForRequest(context, select, listen: listen)
        is RequestCapabilityAvailable;

/// Whether the open library answers a request that belongs to the library, not
/// to the device. A snapshot of another record, as a renderer sends while this
/// device plays on it, says nothing about the library open here, so the request
/// then counts as answered. While this device remote-controls a library, the
/// snapshot is the one of that library.
bool _libraryAnswers(
  BuildContext context,
  RequestCapability Function(RequestCapabilitiesProvider provider) select, {
  bool listen = false,
}) {
  final capabilities = requestCapabilitiesOf(context, listen: listen);
  if (capabilities == null) return true;
  final state = Provider.of<LibraryOpenStateProvider?>(context, listen: listen);
  final open = state?.openRecord;
  if (state != null &&
      !state.isRemoteControl &&
      open != null &&
      capabilities.snapshot?.recordId != open.recordId) {
    return true;
  }
  return select(capabilities) is RequestCapabilityAvailable;
}

/// Whether the open library has analysis data to roam by, which the library of
/// a streaming service does not.
bool canStartRoaming(BuildContext context) =>
    _libraryAnswers(context, (p) => p.getAnalyzeCountRequest);

/// Whether the open library is made of files that a playlist file can list. The
/// directory tree stands for that: only a library of files has one.
bool canExportPlaylistFile(BuildContext context) =>
    _libraryAnswers(context, (p) => p.fetchDirectoryTreeRequest);

/// Whether the open library can be played on another device.
bool canPlayOnOtherDevice(BuildContext context) =>
    _libraryAnswers(context, (p) => p.castResolvedItemsRequest);

/// Whether the open library keeps mixes the user can create. Set [listen] when
/// calling it while building.
bool canCreateMix(BuildContext context, {bool listen = false}) =>
    _libraryAnswers(context, (p) => p.createMixRequest, listen: listen);

/// Whether the open library can create a playlist from an M3U8 file.
bool canCreateM3u8Playlist(BuildContext context) =>
    _libraryAnswers(context, (p) => p.createM3u8PlaylistRequest);

RequestCapability _playPauseCapability(
  BuildContext context,
  RequestCapabilitiesProvider capabilities, {
  required bool listen,
}) {
  final playing =
      Provider.of<PlaybackStatusProvider?>(
        context,
        listen: listen,
      )?.playbackStatus.state ==
      PlaybackState.playing;
  final active = playing ? capabilities.pauseRequest : capabilities.playRequest;
  if (active is RequestCapabilityHidden &&
      (capabilities.pauseRequest is! RequestCapabilityHidden ||
          capabilities.playRequest is! RequestCapabilityHidden)) {
    return const RequestCapabilityDisabled(
      reason: CapabilityReason.notImplemented,
    );
  }
  return active;
}

bool playPauseIsHidden(RequestCapabilitiesProvider capabilities) =>
    capabilities.playRequest is RequestCapabilityHidden &&
    capabilities.pauseRequest is RequestCapabilityHidden;
