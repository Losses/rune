import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../api/set_media_library_path.dart';
import '../../api/testlibrary_initialized.dart';

import '../failed_to_initialize_library.dart';

import 'show_select_library_mode_dialog.dart';

Future<(bool, LibraryInitializeMode?)?> testAndSelectLibraryMode(
  BuildContext context,
  String path,
) async {
  final (testSuccess, initialized, testError) =
      await testLibraryInitialized(path);

  if (!testSuccess) {
    if (!context.mounted) return null;
    await showFailedToInitializeLibrary(context, testError);
    return null;
  }

  if (!initialized) {
    final initializeMode = await showSelectLibraryModeDialog(context);

    if (initializeMode == null) return (false, null);

    return (false, stringToLibraryInitializeMode(initializeMode));
  }

  return (true, null);
}
