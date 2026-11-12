import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../router/navigation.dart';

import 'select_library_mode_dialog.dart';

Future<LibraryInitializeMode?> showSelectLibraryModeDialog(
  BuildContext context,
) async {
  return await $showModal<LibraryInitializeMode?>(
    context,
    (context, $close) => SelectLibraryModeDialog(onClose: $close),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
