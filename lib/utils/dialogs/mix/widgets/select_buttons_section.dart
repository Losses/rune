import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/fic_list_extension.dart';

import '../../mix/utils/select_input_controller.dart';

import 'select_input_section.dart';

class SelectButtonsSection extends StatefulWidget {
  final String title;
  final String defaultValue;
  final List<SelectItem> items;
  final SelectInputController? controller;
  final bool disabled;

  const SelectButtonsSection({
    required this.title,
    required this.defaultValue,
    required this.items,
    this.controller,
    this.disabled = false,
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

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraint) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            widget.title,
            overflow: TextOverflow.ellipsis,
          ),
          const SizedBox(height: 4),
          Row(
            children: widget.items.mapIndexed(
              (i, item) {
                const r4 = Radius.circular(4.0);
                const r0 = Radius.circular(0);

                final buttonStyle = i == 0
                    ? const ButtonStyle(
                        shape: WidgetStatePropertyAll(
                          RoundedRectangleBorder(
                            borderRadius: BorderRadius.only(
                              topLeft: r4,
                              topRight: r0,
                              bottomLeft: r4,
                              bottomRight: r0,
                            ),
                          ),
                        ),
                      )
                    : i == widget.items.length - 1
                        ? const ButtonStyle(
                            shape: WidgetStatePropertyAll(
                              RoundedRectangleBorder(
                                borderRadius: BorderRadius.only(
                                  topLeft: r0,
                                  topRight: r4,
                                  bottomLeft: r0,
                                  bottomRight: r4,
                                ),
                              ),
                            ),
                          )
                        : const ButtonStyle(
                            shape: WidgetStatePropertyAll(
                              RoundedRectangleBorder(
                                borderRadius: BorderRadius.all(r0),
                              ),
                            ),
                          );
                final child = constraint.maxWidth > 140
                    ? SizedBox(
                        width: constraint.maxWidth - 48,
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
                        width: constraint.maxWidth - 48,
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
          ),
          const SizedBox(height: 12),
        ],
      );
    });
  }
}
