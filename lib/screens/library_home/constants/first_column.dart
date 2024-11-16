import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';

final List<(String Function(BuildContext), String, IconData, bool)>
    bandScreenFirstColumn = [
  ((context) => S.of(context).search, '/search', Symbols.search, true),
  ((context) => S.of(context).artists, '/artists', Symbols.face, false),
  ((context) => S.of(context).albums, '/albums', Symbols.album, false),
  (
    (context) => S.of(context).playlists,
    '/playlists',
    Symbols.queue_music,
    false
  ),
  ((context) => S.of(context).mixes, '/mixes', Symbols.magic_button, false),
  ((context) => S.of(context).tracks, '/tracks', Symbols.music_note, false),
  ((context) => S.of(context).settings, '/settings', Symbols.settings, false),
];

final List<(String Function(BuildContext), String, IconData, bool)>
    smallScreenFirstColumn = [
  ((context) => S.of(context).search, '/search', Symbols.search, true),
  ((context) => S.of(context).artists, '/artists', Symbols.face, false),
  ((context) => S.of(context).albums, '/albums', Symbols.album, false),
  (
    (context) => S.of(context).playlists,
    '/playlists',
    Symbols.queue_music,
    false
  ),
  ((context) => S.of(context).mixes, '/mixes', Symbols.magic_button, false),
  ((context) => S.of(context).tracks, '/tracks', Symbols.music_note, false),
];
