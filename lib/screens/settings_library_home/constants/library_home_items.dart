import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';

class LibraryHomeEntry {
  final String id;
  final IconData Function(BuildContext context) icon;
  final String Function(BuildContext context) titleBuilder;
  final String Function(BuildContext context) subtitleBuilder;
  final List<(String, String)> Function(BuildContext context) optionBuilder;
  final String defaultValue;

  LibraryHomeEntry({
    required this.id,
    required this.icon,
    required this.titleBuilder,
    required this.subtitleBuilder,
    required this.optionBuilder,
    required this.defaultValue,
  });
}

List<LibraryHomeEntry> libraryHomeItems = [
  LibraryHomeEntry(
    id: 'artists',
    icon: (context) => Symbols.face,
    titleBuilder: (context) => S.of(context).artists,
    subtitleBuilder: (context) => S.of(context).previousSubtitle,
    optionBuilder: (context) => [
      (S.of(context).newest, 'newest'),
      (S.of(context).oldest, 'oldest'),
      (S.of(context).random, 'random'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'newest',
  ),
  LibraryHomeEntry(
    id: 'albums',
    icon: (context) => Symbols.album,
    titleBuilder: (context) => S.of(context).albums,
    subtitleBuilder: (context) => S.of(context).previousSubtitle,
    optionBuilder: (context) => [
      (S.of(context).newest, 'newest'),
      (S.of(context).oldest, 'oldest'),
      (S.of(context).random, 'random'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'newest',
  ),
  LibraryHomeEntry(
    id: 'playlists',
    icon: (context) => Symbols.queue_music,
    titleBuilder: (context) => S.of(context).playlists,
    subtitleBuilder: (context) => S.of(context).nextSubtitle,
    optionBuilder: (context) => [
      (S.of(context).newest, 'newest'),
      (S.of(context).oldest, 'oldest'),
      (S.of(context).random, 'random'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'newest',
  ),
  LibraryHomeEntry(
    id: 'tracks',
    icon: (context) => Symbols.music_note,
    titleBuilder: (context) => S.of(context).tracks,
    subtitleBuilder: (context) => S.of(context).nextSubtitle,
    optionBuilder: (context) => [
      (S.of(context).newest, 'newest'),
      (S.of(context).oldest, 'oldest'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'newest',
  ),
  LibraryHomeEntry(
    id: 'liked',
    icon: (context) => Symbols.favorite,
    titleBuilder: (context) => S.of(context).liked,
    subtitleBuilder: (context) => S.of(context).nextSubtitle,
    optionBuilder: (context) => [
      (S.of(context).enable, 'enable'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'disable',
  ),
  LibraryHomeEntry(
    id: 'most',
    icon: (context) => Symbols.all_inclusive,
    titleBuilder: (context) => S.of(context).mostPlayed,
    subtitleBuilder: (context) => S.of(context).nextSubtitle,
    optionBuilder: (context) => [
      (S.of(context).enable, 'enable'),
      (S.of(context).disable, 'disable'),
    ],
    defaultValue: 'disable',
  ),
];
