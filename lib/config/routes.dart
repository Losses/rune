import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/material.dart';

import '../utils/query_list.dart';
import '../utils/router/query_tracks_parameter.dart';

import '../routes/home.dart' as home;
import '../routes/mixes.dart' as mixes;
import '../routes/tracks.dart' as tracks;
import '../routes/lyrics.dart' as lyrics;
import '../routes/search.dart' as search;
import '../routes/welcome.dart' as welcome;
import '../routes/settings.dart' as settings;
import '../routes/cover_wall.dart' as cover_wall;
import '../routes/collections.dart' as collections;
import '../routes/query_tracks.dart' as query_tracks;
import '../routes/library_home.dart' as library_home;

import '../bindings/bindings.dart';

final Map<String, WidgetBuilder> routes = {
  '/': (context) => const home.HomePage(),
  '/scanning': (context) => const welcome.ScanningPage(),
  '/library': (context) => const library_home.LibraryHomePage(),
  '/artists': (context) => const collections.CollectionPage(
        key: ValueKey("Artists"),
        collectionType: CollectionType.artist,
      ),
  '/artists/detail': (context) {
    final arguments = getQueryTracksParameter();
    if (arguments is! QueryTracksParameter) {
      throw "Invalid router parameters";
    }

    return query_tracks.QueryTracksPage(
      queries: QueryList(
        [("lib::artist", arguments.id.toString())],
      ),
      title: arguments.title,
      mode: 99,
    );
  },
  '/albums': (context) => const collections.CollectionPage(
        key: ValueKey("Albums"),
        collectionType: CollectionType.album,
      ),
  '/albums/detail': (context) {
    final arguments = getQueryTracksParameter();
    if (arguments is! QueryTracksParameter) {
      throw "Invalid router parameters";
    }

    return query_tracks.QueryTracksPage(
      queries: QueryList(
        [
          ("lib::album", arguments.id.toString()),
          ("sort::track_number", "true")
        ],
      ),
      title: arguments.title,
      mode: 99,
    );
  },
  '/genres': (context) => const collections.CollectionPage(
        key: ValueKey("Genres"),
        collectionType: CollectionType.genre,
      ),
  '/genres/detail': (context) {
    final arguments = getQueryTracksParameter();
    if (arguments is! QueryTracksParameter) {
      throw "Invalid router parameters";
    }

    return query_tracks.QueryTracksPage(
      queries: QueryList(
        [
          ("lib::genre", arguments.id.toString()),
          ("sort::track_number", "true")
        ],
      ),
      title: arguments.title,
      mode: 99,
    );
  },
  '/playlists': (context) => const collections.CollectionPage(
        key: ValueKey("Playlists"),
        collectionType: CollectionType.playlist,
      ),
  '/playlists/detail': (context) {
    final arguments = getQueryTracksParameter();
    if (arguments is! QueryTracksParameter) {
      throw "Invalid router parameters";
    }

    return query_tracks.QueryTracksPage(
      queries: QueryList(
        [("lib::playlist", arguments.id.toString())],
      ),
      title: arguments.title,
      mode: 99,
    );
  },
  '/mixes': (context) => const collections.CollectionPage(
        key: ValueKey("Mixes"),
        collectionType: CollectionType.mix,
      ),
  '/mixes/detail': (context) {
    final arguments = getQueryTracksParameter();
    if (arguments is! QueryTracksParameter) {
      throw "Invalid router parameters";
    }

    return mixes.MixTrackesPage(
      mixId: arguments.id,
      title: arguments.title,
    );
  },
  '/tracks': (context) => const tracks.TracksPage(),
  '/settings': (context) => const settings.SettingsHomePage(),
  '/settings/library': (context) => const settings.SettingsLibraryPage(),
  '/settings/neighbors': (context) => const settings.SettingsNeighborsPage(),
  '/settings/server': (context) => const settings.SettingsServerPage(),
  '/settings/system': (context) => const settings.SettingsAnalysis(),
  '/settings/theme': (context) => const settings.SettingsTheme(),
  '/settings/language': (context) => const settings.SettingsLanguage(),
  '/settings/playback': (context) => const settings.SettingsPlayback(),
  '/settings/log': (context) => const settings.SettingsLogPage(),
  '/settings/about': (context) => const settings.SettingsAboutPage(),
  // '/settings/test': (context) => const settings.SettingsTestPage(),
  '/settings/library_home': (context) => const settings.SettingsLibraryHome(),
  '/settings/media_controller': (context) =>
      const settings.SettingsMediaControllerPage(),
  '/settings/laboratory': (context) => const settings.SettingsLaboratory(),
  '/search': (context) => const search.SearchPage(),
  '/cover_wall': (context) => const cover_wall.CoverWallPage(),
  '/lyrics': (context) => const lyrics.LyricsPage(),
};
