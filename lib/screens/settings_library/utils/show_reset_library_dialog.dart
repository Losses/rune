import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

Future<bool?> showResetLibraryDialog(BuildContext context) {
  return $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: const Text('Reset Library?'),
      content: const Text(
        'Resetting the library will clear your file open history. Your existing media library will remain unchanged. Do you want to proceed?',
      ),
      actions: [
        FilledButton(
          child: const Text('Reset Library'),
          onPressed: () {
            $close(true);
          },
        ),
        Button(
          child: const Text('Cancel'),
          onPressed: () => $close(false),
        ),
      ],
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
