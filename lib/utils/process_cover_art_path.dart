import 'api/fetch_remote_file.dart';

/// Processes a cover art path that may come from a remote server
/// This function will use the Rust API to fetch the file and save it locally if needed
Future<String> processCoverArtPath(String path) async {
  // Skip processing for empty paths or already local paths
  if (path.isEmpty) {
    return path;
  }

  // If the path starts with http:// or https://, it needs to be downloaded
  if (path.startsWith('http://') || path.startsWith('https://')) {
    try {
      final localPath = await fetchRemoteFile(path);

      path = localPath;
    } catch (e) {
      throw Exception('Failed to process remote path: $e');
    }
  }

  // Path is already local, return as-is
  return path;
}
