import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

const fullNavigationBarHeight = 64.0 + 40;
const bandNavigationBarHeight = 44.0;

class NavigationBarPlaceholder extends StatelessWidget {
  const NavigationBarPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    final topInset = MediaQuery.viewInsetsOf(context).top;

    return IgnorePointer(
      ignoring: true,
      child: SmallerOrEqualTo(
        breakpoint: DeviceType.band,
        builder: (context, isBand) {
          if (isBand) {
            return SizedBox(height: bandNavigationBarHeight + topInset);
          } else {
            return SizedBox(height: fullNavigationBarHeight + topInset);
          }
        },
      ),
    );
  }
}
