import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/api/play_play.dart';
import '../../utils/api/play_pause.dart';

class PlayPauseButton extends StatelessWidget {
  final bool disabled;
  final String state;
  final List<Shadow>? shadows;

  const PlayPauseButton({
    super.key,
    required this.disabled,
    required this.state,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: disabled
          ? null
          : state == "Playing"
              ? playPause
              : playPlay,
      icon: state == "Playing"
          ? Icon(Symbols.pause, shadows: shadows)
          : Icon(Symbols.play_arrow, shadows: shadows),
    );
  }
}
