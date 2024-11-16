import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';

final List<(String Function(BuildContext), String, IconData, bool)>
    firstColumn = [
  (
    (context) => S.of(context).library,
    '/settings/library',
    Symbols.video_library,
    false
  ),
  (
    (context) => S.of(context).analysis,
    '/settings/analysis',
    Symbols.grain,
    false
  ),
  (
    (context) => S.of(context).playback,
    '/settings/playback',
    Symbols.playlist_add_check_circle,
    false
  ),
  (
    (context) => S.of(context).theme,
    '/settings/theme',
    Symbols.format_paint,
    false
  ),
  (
    (context) => S.of(context).language,
    '/settings/language',
    Symbols.language,
    false
  ),
  (
    (context) => S.of(context).controller,
    '/settings/media_controller',
    Symbols.tune,
    false
  ),
  ((context) => S.of(context).about, '/settings/about', Symbols.info, false),
];
