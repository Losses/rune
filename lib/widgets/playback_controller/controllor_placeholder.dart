import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/providers/responsive_providers.dart';

import '../../widgets/playback_controller/constants/playback_controller_height.dart';

class ControllerPlaceholder extends StatelessWidget {
  const ControllerPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    final bottomInset = MediaQuery.viewInsetsOf(context).bottom;

    return IgnorePointer(
      ignoring: true,
      child: SmallerOrEqualTo(
        breakpoint: DeviceType.band,
        builder: (context, isBand) {
          if (isBand) {
            return LayoutBuilder(builder: (context, constraints) {
              return SizedBox(height: constraints.maxWidth / 3 + bottomInset);
            });
          } else {
            return SizedBox(height: playbackControllerHeight + bottomInset);
          }
        },
      ),
    );
  }
}
