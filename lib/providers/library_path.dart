import 'package:fluent_ui/fluent_ui.dart';

import '../messages/connection.pb.dart';
import '../utils/file_storage_service.dart';

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;
  bool _scanning = false;

  final FileStorageService _fileStorageService = FileStorageService();

  LibraryPathProvider() {
    String libraryPath =
        const String.fromEnvironment('LIBRARY_PATH', defaultValue: "");
    if (libraryPath.isNotEmpty) {
      setLibraryPath(libraryPath);
    } else {
      final lastOpenedFile = _fileStorageService.getLastOpenedFile();
      if (lastOpenedFile != null) {
        setLibraryPath(lastOpenedFile);
      }
    }
  }

  String? get currentPath => _currentPath;
  bool get scanning => _scanning;

  Future<void> setLibraryPath(String filePath, [bool scan = false]) async {
    _currentPath = filePath;
    _scanning = scan;
    // Send the signal to Rust
    MediaLibraryPath(path: filePath).sendSignalToRust();
    notifyListeners();
    _fileStorageService.storeFilePath(filePath);
  }

  Future<void> finalizeScanning() async {
    _scanning = false;
    notifyListeners();
  }

  List<String> getAllOpenedFiles() {
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
}
