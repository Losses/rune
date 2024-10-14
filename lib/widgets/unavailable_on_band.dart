import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:player/providers/responsive_providers.dart';

class UnavailableOnBand extends StatelessWidget {
  const UnavailableOnBand({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      breakpoint: DeviceType.band,
      builder: (context, isBand) {
        if (isBand) {
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
