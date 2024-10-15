import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/cover_wall/widgets/large_screen_playing_track.dart';
import 'package:rune/screens/cover_wall/widgets/small_screen_playing_track.dart';

class PlayingTrack extends StatelessWidget {
  const PlayingTrack({super.key});

  @override
  Widget build(BuildContext context) {
    return BreakpointBuilder(
      breakpoints: const [DeviceType.phone, DeviceType.tablet],
      builder: (context, activeBreakpoint) {
        return activeBreakpoint == DeviceType.phone
            ? const SmallScreenPlayingTrack()
            : const LargeScreenPlayingTrack();
      },
    );
  }
}
