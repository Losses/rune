import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/providers/responsive_providers.dart';

class ResponsiveDialogActions extends StatelessWidget {
  const ResponsiveDialogActions(this.buttonA, this.buttonB, {super.key});

  final Widget buttonA;
  final Widget buttonB;

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
        deviceType: DeviceType.zune,
        builder: (context, isZune) {
          if (!isZune) {
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Expanded(child: buttonA),
                const SizedBox(width: 8),
                Expanded(child: buttonB),
              ],
            );
          } else {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                buttonA,
                const SizedBox(height: 8),
                buttonB,
              ],
            );
          }
        });
  }
}
