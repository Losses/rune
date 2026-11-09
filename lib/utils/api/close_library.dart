import 'request_failure.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_registry.dart';

Future<void> closeLibrary(BuildContext context) async {
  final library = Provider.of<LibraryRegistryProvider>(context, listen: false);

  while (true) {
    await awaitResponse(
      'CloseLibraryRequest',
      CloseLibraryResponse.rustSignalStream,
      send: () => CloseLibraryRequest().sendSignalToRust(),
    );

    library.onClosed();
    return;
  }
}
