import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/router_path.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_cover_art_wall.dart';

import '../rune_clickable.dart';

void showCoverArtWall() {
  final path = $router.path;
  if (path == '/cover_wall') {
    escapeFromCoverArtWall();
  } else {
    if (path == '/lyrics') {
      $replace('/cover_wall');
    } else {
      $push('/cover_wall');
    }
  }
}

class CoverWallButton extends StatelessWidget {
  final List<Shadow>? shadows;

  const CoverWallButton({
    super.key,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: () => showCoverArtWall(),
      child: Icon(
        Symbols.photo_frame,
        shadows: shadows,
      ),
    );
  }
}
