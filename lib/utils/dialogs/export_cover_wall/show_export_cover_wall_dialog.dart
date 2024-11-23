import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import '../../router/navigation.dart';

import 'export_cover_wall_dialog.dart';

void showExportCoverWallDialog(
  BuildContext context,
  CollectionType type,
  String title,
  int id,
) async {
  await $showModal<void>(
    context,
    (context, $close) => ExportCoverWallDialog(
      type: type,
      id: id,
      title: title,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
