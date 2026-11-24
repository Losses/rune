import 'package:fluent_ui/fluent_ui.dart';

import '../utils/get_dir_path.dart';

class LocalFolderPickerProvider {
  LocalFolderPickerProvider({
    Future<String?> Function(BuildContext context)? pickDirectory,
  }) : pickDirectory = pickDirectory ?? getDirPath;

  /// Asks for a folder; [BuildContext] is where Rune's own dialog opens.
  Future<String?> Function(BuildContext context) pickDirectory;
}
