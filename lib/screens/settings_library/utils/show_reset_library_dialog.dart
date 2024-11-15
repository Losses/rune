import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../generated/l10n.dart';

Future<bool?> showResetLibraryDialog(BuildContext context) {
  return $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Text(S.of(context).resetLibraryQuestion),
      content: Text(
        S.of(context).resetLibrarySubtitle,
      ),
      actions: [
        FilledButton(
          child: Text(S.of(context).resetLibrary),
          onPressed: () {
            $close(true);
          },
        ),
        Button(
          child: Text(S.of(context).cancel),
          onPressed: () => $close(false),
        ),
      ],
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
