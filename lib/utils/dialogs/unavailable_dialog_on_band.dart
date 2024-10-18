import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/responsive_providers.dart';

class UnavailableDialogOnBand extends StatelessWidget {
  const UnavailableDialogOnBand({
    super.key,
    required this.child,
    this.icon
  });

  final IconData? icon;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      breakpoint: DeviceType.band,
      builder: (context, isBand) {
        if (isBand) {
          return Column(
            mainAxisSize: MainAxisSize.max,
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              LayoutBuilder(
                builder: (context, constraint) {
                  return IconButton(
                    icon: Icon(
                      icon ?? Symbols.devices,
                      size: (constraint.maxWidth * 0.8).clamp(0, 48),
                    ),
                    onPressed: () => Navigator.pop(context, null),
                  );
                },
              ),
            ],
          );
        }

        return child;
      },
    );
  }
}
