import 'package:fluent_ui/fluent_ui.dart';

import 'package:rune/utils/dialogs/mix/utils/slider_controller.dart';

class SliderSection extends StatefulWidget {
  final String title;
  final double defaultValue;
  final SliderController? controller;

  const SliderSection({
    super.key,
    this.controller,
    required this.title,
    this.defaultValue = 0.0,
  });

  @override
  State<SliderSection> createState() => _SliderSectionState();
}

class _SliderSectionState extends State<SliderSection> {
  late final SliderController _controller;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? SliderController(widget.defaultValue);
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
        const SizedBox(height: 6),
        Row(
          children: [
            Expanded(
              child: AnimatedBuilder(
                animation: _controller,
                builder: (context, child) {
                  return Slider(
                    value: _controller.value,
                    min: 0,
                    max: 100,
                    onChanged: (v) {
                      setState(() {
                        _controller.value = v;
                      });
                    },
                  );
                },
              ),
            ),
            SizedBox(
              width: 28,
              child: Text(
                _controller.value.toStringAsFixed(0),
                textAlign: TextAlign.end,
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}
