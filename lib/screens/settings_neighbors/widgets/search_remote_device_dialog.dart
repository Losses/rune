import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../bindings/bindings.dart';

import 'discovered_devices_list.dart';

class SearchRemoteDeviceDialog extends StatelessWidget {
  /// Closes the search with the chosen device, or null after cancellation.
  final void Function(DiscoveredDeviceMessage?) $close;

  /// Devices to show without searching, for a preview.
  final List<DiscoveredDeviceMessage>? initialDevices;

  const SearchRemoteDeviceDialog({
    super.key,
    required this.$close,
    this.initialDevices,
  });

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    void close(DiscoveredDeviceMessage? device) {
      if (ModalRoute.of(context)?.isCurrent == true) $close(device);
    }

    return NoShortcuts(
      ContentDialog(
        style: ContentDialogThemeData.standard(
          FluentTheme.of(
            context,
          ),
        ).merge(
          FluentTheme.of(context).dialogTheme.merge(
                ContentDialogThemeData(
                  padding: EdgeInsets.fromLTRB(0, 0, 0, 12),
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
              child: Padding(
                padding: EdgeInsetsDirectional.symmetric(horizontal: 12),
                child: DiscoveredDevicesList(
                  onSelected: close,
                  initialDevices: initialDevices,
                ),
              ),
            )
          ],
        ),
        actions: [
          Button(
            onPressed: () => close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
