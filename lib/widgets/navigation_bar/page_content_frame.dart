import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/constants/navigation_bar_height.dart';
import '../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../providers/responsive_providers.dart';

EdgeInsets getScrollContainerPadding(
  BuildContext context, {
  bool top = true,
  bool left = true,
  bool right = true,
  bool bottom = true,
  double leftPlus = 0,
  double rightPlus = 0,
}) {
  double pTop = 0;
  double pLeft = 0;
  double pRight = 0;
  double pBottom = 0;

  final inset = MediaQuery.viewInsetsOf(context);
  final responsive = Provider.of<ResponsiveProvider>(context);
  final screen = Provider.of<ScreenSizeProvider>(context).screenSize;

  if (bottom) {
    if (responsive.smallerOrEqualTo(DeviceType.zune, false)) {
      pBottom = screen.width / 3 + inset.bottom;
    } else if (responsive.smallerOrEqualTo(DeviceType.dock, false)) {
      pBottom = screen.width / 3 + inset.bottom;
    } else if (responsive.smallerOrEqualTo(DeviceType.phone, false)) {
    } else if (responsive.smallerOrEqualTo(DeviceType.band, false)) {
      pBottom = inset.bottom;
    } else if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
      pBottom = inset.bottom;
    }
  }

  if (right) {
    if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
      pRight = screen.height / 3 + inset.right + rightPlus;
    } else if (responsive.smallerOrEqualTo(DeviceType.belt, false)) {
      pRight = 16 + inset.right + rightPlus;
    } else {
      pRight = inset.right + rightPlus;
    }
  }

  if (left) {
    if (responsive.smallerOrEqualTo(DeviceType.belt, false)) {
      pLeft = 16 + inset.left + rightPlus;
    } else {
      pLeft = inset.left + leftPlus;
    }
  }

  return EdgeInsets.fromLTRB(pLeft, pTop, pRight, pBottom);
}

class PageContentFrame extends StatelessWidget {
  const PageContentFrame({
    super.key,
    this.top = true,
    this.left = true,
    this.right = true,
    this.bottom = true,
    required this.child,
  });

  final bool top;
  final bool left;
  final bool right;
  final bool bottom;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    double pTop = 0;
    double pLeft = 0;
    double pRight = 0;
    double pBottom = 0;

    final inset = MediaQuery.viewInsetsOf(context);
    final responsive = Provider.of<ResponsiveProvider>(context);

    double topBarHeight = Platform.isAndroid
        ? MediaQueryData.fromView(View.of(context)).padding.top
        : 0;

    if (top) {
      if (responsive.smallerOrEqualTo(DeviceType.dock, false)) {
        pTop = bandNavigationBarHeight + inset.top + topBarHeight;
      } else if (responsive.smallerOrEqualTo(DeviceType.band, false)) {
        pTop = inset.top;
      } else if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
        pTop = carNavigationBarHeight + inset.top + topBarHeight;
      } else {
        pTop = fullNavigationBarHeight + inset.top + topBarHeight;
      }
    }

    if (bottom) {
      if (responsive.smallerOrEqualTo(DeviceType.dock, false)) {
      } else if (responsive.smallerOrEqualTo(DeviceType.band, false)) {
      } else if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
      } else if (responsive.smallerOrEqualTo(DeviceType.zune, false)) {
      } else if (responsive.smallerOrEqualTo(DeviceType.phone, false)) {
        pBottom = playbackControllerHeight + inset.bottom;
      } else {
        pBottom = playbackControllerHeight + inset.bottom;
      }
    }

    if (left) {
      if (responsive.smallerOrEqualTo(DeviceType.band, false)) {
        pLeft = 24;
      }
    }

    return Container(
      padding: EdgeInsets.fromLTRB(pLeft, pTop, pRight, pBottom),
      child: child,
    );
  }
}
