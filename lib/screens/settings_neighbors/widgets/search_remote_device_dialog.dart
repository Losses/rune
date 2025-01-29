import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';

import 'discovered_devices_list.dart';

class SearchRemoteDeviceDialog extends StatefulWidget {
  final void Function(void) $close;

  const SearchRemoteDeviceDialog({
    super.key,
    required this.$close,
  });

  @override
  SearchRemoteDeviceDialogState createState() =>
      SearchRemoteDeviceDialogState();
}

class SearchRemoteDeviceDialogState extends State<SearchRemoteDeviceDialog> {
  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              constraints: BoxConstraints(minHeight: 420),
              child: DiscoveredDevicesList(),
            )
          ],
        ),
        actions: [
          Button(
            onPressed: () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
