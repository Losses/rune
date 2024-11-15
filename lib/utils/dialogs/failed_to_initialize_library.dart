import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/navigation.dart';
import '../../generated/l10n.dart';

Future<void> showFailedToInitializeLibrary(
  BuildContext context,
  String? errorMessage,
) async {
  await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Text(S.of(context).unableToOpenLibrary),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 400),
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            S.of(context).unableToOpenLibrarySubtitle,
          ),
          const SizedBox(height: 8),
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
        FilledButton(
          child: Text(S.of(context).close),
          onPressed: () => $close(false),
        ),
      ],
    ),
    dismissWithEsc: true,
  );
}
