import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

Future<void> scanLibrary(BuildContext context, String path) async {
  final library = Provider.of<LibraryPathProvider>(context, listen: false);

  await library.setLibraryPath(path, true);
  ScanAudioLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await ScanAudioLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      library.finalizeScanning();
      AnalyseAudioLibraryRequest(path: path).sendSignalToRust();
      return;
    }
  }
}
