import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../router/navigation.dart';

import 'file_dialog_window.dart';

/// What the file dialog lets a person pick, as the Windows common item dialog
/// offers it.
enum FileDialogMode {
  /// One existing file.
  openFile,

  /// One or more existing files.
  openFiles,

  /// One folder.
  pickFolder,

  /// A file name to save to, new or existing.
  saveFile,
}

/// One entry of the dialog's file type box, such as "Playlists (*.m3u8)".
class FileDialogFileType {
  const FileDialogFileType(this.label, this.extensions);

  /// Every file, whatever its extension.
  const FileDialogFileType.all(this.label) : extensions = const [];

  final String label;

  /// Extensions without the dot. Empty matches every file.
  final List<String> extensions;

  bool get matchesEverything => extensions.isEmpty;

  /// Whether a file called [name] belongs to this type.
  bool matches(String name) {
    if (matchesEverything) return true;
    final lower = name.toLowerCase();
    return extensions.any(
      (extension) => lower.endsWith('.${extension.toLowerCase()}'),
    );
  }

  /// The label with its patterns, as Windows shows it.
  String get display => matchesEverything
      ? '$label (*.*)'
      : '$label (${extensions.map((extension) => '*.$extension').join(';')})';
}

/// How the dialog opens, after `IFileDialog`: its mode, title and button
/// text, the folder it starts in, the name it suggests, and its file types.
class FileDialogOptions {
  const FileDialogOptions({
    required this.mode,
    this.title,
    this.okButtonLabel,
    this.initialFolder,
    this.suggestedName,
    this.fileTypes = const [],
    this.initialFileTypeIndex = 0,
  });

  final FileDialogMode mode;

  /// Replaces the title the mode gives the dialog.
  final String? title;

  /// Replaces the text the mode gives the confirm button.
  final String? okButtonLabel;

  /// The folder shown first. Without one the dialog opens where it was last
  /// used, or in the account's home folder.
  final FileDialogLocation? initialFolder;

  /// The name the file name box starts with, for saving.
  final String? suggestedName;

  /// The file type box's entries. Without any, every file is shown.
  final List<FileDialogFileType> fileTypes;

  final int initialFileTypeIndex;
}

/// What the person picked.
class FileDialogResult {
  const FileDialogResult(this.locations, this.fileTypeIndex);

  /// One location, or several for [FileDialogMode.openFiles].
  final List<FileDialogLocation> locations;

  /// The file type selected when the dialog closed.
  final int fileTypeIndex;

  FileDialogLocation get location => locations.first;
}

/// Show Rune's file dialog over [context] and wait for what the person picks,
/// or null when they cancel.
Future<FileDialogResult?> showFileDialog(
  BuildContext context,
  FileDialogOptions options,
) {
  return $showModal<FileDialogResult>(
    context,
    (context, close) => FileDialogWindow(options: options, close: close),
    name: 'file-dialog',
    dismissWithEsc: false,
    barrierDismissible: false,
    staysUsableWhenShrunk: false,
  );
}
