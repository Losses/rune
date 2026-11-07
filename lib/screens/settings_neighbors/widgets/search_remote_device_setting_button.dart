import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../widgets/settings/settings_button.dart';

import '../utils/pair_with_device.dart';
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
    final s = S.of(context);

    return SettingsButton(
      icon: Symbols.search,
      title: s.searchNeighbors,
      subtitle: s.searchNeighborsSubtitle,
      onPressed: () async {
        final response = await showSearchRemoteDeviceDialog(context);
        if (response == null) return;

        final (device, correct) = response;

        if (correct == null) return;

        if (!correct) {
          if (!context.mounted) return;
          showInformationDialog(
            context: context,
            title: s.pairingFailureTitle,
            subtitle: s.pairingFailureMessage,
          );

          return;
        }

        if (!context.mounted) return;
        await pairWithDevice(context, device.fingerprint, device.ips);
      },
    );
  }
}
