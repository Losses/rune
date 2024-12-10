import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/band_screen_fallback_button.dart';
import '../../providers/responsive_providers.dart';

class UnavailableDialogOnBand extends StatelessWidget {
  const UnavailableDialogOnBand({
    super.key,
    required this.child,
    this.icon,
    required this.$close,
  });

  final IconData? icon;
  final Widget child;
  final void Function(Null) $close;

  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.belt,
        DeviceType.tv
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.belt ||
            activeBreakpoint == DeviceType.dock) {
          return Center(
            child: LayoutBuilder(
              builder: (context, constraint) {
                return BandScreenFallbackButton(
                  icon: Icon(
                    icon ?? Symbols.devices,
                    size: (min(constraint.maxWidth, constraint.maxHeight) * 0.8)
                        .clamp(0, 48),
                  ),
                  onPressed: () => $close(null),
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
