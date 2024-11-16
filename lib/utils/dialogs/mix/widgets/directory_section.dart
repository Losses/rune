import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/dialogs/mix/widgets/directory_picker_dialog.dart';
import '../../../../widgets/directory/directory_tree.dart';
import '../../../../utils/l10n.dart';

import '../../../router/navigation.dart';

class DirectorySection extends StatefulWidget {
  final DirectoryTreeController? controller;
  final Set<String>? defaultValue;

  const DirectorySection({
    super.key,
    this.controller,
    this.defaultValue,
  });

  @override
  State<DirectorySection> createState() => _DirectorySectionState();
}

class _DirectorySectionState extends State<DirectorySection> {
  late final DirectoryTreeController _controller;

  @override
  void initState() {
    super.initState();

    _controller =
        widget.controller ?? DirectoryTreeController(widget.defaultValue);
  }

  @override
  void dispose() {
    if (widget.controller == null) {
      _controller.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final value = _controller.value;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(S.of(context).directories),
        const SizedBox(height: 4),
        Button(
          onPressed: () async {
            await showContentDialog(context, _controller);
            setState(() {});
          },
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 2),
            child: SizedBox(
              width: double.infinity,
              child: Text(
                value == null || value.isEmpty
                    ? S.of(context).allDirectories
                    : value.length == 1
                        ? S.of(context).oneDirectory
                        : S.of(context).manyDirectories(value.length),
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.start,
              ),
            ),
          ),
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}

Future<void> showContentDialog(
    BuildContext context, DirectoryTreeController controller) async {
  final internalController = DirectoryTreeController(controller.value);

  final result = await $showModal<Set<String>>(
    context,
    (context, $close) => DirectoryPickerDialog(
      controller: internalController,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );

  internalController.dispose();

  if (result != null) {
    controller.value = result;
  }
}
