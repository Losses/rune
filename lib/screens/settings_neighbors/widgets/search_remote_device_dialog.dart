import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../messages/all.dart';

import 'discovered_devices_list.dart';

class SearchRemoteDeviceDialog extends StatefulWidget {
  final void Function(void) $close;
  final void Function(DiscoveredDeviceMessage, bool?) onAnswered;

  const SearchRemoteDeviceDialog({
    super.key,
    required this.$close,
    required this.onAnswered,
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
                  padding: EdgeInsets.fromLTRB(12, 0, 12, 12),
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
              child: DiscoveredDevicesList(
                onPaired: () => widget.$close(null),
                onAnswered: widget.onAnswered,
              ),
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
