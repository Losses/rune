import 'package:get_storage/get_storage.dart';

import '../../utils/file_storage/mac_secure_manager.dart';

class FileStorageService {
  static const String _openedFilesKey = 'library_path';
  final GetStorage _storage = GetStorage();

  // Get the list of opened files
  List<String> _getOpenedFiles() {
    return (_storage
            .read<List<dynamic>>(_openedFilesKey)
            ?.cast<String>()
            .toList() ??
        []);
  }

  // Store file path
  void storeFilePath(String filePath) async {
    List<String> openedFiles = _getOpenedFiles();

    // If the file path already exists, remove it to re-add it to the end of the list
    openedFiles.remove(filePath);
    openedFiles.add(filePath);

    await MacSecureManager.shared.saveBookmark(filePath);

    // Store the updated list of file paths
    _storage.write(_openedFilesKey, openedFiles);
  }

  // Get the last opened file
  String? getLastOpenedFile() {
    List<String> openedFiles = _getOpenedFiles();
    if (openedFiles.isNotEmpty) {
      return openedFiles.last;
    }
    return null;
  }

  // Get all opened files
  List<String> getAllOpenedFiles() {
    return _getOpenedFiles();
  }

  // Clear all opened files
  Future<void> clearAllOpenedFiles() async {
    await _storage.remove(_openedFilesKey);
  }

  // Remove a specific file path
  Future<void> removeFilePath(String filePath) async {
    List<String> openedFiles = _getOpenedFiles();
    openedFiles.remove(filePath);
    await _storage.write(_openedFilesKey, openedFiles);
  }
}
