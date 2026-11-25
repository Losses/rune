import 'package:fluent_ui/fluent_ui.dart';

import '../../l10n.dart';

import '../information/error.dart';

Future<void> showCreateImportM3u8FailedDialog(
  BuildContext context,
  String message,
) async {
  await showErrorDialog(
    context: context,
    title: S.of(context).importM3u8Failed,
    subtitle: S.of(context).importM3u8FailedSubtitle,
    errorMessage: message,
    useFilledButton: false,
    barrierDismissible: true,
  );
}
