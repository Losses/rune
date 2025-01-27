import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_button.dart';

class EditDeviceInformationSettingButton extends StatelessWidget {
  const EditDeviceInformationSettingButton({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return SettingsButton(
      icon: Symbols.edit_note,
      title: S.of(context).editDeviceInformation,
      subtitle: S.of(context).editDeviceInformationSubtitle,
      onPressed: () => {},
    );
  }
}
