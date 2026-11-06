import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/kept_clear.dart';
import '../../providers/responsive_providers.dart';

import 'page_chrome.dart';

/// The padding a scrolling list on a page takes: what the [PageContentFrame]
/// around it leaves to its content (the sides of the window, the cover art
/// disk and, where the frame does not clear it, the keyboard), with
/// [leftPlus] and [rightPlus] on top. [context] must be inside the frame.
EdgeInsets getScrollContainerPadding(
  BuildContext context, {
  bool left = true,
  bool right = true,
  bool bottom = true,
  double leftPlus = 0,
  double rightPlus = 0,
  bool listen = true,
}) {
  final remaining = MediaQuery.paddingOf(context);
  final responsive = Provider.of<ResponsiveProvider>(context, listen: listen);
  final belt = responsive.smallerOrEqualTo(DeviceType.belt, false) ? 16.0 : 0.0;

  return EdgeInsets.fromLTRB(
    left ? remaining.left + belt + leftPlus : 0,
    0,
    right ? remaining.right + rightPlus : 0,
    bottom ? remaining.bottom : 0,
  );
}

/// Where a page's content resolves the insets around it. It pads its top
/// clear of the status bar and the navigation bar ([top]), and its bottom
/// clear of the playback controller and the keyboard where there is a
/// controller ([bottom]).
///
/// Below it, the padding and the view padding of the [MediaQuery] are both
/// what the content still has to keep clear of: the sides of the window, the
/// cover art disk, the keyboard in the layouts without a controller, and the
/// status bar when the frame leaves the top to the page. Lists and fluent's
/// scrollbars pad themselves by them, and [getScrollContainerPadding] reads
/// them.
class PageContentFrame extends StatelessWidget {
  const PageContentFrame({
    super.key,
    this.top = true,
    this.left = true,
    this.right = true,
    this.bottom = true,
    this.child,
    this.builder,
  }) : assert(
         (child == null) != (builder == null),
         'Give a child or a builder.',
       );

  final bool top;
  final bool left;
  final bool right;
  final bool bottom;
  final Widget? child;

  /// Builds the content with a context inside the frame, for a page that
  /// reads the insets the frame leaves while it builds itself.
  final WidgetBuilder? builder;

  @override
  Widget build(BuildContext context) {
    final responsive = Provider.of<ResponsiveProvider>(context);
    final screen = Provider.of<ScreenSizeProvider>(context).screenSize;
    final chrome = pageChromeOf(responsive, screen);
    final media = MediaQuery.of(context);
    final keptClear = keptClearOf(media.viewPadding);
    final keyboard = media.viewInsets.bottom;
    final clearsBottom = bottom && chrome.controller > 0;

    final remaining = EdgeInsets.fromLTRB(
      keptClear.left,
      top ? 0 : keptClear.top,
      keptClear.right + chrome.disk.right,
      clearsBottom ? 0 : chrome.disk.bottom + keyboard,
    );

    return Container(
      padding: EdgeInsets.fromLTRB(
        left && responsive.smallerOrEqualTo(DeviceType.band, false) ? 24 : 0,
        top ? keptClear.top + chrome.header : 0,
        0,
        clearsBottom ? chrome.controller + keyboard : 0,
      ),
      child: MediaQuery(
        data: media.copyWith(
          padding: remaining,
          viewPadding: remaining,
          viewInsets: media.viewInsets.copyWith(bottom: 0),
        ),
        child: child ?? Builder(builder: builder!),
      ),
    );
  }
}
