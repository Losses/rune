import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import 'queue.dart';

class QueueButton extends StatelessWidget {
  final List<Shadow>? shadows;

  QueueButton({
    super.key,
    required this.shadows,
  });

  final contextController = FlyoutController();

  openContextMenu(BuildContext context) {
    contextController.showFlyout(
      autoModeConfiguration: FlyoutAutoConfiguration(
        preferredMode: FlyoutPlacementMode.topCenter,
      ),
      builder: (context) {
        return FlyoutContent(
          child: LayoutBuilder(
            builder: (BuildContext context, BoxConstraints constraints) {
              double maxHeight = constraints.maxHeight - 100;

              return ConstrainedBox(
                constraints: BoxConstraints(
                  maxHeight: maxHeight,
                  maxWidth: 380,
                ),
                child: const Queue(),
              );
            },
          ),
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
      controller: contextController,
      child: IconButton(
        onPressed: () {
          openContextMenu(context);
        },
        icon: Icon(
          Symbols.list_alt,
          shadows: shadows,
        ),
      ),
    );
  }
}
