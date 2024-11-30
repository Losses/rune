import 'package:fluent_ui/fluent_ui.dart';

import '../../l10n.dart';

import '../information/error.dart';

Future<void> showCreateImportM3u8SuccessDialog(
  BuildContext context,
  List<String> failedList,
) async {
  await showErrorDialog(
    context: context,
    title: S.of(context).importM3u8Success,
    subtitle: S.of(context).importM3u8SuccessSubtitle,
    errorMessage: failedList.join('\n\n'),
    useFilledButton: false,
    barrierDismissible: true,
  );
}
