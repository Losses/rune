import 'package:fluent_ui/fluent_ui.dart';

import '../utils/toggle_switch_controller.dart';

class ToggleSwitchSection extends StatefulWidget {
  final ToggleSwitchController? controller;
  final Widget content;

  const ToggleSwitchSection({
    super.key,
    this.controller,
    required this.content,
  });

  @override
  ToggleSwitchSectionState createState() => ToggleSwitchSectionState();
}

class ToggleSwitchSectionState extends State<ToggleSwitchSection> {
  late final ToggleSwitchController _controller;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? ToggleSwitchController(false);
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
    return ToggleSwitch(
      checked: _controller.isChecked,
      content: Expanded(child: widget.content),
      leadingContent: true,
      onChanged: (bool value) {
        setState(() {
          _controller.isChecked = value;
        });
      },
    );
  }
}
