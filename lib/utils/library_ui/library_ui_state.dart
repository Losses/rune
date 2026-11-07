import 'package:flutter/foundation.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../platform.dart';
import '../remote_connection_messages.dart';

enum PeerLibrariesStatus { loading, error, empty, rows }

class PeerLibrariesState {
  final PeerLibrariesStatus status;
  final String? error;
  final List<PeerLibrary> libraries;
  const PeerLibrariesState(
    this.status, {
    this.error,
    this.libraries = const [],
  });
}

PeerLibrariesState peerLibrariesState(
  AppLocalizations s,
  ListPeerLibrariesResponse? response,
) {
  if (response == null) {
    return const PeerLibrariesState(PeerLibrariesStatus.loading);
  }
  if (response.errorKind != null) {
    return PeerLibrariesState(
      PeerLibrariesStatus.error,
      error: remoteOpenErrorMessage(s, response.errorKind),
    );
  }
  if (response.libraries.isEmpty) {
    return const PeerLibrariesState(PeerLibrariesStatus.empty);
  }
  return PeerLibrariesState(
    PeerLibrariesStatus.rows,
    libraries: response.libraries,
  );
}

class LibraryRowState {
  final LocalLibrary? library;
  final String subtitle;
  const LibraryRowState(this.library, this.subtitle);
}

LibraryRowState libraryRowState(
  String path,
  List<LocalLibrary> libraries,
  Map<String, String> deviceAliases,
  String Function(String) fromDevice,
  String readOnly, {
  String Function(String)? describePath,
}) {
  for (final library in libraries) {
    if (library.path == path) {
      final owner = library.ownerFingerprint;
      return LibraryRowState(
        library,
        owner == null || owner.isEmpty
            ? (describePath?.call(path) ?? path)
            : fromDevice(deviceAliases[owner] ?? library.alias) +
                (library.accessLevel == DeviceLibraryLevel.read
                    ? ' · $readOnly'
                    : ''),
      );
    }
  }
  return LibraryRowState(null, describePath?.call(path) ?? path);
}

final _filesOnDevice = RegExp(
  r'^(?:/private)?/var/mobile/Containers/Shared/AppGroup/[^/]+'
  r'/File Provider Storage(?:/(.*))?$',
);
final _filesInICloud = RegExp(
  r'^(?:/private)?/var/mobile/Library/Mobile Documents/com~apple~CloudDocs'
  r'(?:/(.*))?$',
);

/// Where an iOS folder shows in the Files app, such as
/// "On My iPhone › music", in place of its sandbox path. Other paths are
/// returned as they are.
String filesAppLocation(
  String path, {
  required String onThisDevice,
  required String iCloudDrive,
}) {
  String join(String root, String? rest) => [
        root,
        ...?rest?.split('/').where((part) => part.isNotEmpty),
      ].join(' › ');

  final local = _filesOnDevice.firstMatch(path);
  if (local != null) return join(onThisDevice, local.group(1));
  final cloud = _filesInICloud.firstMatch(path);
  if (cloud != null) return join(iCloudDrive, cloud.group(1));
  return path;
}

/// How a library folder is named to the user: on iOS by where it shows in
/// the Files app ([filesAppLocation]), elsewhere by its path.
String describeLibraryPath(AppLocalizations s, String path) {
  if (defaultTargetPlatform != TargetPlatform.iOS) return path;
  return filesAppLocation(
    path,
    onThisDevice: iosDevice == IosDevice.iPad ? s.onMyIPad : s.onMyIPhone,
    iCloudDrive: s.iCloudDrive,
  );
}

// Whether the open local library is a read-only replica.
bool currentLibraryIsReadOnly(String? path, List<LocalLibrary> libraries) {
  if (path == null || path.startsWith('@RR|')) return false;
  return libraries.any((library) =>
      library.path == path &&
      library.ownerFingerprint != null &&
      library.ownerFingerprint!.isNotEmpty &&
      library.accessLevel == DeviceLibraryLevel.read);
}
