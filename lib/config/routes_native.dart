import 'package:fluent_ui/fluent_ui.dart';

import '../utils/query_list.dart';
import '../utils/query_tracks_parameter.dart';

import '../routes/home.dart' as home;
import '../routes/mixes.dart' as mixes;
import '../routes/tracks.dart' as tracks;
import '../routes/search.dart' as search;
import '../routes/welcome.dart' as welcome;
import '../routes/settings.dart' as settings;
import '../routes/cover_wall.dart' as cover_wall;
import '../routes/collections.dart' as collections;
import '../routes/query_tracks.dart' as query_tracks;
import '../routes/library_home.dart' as library_home;

import '../messages/collection.pb.dart';

final Map<String, WidgetBuilder> routesNative = {
  '/welcome': (context) => const welcome.WelcomePage(),
  '/welcome/scanning': (context) => const welcome.ScanningPage(),
  '/home': (context) => const home.HomePage(),
  '/library': (context) => const library_home.LibraryHomePage(),
  '/artists': (context) => const collections.CollectionPage(
        key: ValueKey("Artists"),
        collectionType: CollectionType.Artist,
      ),
  '/artists/detail': (context) {
    final arguments = ModalRoute.of(context)?.settings.arguments;
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
        collectionType: CollectionType.Album,
      ),
  '/albums/detail': (context) {
    final arguments = ModalRoute.of(context)?.settings.arguments;
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
  '/playlists': (context) => const collections.CollectionPage(
        key: ValueKey("Playlists"),
        collectionType: CollectionType.Playlist,
      ),
  '/playlists/detail': (context) {
    final arguments = ModalRoute.of(context)?.settings.arguments;
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
        collectionType: CollectionType.Mix,
      ),
  '/mixes/detail': (context) {
    final arguments = ModalRoute.of(context)?.settings.arguments;
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
  '/settings/analysis': (context) => const settings.SettingsAnalysis(),
  '/settings/theme': (context) => const settings.SettingsTheme(),
  '/settings/playback': (context) => const settings.SettingsPlayback(),
  '/settings/about': (context) => const settings.SettingsAboutPage(),
  '/settings/mix': (context) => const settings.SettingsTestPage(),
  '/settings/media_controller': (context) =>
      const settings.SettingsMediaControllerPage(),
  '/search': (context) => const search.SearchPage(),
  '/cover_wall': (context) => const cover_wall.CoverWallPage(),
};