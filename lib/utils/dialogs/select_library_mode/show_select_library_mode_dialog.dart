import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

import 'select_library_mode_dialog.dart';

Future<String?> showSelectLibraryModeDialog(BuildContext context) async {
  return await $showModal<String?>(
    context,
    (context, $close) => SelectLibraryModeDialog(onClose: $close),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
