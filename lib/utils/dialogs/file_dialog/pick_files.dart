import 'dart:io';

import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;
import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';

import 'file_dialog.dart';
import 'file_dialog_model.dart';

/// Whether Rune's own file dialog picks files on this device: on Windows and
/// Linux, the embedded kiosk included. Macs, iPhones, iPads and Android
/// devices keep the system's picker, which is what grants Rune access to a
/// place it has not seen before.
bool get runeFileDialogAvailable => Platform.isWindows || Platform.isLinux;

List<FileDialogFileType> _fileTypes(List<XTypeGroup> groups) => [
  for (final group in groups)
    FileDialogFileType(group.label ?? '', group.extensions ?? const []),
];

FileDialogLocation? _folder(String? path) => path == null
    ? null
    : FileDialogLocation(root: const FileDialogRootFileSystem(), path: path);

FastFilePickerPath _picked(FileDialogLocation location) =>
    FastFilePickerPath.fromPath(nameOf(location), location.path);

/// One existing file.
Future<FastFilePickerPath?> pickFile(
  BuildContext context, {
  List<XTypeGroup> acceptedTypeGroups = const [],
}) async {
  if (!runeFileDialogAvailable) {
    return FastFilePicker.pickFile(acceptedTypeGroups: acceptedTypeGroups);
  }
  final result = await showFileDialog(
    context,
    FileDialogOptions(
      mode: FileDialogMode.openFile,
      fileTypes: _fileTypes(acceptedTypeGroups),
    ),
  );
  return result == null ? null : _picked(result.location);
}

/// One or more existing files.
Future<List<FastFilePickerPath>?> pickMultipleFiles(
  BuildContext context, {
  List<XTypeGroup> acceptedTypeGroups = const [],
}) async {
  if (!runeFileDialogAvailable) {
    return FastFilePicker.pickMultipleFiles(
      acceptedTypeGroups: acceptedTypeGroups,
    );
  }
  final result = await showFileDialog(
    context,
    FileDialogOptions(
      mode: FileDialogMode.openFiles,
      fileTypes: _fileTypes(acceptedTypeGroups),
    ),
  );
  return result?.locations.map(_picked).toList();
}

/// Where to save a file, as a path.
Future<String?> pickSaveFile(
  BuildContext context, {
  String? suggestedName,
  String? initialDirectory,
  List<XTypeGroup> acceptedTypeGroups = const [],
}) async {
  if (!runeFileDialogAvailable) {
    return FastFilePicker.pickSaveFile(
      suggestedName: suggestedName,
      initialDirectory: initialDirectory,
      acceptedTypeGroups: acceptedTypeGroups,
    );
  }
  final result = await showFileDialog(
    context,
    FileDialogOptions(
      mode: FileDialogMode.saveFile,
      suggestedName: suggestedName,
      initialFolder: _folder(initialDirectory),
      fileTypes: _fileTypes(acceptedTypeGroups),
    ),
  );
  return result?.location.path;
}

/// One folder, as a path. The caller handles the platforms where the
/// system's picker grants access ([runeFileDialogAvailable] false).
Future<String?> pickFolder(BuildContext context) async {
  final result = await showFileDialog(
    context,
    const FileDialogOptions(mode: FileDialogMode.pickFolder),
  );
  return result?.location.path;
}
