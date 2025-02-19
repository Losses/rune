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
        style: ContentDialogThemeData.standard(
          FluentTheme.of(
            context,
          ),
        ).merge(
          FluentTheme.of(context).dialogTheme.merge(
                ContentDialogThemeData(
                  padding: EdgeInsets.only(top: 0),
                ),
              ),
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(128.0),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  ProgressBar(),
                  SizedBox(height: 20),
                ],
              ),
            ),
            SizedBox(
              height: 420,
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
