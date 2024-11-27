import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

import 'export_cover_wall_dialog.dart';

void showExportCoverWallDialog(
  BuildContext context,
  List<(String, String)> queries,
  String title,
) async {
  await $showModal<void>(
    context,
    (context, $close) => ExportCoverWallDialog(
      queries: queries,
      title: title,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
