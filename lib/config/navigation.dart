import '../../widgets/navigation_bar/utils/navigation_item.dart';

final List<NavigationItem> navigationItems = [
  NavigationItem('Rune', '/home', tappable: false, children: [
    NavigationItem('Library', '/library', children: [
      NavigationItem('Search', '/search', zuneOnly: true),
      NavigationItem('Artists', '/artists', children: [
        NavigationItem(
          'Artist Query',
          '/artists/:artistId',
          hidden: true,
        ),
      ]),
      NavigationItem('Albums', '/albums', children: [
        NavigationItem(
          'Artist Query',
          '/albums/:albumId',
          hidden: true,
        ),
      ]),
      NavigationItem('Playlists', '/playlists', children: [
        NavigationItem(
          'Playlist Query',
          '/playlists/:playlistId',
          hidden: true,
        ),
      ]),
      NavigationItem('Mixes', '/mixes', children: [
        NavigationItem(
          'Mix Query',
          '/mixes/:mixId',
          hidden: true,
        ),
      ]),
      NavigationItem('Tracks', '/tracks'),
    ]),
    NavigationItem('Settings', '/settings', children: [
      NavigationItem('Library', '/settings/library'),
      NavigationItem('Controller', '/settings/media_controller'),
      NavigationItem('About', '/settings/about'),
      NavigationItem('Test', '/settings/mix'),
    ]),
  ]),
  // We must keep this here to make page transition parsing works correctly!
  NavigationItem('Search', '/search'),
];
