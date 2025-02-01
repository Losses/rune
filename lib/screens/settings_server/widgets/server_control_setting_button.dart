import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_base.dart';
import '../../../providers/broadcast.dart';

class ServerControlSetting extends StatefulWidget {
  const ServerControlSetting({super.key});

  @override
  State<ServerControlSetting> createState() => _ServerControlSettingState();
}

class _ServerControlSettingState extends State<ServerControlSetting> {
  final _menuController = FlyoutController();

  Widget buildExpanderContent(BuildContext context) {
    final broadcast = context.watch<BroadcastProvider>();
    final s = S.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (broadcast.serverError != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 8.0),
            child: Text(
              '${s.error}: ${broadcast.serverError}',
              style: TextStyle(color: Colors.red),
            ),
          ),
        Button(
          onPressed: () => broadcast.isServerRunning
              ? broadcast.stopServer()
              : broadcast.startServer('0.0.0.0'),
          child: Text(broadcast.isServerRunning ? s.stop : s.start),
        ),
      ],
    );
  }

  Widget buildDefaultContent(BuildContext context) {
    final broadcast = context.watch<BroadcastProvider>();
    final s = S.of(context);

    return FlyoutTarget(
      controller: _menuController,
      child: Button(
        onPressed: () => broadcast.isServerRunning
            ? broadcast.stopServer()
            : broadcast.startServer('0.0.0.0'),
        child: Text(broadcast.isServerRunning ? s.stop : s.start),
      ),
    );
  }

  @override
  void dispose() {
    _menuController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxBase(
      title: s.serverControl,
      subtitle: s.serverControlSubtitle,
      icon: Symbols.flag,
      buildExpanderContent: buildExpanderContent,
      buildDefaultContent: buildDefaultContent,
    );
  }
}
