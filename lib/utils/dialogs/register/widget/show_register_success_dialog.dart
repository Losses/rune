import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';

import '../../information/information.dart';

Future<void> showRegisterSuccessDialog(BuildContext context) async {
  await showInformationDialog(
    context: context,
    title: S.of(context).registerSuccess,
    subtitle: S.of(context).registerSuccessSubtitle,
  );
}