import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import '../../router/navigation.dart';

import 'export_cover_wall_dialog.dart';

void showExportCoverWallDialog(
  BuildContext context,
  CollectionType type,
  int id,
) async {
  await $showModal<void>(
    context,
    (context, $close) => ExportCoverWallDialog(
      type: type,
      id: id,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
