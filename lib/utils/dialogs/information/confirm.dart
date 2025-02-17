import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

Future<bool?> showConfirmDialog({
  required BuildContext context,
  required String title,
  required String subtitle,
  required String yesLabel,
  required String noLabel,
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
        FilledButton(
          child: Text(yesLabel),
          onPressed: () => $close(true),
        ),
        Button(
          child: Text(noLabel),
          onPressed: () => $close(false),
        ),
      ],
    ),
    barrierDismissible: barrierDismissible,
    dismissWithEsc: dismissWithEsc,
  );
}
