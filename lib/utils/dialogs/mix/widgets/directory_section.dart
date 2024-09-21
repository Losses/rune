import 'package:fluent_ui/fluent_ui.dart';

import 'package:player/widgets/directory/directory_tree.dart';
import 'package:player/utils/dialogs/mix/widgets/directory_picker_dialog.dart';

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
        const Text("Directories"),
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
              child: Row(
                children: [
                  Text(value == null || value.isEmpty
                      ? "All directories"
                      : value.length == 1
                          ? '1 Directory'
                          : '${value.length} Directories'),
                  Expanded(child: Container()),
                ],
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

  final result = await showDialog<Set<String>>(
    context: context,
    builder: (context) => DirectoryPickerDialog(controller: internalController),
  );

  internalController.dispose();

  if (result != null) {
    controller.value = result;
  }
}
