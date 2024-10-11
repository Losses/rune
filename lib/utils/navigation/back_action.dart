import 'package:fluent_ui/fluent_ui.dart';

import 'utils/escape_from_search.dart';
import 'utils/navigation_backward.dart';
import 'utils/escape_from_cover_art_wall.dart';

import 'back_intent.dart';

class BackAction extends Action<BackIntent> {
  final BuildContext context;

  BackAction(this.context);

  @override
  void invoke(covariant BackIntent intent) {
    print(1);
    if (escapeFromSearch(context)) return;
    print(2);
    if (escapeFromCoverArtWall(context)) return;

    print('!!!');
    navigationBackward(context);
  }
}
