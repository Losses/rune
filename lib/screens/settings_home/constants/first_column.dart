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
    (context) => S.of(context).neighbors,
    '/settings/neighbors',
    Symbols.group,
    false
  ),
  ((context) => S.of(context).server, '/settings/server', Symbols.p2p, false),
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
  (
    (context) => S.of(context).home,
    '/settings/library_home',
    Symbols.home,
    false
  ),
  ((context) => S.of(context).system, '/settings/system', Symbols.radio, false),
  (
    (context) => S.of(context).log,
    '/settings/log',
    Symbols.receipt_long,
    false
  ),
  ((context) => S.of(context).about, '/settings/about', Symbols.info, false),
];
