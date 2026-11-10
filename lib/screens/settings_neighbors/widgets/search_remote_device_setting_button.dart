import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/add_library_flows.dart';
import '../../../widgets/settings/settings_button.dart';

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
    final s = S.of(context);

    return SettingsButton(
      icon: Symbols.search,
      title: s.searchNeighbors,
      subtitle: s.searchNeighborsSubtitle,
      onPressed: () => addRuneDeviceLibrary(context),
    );
  }
}
