import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_path.dart';
import '../../../utils/api/close_library.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';

import '../utils/add_remote_device_form_controller.dart';

import 'add_remote_device_form.dart';

class AddRemoteDeviceDialog extends StatefulWidget {
  final bool navigateIfFailed;
  final void Function(void) $close;

  const AddRemoteDeviceDialog({
    super.key,
    required this.navigateIfFailed,
    required this.$close,
  });

  @override
  AddRemoteDeviceDialogState createState() => AddRemoteDeviceDialogState();
}

class AddRemoteDeviceDialogState extends State<AddRemoteDeviceDialog> {
  late AddRemoteDeviceFormController controller;

  @override
  void initState() {
    super.initState();
    controller = AddRemoteDeviceFormController();
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  _addConnection() async {
    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);

    await closeLibrary(context);

    if (!mounted) return;

    final (switched, cancelled, error) = await libraryPath.setLibraryPath(
      context,
      '@RR|${controller.toWebSocketUrl()}',
      null,
    );

    widget.$close(null);
    if (!cancelled) {
      if (!context.mounted) return;
      await showFailedToInitializeLibrary(context, error);
      if (widget.navigateIfFailed) {
        $$replace('/');
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            SizedBox(height: 8),
            Text(s.addConnection),
          ],
        ),
        content: AddRemoteDeviceForm(
          controller: controller,
        ),
        actions: [
          FilledButton(
            onPressed: _addConnection,
            child: Text(s.addConnection),
          ),
          Button(
            onPressed: () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
