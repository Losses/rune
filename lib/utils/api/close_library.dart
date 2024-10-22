import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/messages/library_manage.pb.dart';
import 'package:rune/providers/library_path.dart';

Future<void> closeLibrary(BuildContext context) async {
  final library = Provider.of<LibraryPathProvider>(context, listen: false);

  final path = library.currentPath;
  CloseLibraryRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await CloseLibraryResponse.rustSignalStream.first;

    if (rustSignal.message.path == path) {
      return;
    }
  }
}
