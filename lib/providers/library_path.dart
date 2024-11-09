import 'package:fluent_ui/fluent_ui.dart';

import '../utils/api/set_media_library_path.dart';
import '../utils/file_storage/file_storage_service.dart';

final FileStorageService _fileStorageService = FileStorageService();

Future<String?> getInitialPath() async {
  const String libraryPath =
      String.fromEnvironment('LIBRARY_PATH', defaultValue: "");
  if (libraryPath.isNotEmpty) {
    return libraryPath;
  } else {
    return await _fileStorageService.getLastOpenedFile();
  }
}

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;
  Set<String> libraryHistory = {};

  LibraryPathProvider(String? initialPath) {
    if (initialPath != null) {
      setLibraryPath(initialPath);
    }

    getAllOpenedFiles().then((x) {
      for (final item in x) {
        libraryHistory.add(item);
      }

      if (libraryHistory.isNotEmpty) {
        notifyListeners();
      }
    });
  }

  String? get currentPath => _currentPath;

  Future<(bool, String?)> setLibraryPath(String filePath) async {
    _currentPath = filePath;
    libraryHistory.add(filePath);
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
    libraryHistory.clear();

    notifyListeners();
  }

  // Add a method to remove a specific file path
  Future<void> removeOpenedFile(String filePath) async {
    await _fileStorageService.removeFilePath(filePath);
    if (_currentPath == filePath) {
      _currentPath = null;
      libraryHistory.remove(filePath);
    }
    notifyListeners();
  }

  removeCurrentPath() {
    _currentPath = null;
    notifyListeners();
  }
}
