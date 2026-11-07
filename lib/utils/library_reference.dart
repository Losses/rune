import 'dart:io';

/// Scheme of the name Rust gives a replica library instead of a folder path.
/// A replica lives in the app's own data directory, which moves when iOS
/// reinstalls the app, so it is named by its library id.
const replicaScheme = 'replica://';

bool isReplicaReference(String path) => path.startsWith(replicaScheme);

/// The last segment of a library's folder, or the library id for a replica.
String libraryFolderName(String path) {
  if (isReplicaReference(path)) return path.substring(replicaScheme.length);
  return File(path).uri.pathSegments.last;
}
