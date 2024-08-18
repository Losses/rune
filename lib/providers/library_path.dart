import 'package:flutter/material.dart';

import '../messages/connection.pb.dart';
import '../utils/file_storage_service.dart';

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;
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

  Future<void> setLibraryPath(String filePath) async {
    _currentPath = filePath;
    // Send the signal to Rust
    MediaLibraryPath(path: filePath).sendSignalToRust();
    notifyListeners();
    _fileStorageService.storeFilePath(filePath);
  }

  List<String> getAllOpenedFiles() {
    return _fileStorageService.getAllOpenedFiles();
  }
}
