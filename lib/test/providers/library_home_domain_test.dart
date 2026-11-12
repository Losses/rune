import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_home.dart';
import '../../screens/settings_library_home/constants/library_home_items.dart';

void main() {
  test('every stored home setting maps to its query and back', () {
    final storedSettings = <(LibraryHomeDomain, String, LibraryHomeSetting)>[
      (
        LibraryHomeDomain.artists,
        'newest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        LibraryHomeDomain.artists,
        'oldest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        LibraryHomeDomain.artists,
        'random',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryArtists(value: LibraryHomeListMode.random),
        ),
      ),
      (LibraryHomeDomain.artists, 'disable', libraryHomeDisabled),
      (
        LibraryHomeDomain.albums,
        'newest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        LibraryHomeDomain.albums,
        'oldest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        LibraryHomeDomain.albums,
        'random',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryAlbums(value: LibraryHomeListMode.random),
        ),
      ),
      (LibraryHomeDomain.albums, 'disable', libraryHomeDisabled),
      (
        LibraryHomeDomain.playlists,
        'newest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        LibraryHomeDomain.playlists,
        'oldest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.forward),
        ),
      ),
      (
        LibraryHomeDomain.playlists,
        'random',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryPlaylists(value: LibraryHomeListMode.random),
        ),
      ),
      (LibraryHomeDomain.playlists, 'disable', libraryHomeDisabled),
      (
        LibraryHomeDomain.tracks,
        'newest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryTracks(value: LibraryHomeListMode.reverse),
        ),
      ),
      (
        LibraryHomeDomain.tracks,
        'oldest',
        const LibraryHomeSettingQuery(
          LibraryHomeQueryTracks(value: LibraryHomeListMode.forward),
        ),
      ),
      (LibraryHomeDomain.tracks, 'disable', libraryHomeDisabled),
      (
        LibraryHomeDomain.liked,
        'enable',
        const LibraryHomeSettingQuery(LibraryHomeQueryLiked()),
      ),
      (LibraryHomeDomain.liked, 'disable', libraryHomeDisabled),
      (
        LibraryHomeDomain.most,
        'enable',
        const LibraryHomeSettingQuery(LibraryHomeQueryMost()),
      ),
      (LibraryHomeDomain.most, 'disable', libraryHomeDisabled),
    ];

    for (final (domain, value, expectedSetting) in storedSettings) {
      final serialized = '${domain.name}::$value';
      final restored = LibraryHomeEntryWithValue.deserialize(
        serialized,
        libraryHomeItems,
      );

      expect(restored?.domain, domain);
      expect(restored?.setting, expectedSetting, reason: serialized);
      expect(restored?.serialize(), serialized);
    }
  });

  test('home query definitions carry the wire enum', () {
    expect(libraryHomeItems.map((entry) => entry.domain), [
      LibraryHomeDomain.artists,
      LibraryHomeDomain.albums,
      LibraryHomeDomain.playlists,
      LibraryHomeDomain.tracks,
      LibraryHomeDomain.liked,
      LibraryHomeDomain.most,
    ]);
  });
}
