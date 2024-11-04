import 'package:fluent_ui/fluent_ui.dart';

class NumberSection extends StatefulWidget {
  final String title;
  final TextEditingController? controller;

  const NumberSection({
    required this.title,
    this.controller,
    super.key,
  });

  @override
  State<NumberSection> createState() => _NumberSectionState();
}

int bestInt(String x) {
  try {
    return int.parse(x);
  } catch (e) {
    return 0;
  }
}

class _NumberSectionState extends State<NumberSection> {
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
              child: NumberBox(
                value: bestInt(_controller.value.text),
                onChanged: (x) {
                  _controller.text = (x ?? 0).toString();
                },
                mode: SpinButtonPlacementMode.inline,
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}
