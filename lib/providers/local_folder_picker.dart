import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';

import '../utils/get_dir_path.dart';

class RecoveryFolder {
  const RecoveryFolder(this.uri, {this.persistAccess});

  final String uri;
  final Future<void> Function()? persistAccess;
}

class LocalFolderPickerProvider {
  LocalFolderPickerProvider({
    Future<String?> Function(BuildContext context)? pickDirectory,
  }) : pickDirectory = pickDirectory ?? getDirPath;

  /// Asks for a folder; [BuildContext] is where Rune's own dialog opens.
  Future<String?> Function(BuildContext context) pickDirectory;

  late Future<RecoveryFolder?> Function(BuildContext, String)
  pickRecoveryFolder = _pickRecoveryFolder;

  Future<RecoveryFolder?> _pickRecoveryFolder(
    BuildContext context,
    String initialUri,
  ) async {
    if (!Platform.isAndroid) {
      final path = await pickDirectory(context);
      return path == null ? null : RecoveryFolder(path);
    }
    const channel = MethodChannel('ci.not.rune/document_tree_recovery');
    final result = await channel.invokeMapMethod<String, Object?>(
      'pick',
      initialUri,
    );
    if (result == null) return null;
    return RecoveryFolder(
      result['uri']! as String,
      persistAccess: () => channel.invokeMethod<void>('persist', result),
    );
  }
}
