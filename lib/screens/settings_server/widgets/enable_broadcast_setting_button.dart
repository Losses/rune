import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';
import '../../../widgets/settings/settings_box_base.dart';
import '../../../providers/broadcast.dart';

class EnableBroadcastSetting extends StatefulWidget {
  const EnableBroadcastSetting({
    super.key,
  });

  @override
  State<EnableBroadcastSetting> createState() => _EnableBroadcastSettingState();
}

class _EnableBroadcastSettingState extends State<EnableBroadcastSetting> {
  final _menuController = RouterAwareFlyoutController();

  Widget buildExpanderContent(BuildContext context) {
    final broadcast = Provider.of<BroadcastProvider>(context, listen: false);
    final s = S.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Button(
          onPressed: broadcast.isServerRunning
              ? () => broadcast.isBroadcasting
                  ? broadcast.stopBroadcast()
                  : broadcast.startBroadcast()
              : null,
          child: broadcast.isBroadcasting ? Text(s.stop) : Text(s.start),
        ),
      ],
    );
  }

  Widget buildDefaultContent(BuildContext context) {
    final broadcast = Provider.of<BroadcastProvider>(context, listen: false);
    final s = S.of(context);

    return FlyoutTarget(
      controller: _menuController.controller,
      child: Button(
        onPressed: broadcast.isServerRunning
            ? () => broadcast.isBroadcasting
                ? broadcast.stopBroadcast()
                : broadcast.startBroadcast()
            : null,
        child: broadcast.isBroadcasting ? Text(s.stop) : Text(s.start),
      ),
    );
  }

  @override
  dispose() {
    _menuController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    Provider.of<BroadcastProvider>(context);

    return SettingsBoxBase(
      title: s.enableBroadcast,
      subtitle: s.enableBroadcastSubtitle,
      icon: Symbols.graph_5,
      buildExpanderContent: buildExpanderContent,
      buildDefaultContent: buildDefaultContent,
    );
  }
}
