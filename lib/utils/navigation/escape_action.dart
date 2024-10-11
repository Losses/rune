import 'package:fluent_ui/fluent_ui.dart';

import 'utils/escape_from_search.dart';
import 'utils/escape_from_cover_art_wall.dart';

import 'escape_intent.dart';

class EscapeAction extends Action<EscapeIntent> {
  final BuildContext context;

  EscapeAction(this.context);

  @override
  void invoke(covariant EscapeIntent intent) {
    if (escapeFromSearch(context)) return;
    if (escapeFromCoverArtWall(context)) return;
  }
}
