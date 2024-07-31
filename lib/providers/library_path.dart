import 'package:fluent_ui/fluent_ui.dart';

import '../utils/file_storage_service.dart';
import '../../messages/connection.pb.dart';

class LibraryPathProvider with ChangeNotifier {
  String? _currentPath;
  final FileStorageService _fileStorageService = FileStorageService();

  LibraryPathProvider() {
    _initialize();
  }

  String? get currentPath => _currentPath;

  _initialize() {
    final lastOpenedFile = _fileStorageService.getLastOpenedFile();
    if (lastOpenedFile != null) {
      setLibraryPath(lastOpenedFile);
    }
  }

  setLibraryPath(String filePath) {
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
