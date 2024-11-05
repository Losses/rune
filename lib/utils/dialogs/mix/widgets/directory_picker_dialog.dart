import 'package:fluent_ui/fluent_ui.dart';

import '../../../../widgets/directory/directory_tree.dart';
import '../../../../widgets/responsive_dialog_actions.dart';

import '../../unavailable_dialog_on_band.dart';

class DirectoryPickerDialog extends StatelessWidget {
  final DirectoryTreeController controller;
  final void Function(Set<String>?) $close;
  const DirectoryPickerDialog({
    super.key,
    required this.controller,
    required this.$close,
  });

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      $close: $close,
      child: ContentDialog(
        title: const Column(
          children: [
            SizedBox(height: 8),
            Text("Pick Directory"),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            DirectoryTree(
              controller: controller,
            )
          ],
        ),
        actions: [
          ResponsiveDialogActions(
            FilledButton(
              child: const Text('Confirm'),
              onPressed: () {
                $close(controller.value);
              },
            ),
            Button(
              child: const Text('Cancel'),
              onPressed: () => $close(null),
            ),
          ),
        ],
      ),
    );
  }
}
