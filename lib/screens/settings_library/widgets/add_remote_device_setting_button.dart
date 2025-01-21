import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';

import '../utils/show_add_remote_device_dialog.dart';

import 'settings_button.dart';

class AddRemoteDeviceSettingButton extends StatelessWidget {
  const AddRemoteDeviceSettingButton({
    super.key,
    required this.tryClose,
    required this.navigateIfFailed,
  });

  final bool tryClose;
  final bool navigateIfFailed;

  @override
  Widget build(BuildContext context) {
    return SettingsButton(
      icon: Symbols.conversion_path,
      title: S.of(context).addRemoteDevice,
      subtitle: S.of(context).addRemoteDeviceSubtitle,
      onPressed: () => showAddRemoteDeviceDialog(navigateIfFailed, context),
    );
  }
}
