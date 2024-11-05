import 'package:fluent_ui/fluent_ui.dart';

import 'utils/escape_from_search.dart';
import 'utils/escape_from_cover_art_wall.dart';

class EscapeAction extends Action<DismissIntent> {
  EscapeAction();

  @override
  void invoke(covariant DismissIntent intent) {
    print('INVOKING');
    if (escapeFromSearch()) return;
    if (escapeFromCoverArtWall()) return;
  }
}
