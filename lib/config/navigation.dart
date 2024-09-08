import '../../widgets/navigation_bar/utils/navigation_item.dart';

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
        NavigationItem('Playlist Query', '/playlists/:playlistId',
            hidden: true),
      ]),
      NavigationItem('Tracks', '/tracks'),
    ]),
    NavigationItem('Settings', '/settings', children: [
      NavigationItem('Library', '/settings/library'),
      NavigationItem('Test', '/settings/test'),
    ]),
  ]),
  NavigationItem('Search', '/search'),
];
