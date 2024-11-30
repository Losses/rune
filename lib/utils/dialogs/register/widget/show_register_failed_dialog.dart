import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';
import '../../../router/navigation.dart';

Future<void> showRegisterFailedDialog(
  BuildContext context,
  String? errorMessage,
) async {
  await $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Text(S.of(context).registerFailed),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 400),
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            S.of(context).registerFailedSubtitle,
            style: const TextStyle(height: 1.4),
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
