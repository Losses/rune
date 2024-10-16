import 'package:fluent_ui/fluent_ui.dart';

import '../providers/responsive_providers.dart';

class SettingsPagePadding extends StatelessWidget {
  const SettingsPagePadding({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return BreakpointBuilder(
        breakpoints: const [DeviceType.phone, DeviceType.tv],
        builder: (context, deviceType) {
          return Padding(
            padding: EdgeInsets.symmetric(
              horizontal: deviceType == DeviceType.phone ? 2 : 24,
            ),
            child: child,
          );
        });
  }
}
