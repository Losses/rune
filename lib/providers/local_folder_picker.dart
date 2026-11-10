import '../utils/get_dir_path.dart';

class LocalFolderPickerProvider {
  LocalFolderPickerProvider({Future<String?> Function()? pickDirectory})
    : pickDirectory = pickDirectory ?? getDirPath;

  Future<String?> Function() pickDirectory;
}
