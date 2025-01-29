import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_button.dart';

import '../utils/show_search_remote_device_dialog.dart';

class SearchRemoteDeviceSettingButton extends StatelessWidget {
  const SearchRemoteDeviceSettingButton({
    super.key,
    required this.tryClose,
    required this.navigateIfFailed,
  });

  final bool tryClose;
  final bool navigateIfFailed;

  @override
  Widget build(BuildContext context) {
    return SettingsButton(
      icon: Symbols.search,
      title: S.of(context).searchNeighbors,
      subtitle: S.of(context).searchNeighborsSubtitle,
      onPressed: () => showSearchRemoteDeviceDialog(context),
    );
  }
}
