import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/context_menu_wrapper.dart';
import '../../../messages/all.dart';

import '../../settings_library/widgets/settings_button.dart';

void openLogItemContextMenu(
  Offset localPosition,
  BuildContext context,
  GlobalKey contextAttachKey,
  FlyoutController contextController,
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

class LogItem extends StatefulWidget {
  final LogDetail log;
  final int index;
  final void Function(int) onTap;
  final void Function(int) onRemove;

  const LogItem({
    super.key,
    required this.log,
    required this.index,
    required this.onTap,
    required this.onRemove,
  });

  @override
  LogItemState createState() => LogItemState();
}

class LogItemState extends State<LogItem> {
  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  @override
  dispose() {
    super.dispose();
    contextController.dispose();
  }

  IconData _getLogLevelIcon(String level) {
    switch (level.toLowerCase()) {
      case 'error':
        return Symbols.error;
      case 'warning':
        return Symbols.warning;
      case 'info':
        return Symbols.info;
      default:
        return Symbols.description;
    }
  }

  @override
  Widget build(BuildContext context) {
    final splittedDomain = widget.log.domain.split("::");

    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: (x) => openLogItemContextMenu(
        x,
        context,
        contextAttachKey,
        contextController,
        widget.index,
        widget.onTap,
        widget.onRemove,
      ),
      onMiddleClick: (_) {},
      child: SettingsButton(
        icon: _getLogLevelIcon(widget.log.level),
        title: splittedDomain.last,
        subtitle: DateTime.fromMillisecondsSinceEpoch(
          widget.log.date.toInt() * 1000,
        ).toString(),
        onPressed: () {
          widget.onTap(widget.index);
        },
      ),
    );
  }
}
