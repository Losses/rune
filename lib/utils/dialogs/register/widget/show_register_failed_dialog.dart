import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';

import '../../information/error.dart';

Future<void> showRegisterFailedDialog(
  BuildContext context,
  String? errorMessage,
) async {
  await showErrorDialog(
    context: context,
    title: S.of(context).registerFailed,
    subtitle: S.of(context).registerFailedSubtitle,
    errorMessage: errorMessage,
    useFilledButton: false,
    barrierDismissible: true,
  );
}
