import 'package:fluent_ui/fluent_ui.dart';

import 'utils/escape_from_search.dart';
import 'utils/escape_from_cover_art_wall.dart';

import 'escape_intent.dart';

class EscapeAction extends Action<EscapeIntent> {
  EscapeAction();

  @override
  void invoke(covariant EscapeIntent intent) {
    if (escapeFromSearch()) return;
    if (escapeFromCoverArtWall()) return;
  }
}
