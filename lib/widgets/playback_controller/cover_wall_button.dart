import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/router_path.dart';
import '../../utils/router/navigation.dart';

void showCoverArtWall() {
  final path = $routerPath.path;
  if (path == "/cover_wall") {
    if ($canPop()) {
      $pop();
    }
  } else {
    $push("/cover_wall");
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
    return IconButton(
      onPressed: () => showCoverArtWall(),
      icon: Icon(
        Symbols.photo_frame,
        shadows: shadows,
      ),
    );
  }
}
