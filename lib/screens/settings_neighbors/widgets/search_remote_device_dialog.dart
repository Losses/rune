import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../bindings/bindings.dart';

import 'discovered_devices_list.dart';

class SearchRemoteDeviceDialog extends StatefulWidget {
  final void Function((DiscoveredDeviceMessage, bool?)?) $close;
  final Completer<(DiscoveredDeviceMessage, bool?)?> completer;

  const SearchRemoteDeviceDialog({
    super.key,
    required this.$close,
    required this.completer,
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
                  onPaired: () => widget.$close(null),
                  onAnswered: (device, correct) =>
                      widget.completer.complete((device, correct)),
                ),
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
