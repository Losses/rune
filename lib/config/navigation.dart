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
          const SingleActivator(LogicalKeyboardKey.home),
          const SingleActivator(alt: true, LogicalKeyboardKey.keyL)
        ],
        children: [
          NavigationItem(
            'Search',
            '/search',
            zuneOnly: true,
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyS)
            ],
          ),
          NavigationItem(
            'Artists',
            '/artists',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyR)
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
              const SingleActivator(alt: true, LogicalKeyboardKey.keyA)
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
              const SingleActivator(alt: true, LogicalKeyboardKey.keyP)
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
              const SingleActivator(alt: true, LogicalKeyboardKey.keyM)
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
              const SingleActivator(alt: true, LogicalKeyboardKey.keyT)
            ],
          ),
        ],
      ),
      NavigationItem(
        'Settings',
        '/settings',
        shortcuts: [
          const SingleActivator(
            control: true,
            alt: true,
            LogicalKeyboardKey.keyS,
          )
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
