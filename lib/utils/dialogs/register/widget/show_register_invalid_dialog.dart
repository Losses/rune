import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';

import '../../information/information.dart';

Future<void> showRegisterInvalidDialog(BuildContext context) async {
  await showInformationDialog(
    context: context,
    title: S.of(context).registerInvalid,
    subtitle: S.of(context).registerInvalidSubtitle,
  );
}
