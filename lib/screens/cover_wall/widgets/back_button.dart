import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../widgets/playback_controller/cover_wall_button.dart';
import '../../../providers/responsive_providers.dart';

class BackButton extends StatelessWidget {
  const BackButton({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.mobile,
      builder: (_, isTrue) {
        if (!isTrue) return Container();

        return Padding(
          padding: const EdgeInsets.only(top: 16, left: 16),
          child: IconButton(
            icon: const Icon(
              Symbols.arrow_back,
              size: 24,
            ),
            onPressed: () {
              showCoverArtWall(context);
            },
          ),
        );
      },
    );
  }
}
