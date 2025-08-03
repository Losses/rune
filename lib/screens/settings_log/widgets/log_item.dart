import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/router/router_aware_flyout_controller.dart';
import '../../../widgets/context_menu_wrapper.dart';
import '../../../screens/settings_log/utils/open_log_item_context_menu.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../bindings/bindings.dart';

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
  final contextController = RouterAwareFlyoutController();
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
