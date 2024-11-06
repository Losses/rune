import 'package:fluent_ui/fluent_ui.dart';

import '../utils/api/set_media_library_path.dart';
import '../utils/file_storage/file_storage_service.dart';

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;

  final FileStorageService _fileStorageService = FileStorageService();

  LibraryPathProvider() {
    String libraryPath =
        const String.fromEnvironment('LIBRARY_PATH', defaultValue: "");
    if (libraryPath.isNotEmpty) {
      setLibraryPath(libraryPath);
    } else {
      _fileStorageService.getLastOpenedFile().then((x) {
        if (x != null) {
          setLibraryPath(x);
        }
      });
    }
  }

  String? get currentPath => _currentPath;

  Future<(bool, String?)> setLibraryPath(String filePath) async {
    _currentPath = filePath;
    notifyListeners();

    final (success, error) = await setMediaLibraryPath(filePath);

    if (success) {
      _fileStorageService.storeFilePath(filePath);
    } else {
      removeCurrentPath();
    }

    return (success, error);
  }

  Future<List<String>> getAllOpenedFiles() {
    return _fileStorageService.getAllOpenedFiles();
  }

  // Clear all opened files
  Future<void> clearAllOpenedFiles() async {
    await _fileStorageService.clearAllOpenedFiles();
    _currentPath = null;

    notifyListeners();
  }

  // Add a method to remove a specific file path
  Future<void> removeOpenedFile(String filePath) async {
    await _fileStorageService.removeFilePath(filePath);
    if (_currentPath == filePath) {
      _currentPath = null;
    }
    notifyListeners();
  }

  removeCurrentPath() {
    _currentPath = null;
    notifyListeners();
  }
}
