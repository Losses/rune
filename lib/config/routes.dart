import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

import '../utils/query_list.dart';
import '../utils/router_extra.dart';

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

final routes = <GoRoute>[
  GoRoute(
    path: '/welcome',
    builder: (context, state) => const GoRouterModalBarrierFix(
      welcome.WelcomePage(),
    ),
  ),
  GoRoute(
    path: '/welcome/scanning',
    builder: (context, state) => const GoRouterModalBarrierFix(
      welcome.ScanningPage(),
    ),
  ),
  GoRoute(
    path: '/home',
    builder: (context, state) => const GoRouterModalBarrierFix(
      home.HomePage(),
    ),
  ),
  GoRoute(
    path: '/library',
    builder: (context, state) => const GoRouterModalBarrierFix(
      library_home.LibraryHomePage(),
    ),
  ),
  GoRoute(
    path: '/artists',
    builder: (context, state) => const GoRouterModalBarrierFix(
      collections.CollectionPage(
        key: ValueKey("Artists"),
        collectionType: CollectionType.Artist,
      ),
    ),
  ),
  GoRoute(
    path: '/artists/:artistId',
    builder: (context, state) => GoRouterModalBarrierFix(
      query_tracks.QueryTracksPage(
        queries: QueryList(
          [("lib::artist", state.pathParameters['artistId'] ?? "0")],
        ),
        title: state.extra is QueryTracksExtra
            ? (state.extra as QueryTracksExtra).title
            : null,
        mode: 99,
      ),
    ),
  ),
  GoRoute(
    path: '/albums',
    builder: (context, state) => const GoRouterModalBarrierFix(
      collections.CollectionPage(
        key: ValueKey("Albums"),
        collectionType: CollectionType.Album,
      ),
    ),
  ),
  GoRoute(
    path: '/albums/:albumId',
    builder: (context, state) => GoRouterModalBarrierFix(
      query_tracks.QueryTracksPage(
        queries: QueryList(
          [("lib::album", state.pathParameters['albumId'] ?? "0")],
        ),
        title: state.extra is QueryTracksExtra
            ? (state.extra as QueryTracksExtra).title
            : null,
        mode: 99,
      ),
    ),
  ),
  GoRoute(
    path: '/playlists',
    builder: (context, state) => const GoRouterModalBarrierFix(
      collections.CollectionPage(
        key: ValueKey("Playlists"),
        collectionType: CollectionType.Playlist,
      ),
    ),
  ),
  GoRoute(
    path: '/playlists/:playlistId',
    builder: (context, state) => GoRouterModalBarrierFix(
      query_tracks.QueryTracksPage(
        queries: QueryList(
          [("lib::playlist", state.pathParameters['playlistId'] ?? "0")],
        ),
        title: state.extra is QueryTracksExtra
            ? (state.extra as QueryTracksExtra).title
            : null,
        mode: 99,
      ),
    ),
  ),
  GoRoute(
    path: '/mixes',
    builder: (context, state) => const GoRouterModalBarrierFix(
      collections.CollectionPage(
        key: ValueKey("Mixes"),
        collectionType: CollectionType.Mix,
      ),
    ),
  ),
  GoRoute(
    path: '/mixes/:mixId',
    builder: (context, state) => GoRouterModalBarrierFix(
      mixes.MixTrackesPage(
        mixId: int.parse(state.pathParameters['mixId'] ?? "0"),
        title: state.extra is QueryTracksExtra
            ? (state.extra as QueryTracksExtra).title
            : null,
      ),
    ),
  ),
  GoRoute(
    path: '/tracks',
    builder: (context, state) => const GoRouterModalBarrierFix(
      tracks.TracksPage(),
    ),
  ),
  GoRoute(
    path: '/settings',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsHomePage(),
    ),
  ),
  GoRoute(
    path: '/settings/library',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsLibraryPage(),
    ),
  ),
  GoRoute(
    path: '/settings/theme',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsTheme(),
    ),
  ),
  GoRoute(
    path: '/settings/playback',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsPlayback(),
    ),
  ),
  GoRoute(
    path: '/settings/about',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsAboutPage(),
    ),
  ),
  GoRoute(
    path: '/settings/mix',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsTestPage(),
    ),
  ),
  GoRoute(
    path: '/settings/media_controller',
    builder: (context, state) => const GoRouterModalBarrierFix(
      settings.SettingsMediaControllerPage(),
    ),
  ),
  GoRoute(
    path: '/search',
    builder: (context, state) => const GoRouterModalBarrierFix(
      search.SearchPage(),
    ),
  ),
  GoRoute(
    path: '/cover_wall',
    builder: (context, state) => const GoRouterModalBarrierFix(
      cover_wall.CoverWallPage(),
    ),
  ),
];

class GoRouterModalBarrierFix extends StatelessWidget {
  const GoRouterModalBarrierFix(
    this.child, {
    super.key,
  });

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        ModalBarrier(
          dismissible: true,
          color: Colors.transparent,
          onDismiss: () {},
        ),
        child,
      ],
    );
  }
}
