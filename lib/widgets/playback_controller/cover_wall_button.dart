import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../router.dart';

void showCoverArtWall(BuildContext context) {
  final RouteMatch lastMatch = router.routerDelegate.currentConfiguration.last;
  final RouteMatchList matchList = lastMatch is ImperativeRouteMatch
      ? lastMatch.matches
      : router.routerDelegate.currentConfiguration;
  final String location = matchList.uri.toString();

  if (location == "/cover_wall") {
    if (context.canPop()) {
      context.pop();
    }
  } else {
    context.push("/cover_wall");
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
      onPressed: () => showCoverArtWall(context),
      icon: Icon(
        Symbols.photo_frame,
        shadows: shadows,
      ),
    );
  }
}
