import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller/constants/playback_controller_height.dart';

class PlaybackPlaceholder extends StatelessWidget {
  const PlaybackPlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return const SizedBox(height: playbackControllerHeight);
  }
}
