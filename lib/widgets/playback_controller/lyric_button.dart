import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/router_path.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_lyrics.dart';

import '../rune_clickable.dart';

void showLyrics() {
  final path = $router.path;
  if (path == '/lyrics') {
    escapeFromLyrics();
  } else {
    if (path == '/cover_wall') {
      $replace('/lyrics');
    } else {
      $push('/lyrics');
    }
  }
}

class LyricsButton extends StatelessWidget {
  final List<Shadow>? shadows;

  const LyricsButton({
    super.key,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: () => showLyrics(),
      child: Icon(
        Symbols.lyrics,
        shadows: shadows,
      ),
    );
  }
}
