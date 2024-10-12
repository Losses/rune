import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';

import '../utils/navigation/navigation_item.dart';

final List<NavigationItem> navigationItems = [
  NavigationItem(
    'Rune',
    '/home',
    tappable: false,
    children: [
      NavigationItem(
        'Library',
        '/library',
        shortcuts: [
          LogicalKeySet(LogicalKeyboardKey.home),
          LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyL)
        ],
        children: [
          NavigationItem(
            'Search',
            '/search',
            zuneOnly: true,
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyS)
            ],
          ),
          NavigationItem(
            'Artists',
            '/artists',
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyR)
            ],
            children: [
              NavigationItem(
                'Artist Query',
                '/artists/:artistId',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            'Albums',
            '/albums',
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyA)
            ],
            children: [
              NavigationItem(
                'Artist Query',
                '/albums/:albumId',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            'Playlists',
            '/playlists',
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyP)
            ],
            children: [
              NavigationItem(
                'Playlist Query',
                '/playlists/:playlistId',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            'Mixes',
            '/mixes',
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyM)
            ],
            children: [
              NavigationItem(
                'Mix Query',
                '/mixes/:mixId',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            'Tracks',
            '/tracks',
            shortcuts: [
              LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyT)
            ],
          ),
        ],
      ),
      NavigationItem(
        'Settings',
        '/settings',
        shortcuts: [
          LogicalKeySet(LogicalKeyboardKey.alt, LogicalKeyboardKey.keyT)
        ],
        children: [
          NavigationItem('Library', '/settings/library'),
          NavigationItem('Controller', '/settings/media_controller'),
          NavigationItem('About', '/settings/about'),
          NavigationItem('Test', '/settings/mix'),
        ],
      ),
    ],
  ),
  // We must keep this here to make page transition parsing works correctly!
  NavigationItem('Search', '/search'),
];
