import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/l10n.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';

void openLogItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  RouterAwareFlyoutController contextController,
  int index,
  void Function(int) onViewDetail,
  void Function(int) onDelete,
) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );

  if (!context.mounted) return;

  contextController.showFlyout(
    position: position,
    builder: (context) => MenuFlyout(
      items: [
        MenuFlyoutItem(
          leading: const Icon(Symbols.visibility),
          text: Text(S.of(context).viewLogDetail),
          onPressed: () => onViewDetail(index),
        ),
        MenuFlyoutItem(
          leading: const Icon(Symbols.delete),
          text: Text(S.of(context).delete),
          onPressed: () => onDelete(index),
        ),
      ],
    ),
  );
}
