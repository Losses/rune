import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/navigation_bar/navigation_bar_placeholder.dart';
import 'package:rune/widgets/playback_controller/constants/playback_controller_height.dart';

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
    final screen = Provider.of<ScreenSizeProvider>(context);

    if (widget.top) {
      if (responsive.smallerOrEqualTo(DeviceType.band)) {
        top = bandNavigationBarHeight + inset.top;
      } else {
        top = fullNavigationBarHeight + inset.top;
      }
    }

    if (widget.bottom) {
      if (responsive.smallerOrEqualTo(DeviceType.band)) {
        bottom = screen.screenSize.width / 3 + inset.bottom;
      } else {
        bottom = playbackControllerHeight + inset.bottom;
      }
    }

    return Container(
      padding: EdgeInsets.fromLTRB(left, top, right, bottom),
      child: widget.child,
    );
  }
}
