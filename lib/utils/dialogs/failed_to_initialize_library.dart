import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';

Future<void> showFailedToInitializeLibrary(
    BuildContext context, String? errorMessage) async {
  await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: const Text('Unable to Open Library'),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 400),
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'The Library could not be opened due to the following error:',
          ),
          const SizedBox(height: 8),
          Expanded(
            child: SingleChildScrollView(
              child: SelectableText(
                errorMessage ?? 'Unknown Error',
              ),
            ),
          ),
        ],
      ),
      actions: [
        FilledButton(
          child: const Text('Close'),
          onPressed: () => $close(false),
        ),
      ],
    ),
    dismissWithEsc: true,
  );
}
