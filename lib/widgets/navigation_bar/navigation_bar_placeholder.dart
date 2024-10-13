import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

const fullNavigationBarHeight = 64.0 + 40;
const bandNavigationBarHeight = 44.0;

class NavigationBarPlaceholder extends StatelessWidget {
  const NavigationBarPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
        breakpoint: DeviceType.band,
        builder: (context, isBand) {
          if (isBand) {
            return const SizedBox(height: bandNavigationBarHeight);
          } else {
            return const SizedBox(height: fullNavigationBarHeight);
          }
        });
  }
}
