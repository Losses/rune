import 'package:fluent_ui/fluent_ui.dart';

import 'utils/escape_from_search.dart';
import 'utils/navigation_backward.dart';
import 'utils/escape_from_cover_art_wall.dart';

import 'back_intent.dart';

class BackAction extends Action<BackIntent> {
  BackAction();

  @override
  void invoke(covariant BackIntent intent) {
    if (escapeFromSearch()) return;
    if (escapeFromCoverArtWall()) return;

    navigateBackwardWithPop();
  }
}
