import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

Future<void> scanLibrary(BuildContext context, String path,
    [bool reload = true]) async {
  final libraryPath = Provider.of<LibraryPathProvider>(context, listen: false);

  await libraryPath.setLibraryPath(path, reload);
  ScanAudioLibraryRequest(path: path).sendSignalToRust();
}
