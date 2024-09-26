import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/context_menu_wrapper.dart';

abstract class SearchCard extends StatelessWidget {
  final int index;
  final FlyoutController contextController = FlyoutController();
  final GlobalKey contextAttachKey = GlobalKey();

  SearchCard({super.key, required this.index});

  int getItemId();
  String getItemTitle();
  Widget buildLeadingWidget(double size);
  void onPressed(BuildContext context);
  void onContextMenu(BuildContext context, Offset position);

  @override
  Widget build(BuildContext context) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (position) => onContextMenu(context, position),
      child: Button(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(0)),
        ),
        onPressed: () => onPressed(context),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(3),
          child: LayoutBuilder(
            builder: (context, constraints) {
              final size = min(constraints.maxWidth, constraints.maxHeight);
              return Row(
                children: [
                  buildLeadingWidget(size),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Text(
                            getItemTitle(),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}
