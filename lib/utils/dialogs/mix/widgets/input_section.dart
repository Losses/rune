import 'package:fluent_ui/fluent_ui.dart';

class InputSection extends StatefulWidget {
  final String title;
  final TextEditingController? controller;

  const InputSection({
    required this.title,
    this.controller,
    super.key,
  });

  @override
  State<InputSection> createState() => _InputSectionState();
}

class _InputSectionState extends State<InputSection> {
  late final TextEditingController _controller;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? TextEditingController();
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
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(widget.title),
        const SizedBox(height: 4),
        Row(
          children: [
            Expanded(
              child: TextBox(controller: _controller),
            ),
          ],
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}
