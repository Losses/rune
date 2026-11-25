import 'package:fluent_ui/fluent_ui.dart';

import '../../../chip_input/chip_input.dart';

class SearchChipInputSection extends StatefulWidget {
  final String title;
  final Future<List<AutoSuggestBoxItem<int>>> Function() getInitResult;
  final Future<List<AutoSuggestBoxItem<int>>> Function(String) searchForItems;
  final ChipInputController<int>? controller;

  const SearchChipInputSection({
    super.key,
    this.controller,
    required this.title,
    required this.getInitResult,
    required this.searchForItems,
  });

  @override
  State<SearchChipInputSection> createState() => _SearchChipInputSectionState();
}

class _SearchChipInputSectionState extends State<SearchChipInputSection> {
  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(widget.title),
        const SizedBox(height: 4),
        ChipInput(
          controller: widget.controller,
          getInitResult: widget.getInitResult,
          searchFor: widget.searchForItems,
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}
