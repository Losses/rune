import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/cover_wall/widgets/large_screen_playing_track.dart';
import '../../../screens/cover_wall/widgets/small_screen_playing_track.dart';
import '../../../providers/responsive_providers.dart';

class PlayingTrack extends StatelessWidget {
  const PlayingTrack({super.key});

  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [DeviceType.car, DeviceType.phone, DeviceType.tablet],
      builder: (context, activeBreakpoint) {
        return activeBreakpoint == DeviceType.phone || activeBreakpoint == DeviceType.car
            ? const SmallScreenPlayingTrack()
            : const LargeScreenPlayingTrack();
      },
    );
  }
}
