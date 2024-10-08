import 'package:fluent_ui/fluent_ui.dart';

import '../../mix/utils/select_input_controller.dart';

class SelectItem {
  final String value;
  final String title;
  final IconData icon;

  SelectItem({
    required this.value,
    required this.title,
    required this.icon,
  });
}

class SelectInputSection extends StatefulWidget {
  final String title;
  final String defaultValue;
  final List<SelectItem> items;
  final SelectInputController? controller;

  const SelectInputSection({
    required this.title,
    required this.defaultValue,
    required this.items,
    this.controller,
    super.key,
  });

  @override
  State<SelectInputSection> createState() => _SelectInputSectionState();
}

class _SelectInputSectionState extends State<SelectInputSection> {
  late final SelectInputController _controller;

  @override
  void initState() {
    super.initState();

    _controller =
        widget.controller ?? SelectInputController(widget.defaultValue);
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
    return LayoutBuilder(builder: (context, constraint) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(widget.title),
          const SizedBox(height: 4),
          Row(
            children: [
              Expanded(
                child: ComboBox<String>(
                  value: _controller.selectedValue,
                  items: widget.items.map<ComboBoxItem<String>>((item) {
                    return ComboBoxItem<String>(
                      value: item.value,
                      child: SizedBox(
                        width: constraint.maxWidth - 48,
                        child: Row(
                          children: [
                            Icon(item.icon, size: 18),
                            const SizedBox(width: 8),
                            Text(
                              item.title,
                              textAlign: TextAlign.start,
                            )
                          ],
                        ),
                      ),
                    );
                  }).toList(),
                  onChanged: (value) => setState(() {
                    _controller.selectedValue = value;
                  }),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
        ],
      );
    });
  }
}
