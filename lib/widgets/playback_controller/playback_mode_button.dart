import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/api/play_mode.dart';
import '../../utils/settings_manager.dart';
import '../../widgets/playback_controller/utils/playback_mode.dart';
import '../../providers/status.dart';

import '../rune_clickable.dart';

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

String modeToLabel(BuildContext context, PlaybackMode mode) {
  switch (mode) {
    case PlaybackMode.sequential:
      return S.of(context).sequential;
    case PlaybackMode.repeatAll:
      return S.of(context).repeatAll;
    case PlaybackMode.repeatOne:
      return S.of(context).repeatOne;
    case PlaybackMode.shuffle:
      return S.of(context).shuffle;
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
  final List<Shadow>? shadows;
  const PlaybackModeButton({
    required this.shadows,
    super.key,
  });

  Future<PlaybackMode> getNextEnabledMode(PlaybackMode currentMode) async {
    // Retrieve disabled modes
    List<dynamic>? storedDisabledModes = await SettingsManager()
        .getValue<List<dynamic>>('disabled_playback_modes');
    List<PlaybackMode> disabledModes = storedDisabledModes != null
        ? storedDisabledModes
            .map((index) => PlaybackMode.values[index])
            .toList()
        : [];

    // Check if all modes are disabled
    if (disabledModes.length >= PlaybackMode.values.length) {
      return PlaybackMode.sequential; // Default to sequential mode
    }

    PlaybackMode next = currentMode;

    // Find the next enabled mode
    do {
      next = nextMode(next);
    } while (disabledModes.contains(next));

    return next;
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<PlaybackStatusProvider>(
      builder: (context, playbackStatusProvider, child) {
        final PlaybackMode currentMode = PlaybackModeExtension.fromValue(
          playbackStatusProvider.playbackStatus.playbackMode ?? 0,
        );

        return RuneClickable(
          onPressed: () async {
            final next = await getNextEnabledMode(currentMode);
            playMode(next.toValue());
          },
          child: Icon(
            modeToIcon(currentMode),
            shadows: shadows,
          ),
        );
      },
    );
  }
}
