import '../widgets/navigation_bar.dart';

final List<NavigationItem> navigationItems = [
  NavigationItem('Rune', '/home', tappable: false, children: [
    NavigationItem('Library', '/library', children: [
      NavigationItem('Artists', '/artists', children: [
        NavigationItem('Artist Query', '/artists/:artistId', hidden: true),
      ]),
      NavigationItem('Albums', '/albums', children: [
        NavigationItem('Artist Query', '/albums/:albumId', hidden: true),
      ]),
      NavigationItem('Playlists', '/playlists', children: [
        NavigationItem('Playlist Query', '/playlists/:playlistId', hidden: true),
      ]),
      NavigationItem('Tracks', '/tracks'),
    ]),
    NavigationItem('Settings', '/settings'),
  ]),
  NavigationItem('Search', '/search'),
];
