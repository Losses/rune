import 'package:get_storage/get_storage.dart';

class FileStorageService {
  static const String _openedFilesKey = 'opened_files';
  final GetStorage _storage = GetStorage();

  FileStorageService() {
    // Initialize storage
    GetStorage.init();
  }

  // Get the list of opened files
  List<String> _getOpenedFiles() {
    return _storage.read<List<String>>(_openedFilesKey)?.toSet().toList() ?? [];
  }

  // Store file path
  Future<void> storeFilePath(String filePath) async {
    List<String> openedFiles = _getOpenedFiles();

    // If the file path already exists, remove it to re-add it to the end of the list
    openedFiles.remove(filePath);
    openedFiles.add(filePath);

    // Store the updated list of file paths
    await _storage.write(_openedFilesKey, openedFiles);
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
}
