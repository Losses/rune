import 'package:fluent_ui/fluent_ui.dart';

import '../../../../bindings/bindings.dart';
import '../../../l10n.dart';
import '../../../api/request_failure.dart';

import '../../information/error.dart';
import 'show_register_invalid_dialog.dart';

/// Explain why a license was not registered: an invalid key has its own
/// dialog, any other failure shows its message.
Future<void> showRegisterFailure(
  BuildContext context,
  RequestFailure failure,
) => failure.cause is RequestFailureCauseLicenseInvalid
    ? showRegisterInvalidDialog(context)
    : showRegisterFailedDialog(
        context,
        requestFailureMessage(S.of(context), failure),
      );

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
