import 'package:file_selector/file_selector.dart';

Future<String?> getDirPath() async {
  final path = await getDirectoryPath();
  return path;
}
