import 'package:fluent_ui/fluent_ui.dart';

class EditableComboBoxSection extends StatefulWidget {
  final String title;
  final Future<List<String>> Function() getItems;
  final TextEditingController? controller;

  const EditableComboBoxSection({
    required this.title,
    required this.getItems,
    this.controller,
    super.key,
  });

  @override
  State<EditableComboBoxSection> createState() =>
      _EditableComboBoxSectionState();
}

class _EditableComboBoxSectionState extends State<EditableComboBoxSection> {
  late TextEditingController _controller;
  late Future<List<String>> items;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? TextEditingController();
    items = widget.getItems();
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
                child: FutureBuilder<List<String>>(
                    future: items,
                    builder: (context, snapshot) {
                      return AutoSuggestBox<String>(
                        controller: _controller,
                        items: (snapshot.data ?? [])
                            .map<AutoSuggestBoxItem<String>>((e) {
                          return AutoSuggestBoxItem<String>(
                            value: e,
                            label: e,
                          );
                        }).toList(),
                      );
                    }),
              ),
            ],
          ),
          const SizedBox(height: 12),
        ],
      );
    });
  }
}
