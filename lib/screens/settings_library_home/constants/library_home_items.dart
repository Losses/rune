import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../bindings/bindings.dart';

sealed class LibraryHomeSetting {
  const LibraryHomeSetting();
}

class LibraryHomeSettingDisabled extends LibraryHomeSetting {
  const LibraryHomeSettingDisabled();
}

class LibraryHomeSettingQuery extends LibraryHomeSetting {
  const LibraryHomeSettingQuery(this.query);

  final LibraryHomeQuery query;

  @override
  bool operator ==(Object other) =>
      other is LibraryHomeSettingQuery && other.query == query;

  @override
  int get hashCode => query.hashCode;
}

const libraryHomeDisabled = LibraryHomeSettingDisabled();

class LibraryHomeEntry {
  final LibraryHomeDomain domain;
  final IconData Function(BuildContext context) icon;
  final String Function(BuildContext context) titleBuilder;
  final String Function(BuildContext context) subtitleBuilder;
  final List<(String, LibraryHomeSetting)> Function(BuildContext context)
  optionBuilder;
  final LibraryHomeSetting defaultSetting;

  LibraryHomeEntry({
    required this.domain,
    required this.icon,
    required this.titleBuilder,
    required this.subtitleBuilder,
    required this.optionBuilder,
    required this.defaultSetting,
  });
}

List<LibraryHomeEntry> libraryHomeItems = [
  LibraryHomeEntry(
    domain: LibraryHomeDomain.artists,
    icon: (context) => Symbols.face,
    titleBuilder: (context) => S.of(context).artists,
    subtitleBuilder: (context) => S.of(context).artistsLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).newest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        S.of(context).oldest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        S.of(context).random,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.random),
        ),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: const LibraryHomeSettingQuery(
      LibraryHomeQueryArtists(value: LibraryHomeListMode.reverse),
    ),
  ),
  LibraryHomeEntry(
    domain: LibraryHomeDomain.albums,
    icon: (context) => Symbols.album,
    titleBuilder: (context) => S.of(context).albums,
    subtitleBuilder: (context) => S.of(context).albumsLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).newest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        S.of(context).oldest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        S.of(context).random,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.random),
        ),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: const LibraryHomeSettingQuery(
      LibraryHomeQueryAlbums(value: LibraryHomeListMode.reverse),
    ),
  ),
  LibraryHomeEntry(
    domain: LibraryHomeDomain.playlists,
    icon: (context) => Symbols.queue_music,
    titleBuilder: (context) => S.of(context).playlists,
    subtitleBuilder: (context) => S.of(context).playlistsLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).newest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        S.of(context).oldest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        S.of(context).random,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.random),
        ),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: libraryHomeDisabled,
  ),
  LibraryHomeEntry(
    domain: LibraryHomeDomain.tracks,
    icon: (context) => Symbols.music_note,
    titleBuilder: (context) => S.of(context).tracks,
    subtitleBuilder: (context) => S.of(context).tracksLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).newest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryTracks(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        S.of(context).oldest,
        const LibraryHomeSettingQuery(
          LibraryHomeQueryTracks(value: LibraryHomeListMode.forward),
        ),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: const LibraryHomeSettingQuery(
      LibraryHomeQueryTracks(value: LibraryHomeListMode.reverse),
    ),
  ),
  LibraryHomeEntry(
    domain: LibraryHomeDomain.liked,
    icon: (context) => Symbols.favorite,
    titleBuilder: (context) => S.of(context).liked,
    subtitleBuilder: (context) => S.of(context).likedLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).enable,
        const LibraryHomeSettingQuery(LibraryHomeQueryLiked()),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: libraryHomeDisabled,
  ),
  LibraryHomeEntry(
    domain: LibraryHomeDomain.most,
    icon: (context) => Symbols.all_inclusive,
    titleBuilder: (context) => S.of(context).mostPlayed,
    subtitleBuilder: (context) => S.of(context).mostLibraryHomeSubtitle,
    optionBuilder: (context) => [
      (
        S.of(context).enable,
        const LibraryHomeSettingQuery(LibraryHomeQueryMost()),
      ),
      (S.of(context).disable, libraryHomeDisabled),
    ],
    defaultSetting: libraryHomeDisabled,
  ),
];
