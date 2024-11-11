import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/all.dart';
import '../../providers/library_path.dart';

Future<void> closeLibrary(BuildContext context) async {
  final library = Provider.of<LibraryPathProvider>(context, listen: false);

  final path = library.currentPath;
  CloseLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await CloseLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      library.removeCurrentPath();
      return;
    }
  }
}
