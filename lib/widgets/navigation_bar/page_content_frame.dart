import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/constants/navigation_bar_height.dart';
import '../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../providers/responsive_providers.dart';

class PageContentFrame extends StatefulWidget {
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
  PageContentFrameState createState() => PageContentFrameState();
}

class PageContentFrameState extends State<PageContentFrame> {
  @override
  Widget build(BuildContext context) {
    double top = 0;
    double left = 0;
    double right = 0;
    double bottom = 0;

    final inset = MediaQuery.viewInsetsOf(context);
    final responsive = Provider.of<ResponsiveProvider>(context);
    final screen = Provider.of<ScreenSizeProvider>(context).screenSize;

    if (widget.top) {
      if (responsive.smallerOrEqualTo(DeviceType.dock)) {
        top = bandNavigationBarHeight + inset.top;
      } else {
        top = fullNavigationBarHeight + inset.top;
      }
    }

    if (widget.bottom) {
      if (responsive.smallerOrEqualTo(DeviceType.dock)) {
        bottom = screen.width / 3 + inset.bottom;
      } else {
        bottom = playbackControllerHeight + inset.bottom;
      }
    }

    if (widget.right) {
      if (responsive.smallerOrEqualTo(DeviceType.car, false)) {
        bottom = 0;

        if (widget.right) {
          right = screen.height / 3 + inset.right;
        }

        if (widget.top) {
          top = carNavigationBarHeight + inset.top;
        }
      }
    }

    return Container(
      padding: EdgeInsets.fromLTRB(left, top, right, bottom),
      child: widget.child,
    );
  }
}
