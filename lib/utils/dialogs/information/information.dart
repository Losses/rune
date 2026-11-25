import 'package:fluent_ui/fluent_ui.dart';

import '../../l10n.dart';
import '../../router/navigation.dart';

Future<bool?> showInformationDialog({
  required BuildContext context,
  required String title,
  required String subtitle,
  bool barrierDismissible = true,
  bool dismissWithEsc = true,
}) async {
  return await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Column(
        children: [
          SizedBox(height: 8),
          Text(title),
        ],
      ),
      content: Text(
        subtitle,
        style: const TextStyle(height: 1.4),
      ),
      actions: [
        Button(
          child: Text(S.of(context).close),
          onPressed: () => $close(false),
        ),
      ],
    ),
    barrierDismissible: barrierDismissible,
    dismissWithEsc: dismissWithEsc,
  );
}
