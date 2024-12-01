import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';

import 'information/error.dart';

Future<void> showFailedToInitializeLibrary(
  BuildContext context,
  String? errorMessage,
) async {
  await showErrorDialog(
    context: context,
    title: S.of(context).unableToOpenLibrary,
    subtitle: S.of(context).unableToOpenLibrarySubtitle,
    errorMessage: errorMessage,
    useFilledButton: true,
  );
}
