import 'dart:async';
import 'dart:typed_data';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

/// The places the file dialog's navigation pane lists on this device.
Future<ListFileDialogPlacesResponse> listFileDialogPlaces() async {
  final signal = await awaitResponse(
    'ListFileDialogPlacesRequest',
    ListFileDialogPlacesResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        ListFileDialogPlacesRequest(callId: callId).sendSignalToRust(),
  );
  return signal.message;
}

/// What [typed] names, resolved against the folder at [current].
Future<ResolveFileDialogPathResponse> resolveFileDialogPath(
  String typed,
  FileDialogLocation current,
) async {
  final signal = await awaitResponse(
    'ResolveFileDialogPathRequest',
    ResolveFileDialogPathResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ResolveFileDialogPathRequest(
      typed: typed,
      current: current,
      callId: callId,
    ).sendSignalToRust(),
  );
  return signal.message;
}

/// The items of the folder at [location].
Future<List<FileDialogEntry>> listFileDialogFolder(
  FileDialogLocation location,
) async {
  final signal = await awaitResponse(
    'ListFileDialogFolderRequest',
    ListFileDialogFolderResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ListFileDialogFolderRequest(
      location: location,
      callId: callId,
    ).sendSignalToRust(),
  );
  return signal.message.entries;
}

/// The pixels of a thumbnail the platform drew.
class FileDialogThumbnailPixels {
  const FileDialogThumbnailPixels(this.width, this.height, this.rgba);

  final int width;
  final int height;

  /// Straight RGBA8 rows, top row first.
  final Uint8List rgba;
}

/// The platform's thumbnail of the item at [location], fit into a square of
/// [size] physical pixels, or null when it draws none.
Future<FileDialogThumbnailPixels?> fetchFileDialogThumbnail(
  FileDialogLocation location,
  int size,
) async {
  final signal = await awaitResponse(
    'FetchFileDialogThumbnailRequest',
    FileDialogThumbnailResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => FetchFileDialogThumbnailRequest(
      location: location,
      size: size,
      callId: callId,
    ).sendSignalToRust(),
    // A thumbnailer program may take a while on a large video.
    timeout: const Duration(seconds: 60),
  );
  return switch (signal.message.thumbnail) {
    FileDialogThumbnailDrawn(:final width, :final height) =>
      FileDialogThumbnailPixels(width, height, signal.binary),
    _ => null,
  };
}

/// A new folder called [name] in [parent].
Future<FileDialogLocation> createFileDialogFolder(
  FileDialogLocation parent,
  String name,
) async {
  final signal = await awaitResponse(
    'CreateFileDialogFolderRequest',
    CreateFileDialogFolderResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => CreateFileDialogFolderRequest(
      parent: parent,
      name: name,
      callId: callId,
    ).sendSignalToRust(),
  );
  return signal.message.location;
}

/// The item at [location] renamed to [name] within its folder.
Future<FileDialogLocation> renameFileDialogEntry(
  FileDialogLocation location,
  String name,
) async {
  final signal = await awaitResponse(
    'RenameFileDialogEntryRequest',
    RenameFileDialogEntryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => RenameFileDialogEntryRequest(
      location: location,
      name: name,
      callId: callId,
    ).sendSignalToRust(),
  );
  return signal.message.location;
}

/// The file dialog failure carried by [error], if it is one.
FileDialogFailure? fileDialogFailureOf(Object error) => switch (error) {
  RequestFailure(cause: RequestFailureCauseFileDialogFailed(:final failure)) =>
    failure,
  _ => null,
};
