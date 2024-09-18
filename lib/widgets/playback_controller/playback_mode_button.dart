import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/status.dart';
import '../../messages/playback.pb.dart';
import '../../widgets/playback_controller/utils/playback_mode.dart';

IconData modeToIcon(PlaybackMode mode) {
  switch (mode) {
    case PlaybackMode.sequential:
      return Symbols.east;
    case PlaybackMode.repeatAll:
      return Symbols.repeat;
    case PlaybackMode.repeatOne:
      return Symbols.repeat_one;
    case PlaybackMode.shuffle:
      return Symbols.shuffle;
  }
}

String modeToLabel(PlaybackMode mode) {
  switch (mode) {
    case PlaybackMode.sequential:
      return "Sequential";
    case PlaybackMode.repeatAll:
      return "Repeat All";
    case PlaybackMode.repeatOne:
      return "Repeat One";
    case PlaybackMode.shuffle:
      return "Shuffle";
  }
}

int modeToInt(PlaybackMode mode) {
  switch (mode) {
    case PlaybackMode.sequential:
      return 0;
    case PlaybackMode.repeatAll:
      return 1;
    case PlaybackMode.repeatOne:
      return 2;
    case PlaybackMode.shuffle:
      return 3;
  }
}

PlaybackMode nextMode(PlaybackMode mode) {
  switch (mode) {
    case PlaybackMode.sequential:
      return PlaybackMode.repeatAll;
    case PlaybackMode.repeatAll:
      return PlaybackMode.repeatOne;
    case PlaybackMode.repeatOne:
      return PlaybackMode.shuffle;
    case PlaybackMode.shuffle:
      return PlaybackMode.sequential;
  }
}

class PlaybackModeButton extends StatelessWidget {
  const PlaybackModeButton({super.key});

  @override
  Widget build(BuildContext context) {
    return Consumer<PlaybackStatusProvider>(
      builder: (context, playbackStatusProvider, child) {
        final PlaybackMode currentMode = PlaybackModeExtension.fromValue(
            playbackStatusProvider.playbackStatus?.playbackMode ?? 0);

        return IconButton(
          onPressed: () {
            final next = nextMode(currentMode);
            SetPlaybackModeRequest(mode: next.toValue())
                .sendSignalToRust(); // GENERATED
          },
          icon: Icon(modeToIcon(currentMode)),
        );
      },
    );
  }
}
