import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/router/router_aware_flyout_controller.dart';

import '../rune_clickable.dart';

import 'queue.dart';

class QueueButton extends StatefulWidget {
  final List<Shadow>? shadows;

  const QueueButton({
    super.key,
    required this.shadows,
  });

  @override
  State<QueueButton> createState() => _QueueButtonState();
}

class _QueueButtonState extends State<QueueButton> {
  final _contextController = RouterAwareFlyoutController();

  @override
  dispose() {
    super.dispose();
    _contextController.dispose();
  }

  openContextMenu(BuildContext context) {
    _contextController.showFlyout(
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
      controller: _contextController.controller,
      child: RuneClickable(
        onPressed: () {
          openContextMenu(context);
        },
        child: Icon(
          Symbols.list_alt,
          shadows: widget.shadows,
        ),
      ),
    );
  }
}
