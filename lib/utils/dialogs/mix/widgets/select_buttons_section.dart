import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/fic_list_extension.dart';
import '../../mix/utils/select_input_controller.dart';
import 'select_input_section.dart';

class SelectButtonsSection extends StatefulWidget {
  final String? title;
  final String defaultValue;
  final List<SelectItem> Function(BuildContext) items;
  final SelectInputController? controller;
  final bool disabled;
  final int rows;

  const SelectButtonsSection({
    required this.title,
    required this.defaultValue,
    required this.items,
    this.controller,
    this.disabled = false,
    this.rows = 1,
    super.key,
  });

  @override
  State<SelectButtonsSection> createState() => _SelectButtonsSectionState();
}

class _SelectButtonsSectionState extends State<SelectButtonsSection> {
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

  // Generate the border style for the buttons
  ButtonStyle _getButtonStyle(
      int index, int rowItemCount, int rowIndex, int totalRows) {
    const r4 = Radius.circular(4.0);
    const r0 = Radius.circular(0);

    // Determine if it's the first button in the row
    final isFirstInRow = index % rowItemCount == 0;
    // Determine if it's the last button in the row
    final isLastInRow = (index + 1) % rowItemCount == 0;
    // Determine if it's the first row
    final isFirstRow = rowIndex == 0;
    // Determine if it's the last row
    final isLastRow = rowIndex == totalRows - 1;

    return ButtonStyle(
      shape: WidgetStatePropertyAll(
        RoundedRectangleBorder(
          borderRadius: BorderRadius.only(
            topLeft: isFirstInRow && isFirstRow ? r4 : r0,
            topRight: isLastInRow && isFirstRow ? r4 : r0,
            bottomLeft: isFirstInRow && isLastRow ? r4 : r0,
            bottomRight: isLastInRow && isLastRow ? r4 : r0,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final items = widget.items(context);

    return LayoutBuilder(builder: (context, constraint) {
      // Calculate the number of buttons per row
      final itemsPerRow = (items.length / widget.rows).ceil();

      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (widget.title != null) ...[
            Text(
              widget.title!,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 4),
          ],
          // Use a Column to wrap multiple rows of buttons
          Column(
            children: List.generate(widget.rows, (rowIndex) {
              // Calculate the start and end index for the current row
              final startIndex = rowIndex * itemsPerRow;
              final endIndex =
                  (startIndex + itemsPerRow).clamp(0, items.length);

              // If there are no items for this row, don't render it
              if (startIndex >= items.length) return const SizedBox.shrink();

              // Get the items for the current row
              final rowItems = items.sublist(startIndex, endIndex);

              return Row(
                children: rowItems.mapIndexed(
                  (i, item) {
                    final absoluteIndex = startIndex + i;
                    final buttonStyle = _getButtonStyle(
                      absoluteIndex,
                      itemsPerRow,
                      rowIndex,
                      widget.rows,
                    );

                    final child = constraint.maxWidth > 140
                        ? SizedBox(
                            width: (constraint.maxWidth - 48) / rowItems.length,
                            child: Row(
                              children: [
                                Icon(item.icon, size: 18),
                                const SizedBox(width: 8),
                                Expanded(
                                  child: Text(
                                    item.title,
                                    textAlign: TextAlign.start,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                ),
                              ],
                            ),
                          )
                        : SizedBox(
                            width: (constraint.maxWidth - 48) / rowItems.length,
                            child: Text(
                              item.title,
                              textAlign: TextAlign.start,
                              overflow: TextOverflow.ellipsis,
                            ),
                          );

                    return Expanded(
                      child: _controller.selectedValue == item.value
                          ? FilledButton(
                              onPressed: widget.disabled ? null : () {},
                              style: buttonStyle,
                              child: child,
                            )
                          : Button(
                              onPressed: widget.disabled
                                  ? null
                                  : () => setState(() {
                                        _controller.selectedValue = item.value;
                                      }),
                              style: buttonStyle,
                              child: child,
                            ),
                    );
                  },
                ).toList(),
              );
            }),
          ),
          const SizedBox(height: 12),
        ],
      );
    });
  }
}
