import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../providers/responsive_providers.dart';
import '../playback_controller/constants/playback_controller_height.dart';

import 'constants/navigation_bar_height.dart';

/// What the app's own chrome takes from the edges of a page in a layout.
@immutable
class PageChrome {
  const PageChrome({
    required this.header,
    required this.controller,
    required this.disk,
  });

  /// The navigation bar at the top, or the room the back button takes there.
  final double header;

  /// The playback controller at the bottom.
  final double controller;

  /// The cover art disk, which stands in for the controller in the smallest
  /// layouts: at the bottom in the zune and dock layouts, at the right in the
  /// car layouts and those shorter.
  final EdgeInsets disk;

  @override
  bool operator ==(Object other) =>
      other is PageChrome &&
      other.header == header &&
      other.controller == controller &&
      other.disk == disk;

  @override
  int get hashCode => Object.hash(header, controller, disk);
}

/// The chrome of a page in the layout [responsive] has, in a [screen] of the
/// window's size at the interface scale.
PageChrome pageChromeFor(ResponsiveProvider responsive, Size screen) {
  final double header;
  if (responsive.smallerOrEqualTo(DeviceType.dock, false)) {
    header = bandNavigationBarHeight;
  } else if (responsive.smallerOrEqualTo(DeviceType.band, false)) {
    header = 0;
  } else if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
    header = carNavigationBarHeight;
  } else {
    header = fullNavigationBarHeight;
  }

  final diskAtBottom = responsive.smallerOrEqualTo(DeviceType.zune, false);
  final diskAtRight = responsive.smallerOrEqualTo(DeviceType.car, false);

  return PageChrome(
    header: header,
    controller: diskAtBottom || diskAtRight ? 0 : playbackControllerHeight,
    disk: EdgeInsets.only(
      right: diskAtRight ? screen.height / 3 : 0,
      bottom: diskAtBottom ? screen.width / 3 : 0,
    ),
  );
}

/// The chrome of a page in the layout and the window [context] is in.
///
/// It rebuilds [context] when the layout changes, and on a resize only when
/// the chrome changes with it: the disk follows the window, the rest does not.
PageChrome pageChromeOf(BuildContext context) {
  final responsive = Provider.of<ResponsiveProvider>(context);
  return context.select<ScreenSizeProvider, PageChrome>(
    (sizes) => pageChromeFor(responsive, sizes.screenSize),
  );
}
