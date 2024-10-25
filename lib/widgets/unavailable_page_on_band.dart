import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/responsive_providers.dart';

class UnavailablePageOnBand extends StatelessWidget {
  const UnavailablePageOnBand({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [DeviceType.band, DeviceType.dock, DeviceType.tv],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.belt) {
          return Center(
            child: LayoutBuilder(
              builder: (context, constraint) {
                return Icon(
                  Symbols.devices,
                  size: (constraint.maxWidth * 0.8).clamp(0, 48),
                );
              },
            ),
          );
        }
        return child;
      },
    );
  }
}
