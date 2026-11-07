import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_button.dart';

import '../utils/show_add_remote_device_dialog.dart';

class AddNeighborManuallySettingButton extends StatelessWidget {
  const AddNeighborManuallySettingButton({super.key});

  @override
  Widget build(BuildContext context) {
    return SettingsButton(
      icon: Symbols.add,
      title: S.of(context).addNeighborManually,
      subtitle: S.of(context).addNeighborManuallySubtitle,
      onPressed: () => showAddRemoteDeviceDialog(context),
    );
  }
}
