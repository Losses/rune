import 'package:fluent_ui/fluent_ui.dart';

import '../../l10n.dart';
import '../../router/navigation.dart';

Future<void> showErrorDialog({
  required BuildContext context,
  required String title,
  String? subtitle,
  String? errorMessage,
  bool useFilledButton = true,
  bool barrierDismissible = true,
}) async {
  await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Column(
        children: [
          SizedBox(height: 8),
          Text(title),
        ],
      ),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 400),
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (subtitle != null)
            Text(
              subtitle,
              style: const TextStyle(height: 1.4),
            ),
          if (subtitle != null) const SizedBox(height: 8),
          Expanded(
            child: SingleChildScrollView(
              child: SelectableText(
                errorMessage ?? S.of(context).unknownError,
              ),
            ),
          ),
        ],
      ),
      actions: [
        useFilledButton
            ? FilledButton(
                child: Text(S.of(context).close),
                onPressed: () => $close(false),
              )
            : Button(
                child: Text(S.of(context).close),
                onPressed: () => $close(false),
              ),
      ],
    ),
    barrierDismissible: barrierDismissible,
    dismissWithEsc: true,
  );
}
