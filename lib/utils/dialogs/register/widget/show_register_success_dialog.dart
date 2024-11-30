import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';
import '../../../router/navigation.dart';

Future<void> showRegisterSuccessDialog(BuildContext context) async {
  await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Text(S.of(context).registerSuccess),
      content: Text(
        S.of(context).registerSuccessSubtitle,
        style: const TextStyle(height: 1.4),
      ),
      actions: [
        Button(
          child: Text(S.of(context).close),
          onPressed: () => $close(false),
        ),
      ],
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
