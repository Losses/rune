import 'dart:io';

/// The folder the tests of one file keep their settings in, empty to begin
/// with. Each test file passes a [name] of its own.
///
/// The folder is emptied here, before the settings store writes anything, and
/// is left in place when the tests are done. The store writes its file and a
/// backup of it some time after a setting is saved and gives a caller nothing
/// to wait for, so a write can still be on its way when the tests end.
/// Deleting the folder then either fails with "Directory not empty", when the
/// write lands first, or makes the write fail for want of its folder.
Future<Directory> emptySettingsFolder(String name) async {
  final folder = Directory('${Directory.systemTemp.path}/$name');
  if (folder.existsSync()) await folder.delete(recursive: true);
  return folder.create();
}
