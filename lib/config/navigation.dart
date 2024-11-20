import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../utils/navigation/navigation_item.dart';
import '../providers/router_path.dart';
import '../utils/l10n.dart';

final List<NavigationItem> navigationItems = [
  NavigationItem(
    (context) => S.of(context).rune,
    '/',
    onTap: (context) {
      final path = $router.path;

      if (path != '/library') {
        $replace('/library');
      }
    },
    children: [
      NavigationItem(
        (context) => S.of(context).library,
        '/library',
        shortcuts: [
          const SingleActivator(LogicalKeyboardKey.home),
          const SingleActivator(alt: true, LogicalKeyboardKey.keyL)
        ],
        children: [
          NavigationItem(
            (context) => S.of(context).search,
            '/search',
            zuneOnly: true,
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyS)
            ],
          ),
          NavigationItem(
            (context) => S.of(context).artists,
            '/artists',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyR)
            ],
            children: [
              NavigationItem(
                (context) => S.of(context).artistQuery,
                '/artists/detail',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            (context) => S.of(context).albums,
            '/albums',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyA)
            ],
            children: [
              NavigationItem(
                (context) => S.of(context).artistQuery,
                '/albums/detail',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            (context) => S.of(context).playlists,
            '/playlists',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyP)
            ],
            children: [
              NavigationItem(
                (context) => S.of(context).playlistQuery,
                '/playlists/detail',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            (context) => S.of(context).mixes,
            '/mixes',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyM)
            ],
            children: [
              NavigationItem(
                (context) => S.of(context).mixQuery,
                '/mixes/detail',
                hidden: true,
              ),
            ],
          ),
          NavigationItem(
            (context) => S.of(context).tracks,
            '/tracks',
            shortcuts: [
              const SingleActivator(alt: true, LogicalKeyboardKey.keyT)
            ],
          ),
        ],
      ),
      NavigationItem(
        (context) => S.of(context).settings,
        '/settings',
        shortcuts: [
          const SingleActivator(
            control: true,
            alt: true,
            LogicalKeyboardKey.keyS,
          )
        ],
        children: [
          NavigationItem(
            (context) => S.of(context).library,
            '/settings/library',
          ),
          NavigationItem(
            (context) => S.of(context).analysis,
            '/settings/analysis',
          ),
          NavigationItem(
            (context) => S.of(context).playback,
            '/settings/playback',
          ),
          NavigationItem(
            (context) => S.of(context).theme,
            '/settings/theme',
          ),
          NavigationItem(
            (context) => S.of(context).language,
            '/settings/language',
          ),
          NavigationItem(
            (context) => S.of(context).controller,
            '/settings/media_controller',
          ),
          NavigationItem(
            (context) => S.of(context).home,
            '/settings/library_home',
          ),
          NavigationItem(
            (context) => S.of(context).log,
            '/settings/log',
          ),
          NavigationItem(
            (context) => S.of(context).about,
            '/settings/about',
          ),
          // NavigationItem((_) => 'Test', '/settings/test'),
        ],
      ),
    ],
  ),
  // We must keep this here to make page transition parsing works correctly!
  NavigationItem((context) => S.of(context).search, '/search'),
];
