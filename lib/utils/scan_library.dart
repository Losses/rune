import 'package:player/providers/library_manager.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

Future<void> scanLibrary(BuildContext context, String path) async {
  final libraryPath = Provider.of<LibraryPathProvider>(context, listen: false);
  final libraryManager =
      Provider.of<LibraryManagerProvider>(context, listen: false);

  await libraryPath.setLibraryPath(path, true);
  ScanAudioLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await ScanAudioLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      libraryPath.finalizeScanning();
      libraryManager.analyseLibrary(path);
      return;
    }
  }
}
