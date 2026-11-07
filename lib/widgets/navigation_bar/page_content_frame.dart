import 'dart:math' as math;

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
  // Outside a frame the padding is the system's, which would put the home
  // indicator under the list and leave the cover art disk over it.
  assert(
    context.findAncestorWidgetOfExactType<PageContentFrame>() != null,
    'getScrollContainerPadding reads what a PageContentFrame leaves to its '
    'content, so its context must be inside one. A page that builds its '
    'list itself gives the frame a builder.',
  );
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
/// clear of the playback controller, the keyboard and the insets kept clear
/// of there ([keptClearOf]) where there is a controller ([bottom]).
///
/// Below it, the padding and the view padding of the [MediaQuery] are both
/// what the content still has to keep clear of: the sides of the window, the
/// cover art disk, the bottom in the layouts without a controller, and the
/// status bar when the frame leaves the top to the page. Lists and fluent's
/// scrollbars pad themselves by them, and [getScrollContainerPadding] reads
/// them. The keyboard is taken out of the view insets there; content that
/// needs its height reads [keyboardOf].
class PageContentFrame extends StatefulWidget {
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

  /// The height of the on-screen keyboard over the window, at [context].
  ///
  /// Inside a frame the view insets no longer have it, so a popup there that
  /// must end above the keyboard reads it here. Outside one it is the view
  /// insets' own.
  static double keyboardOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_Keyboard>()?.height ??
      MediaQuery.viewInsetsOf(context).bottom;

  @override
  State<PageContentFrame> createState() => _PageContentFrameState();
}

class _PageContentFrameState extends State<PageContentFrame> {
  // The content keeps its widget while only the insets change, so a page
  // given as a builder is not built again on every frame of the keyboard
  // sliding in or of a resize: only what reads the insets is.
  late Widget _content = _contentOf(widget);

  static Widget _contentOf(PageContentFrame frame) =>
      frame.child ?? Builder(builder: frame.builder!);

  @override
  void didUpdateWidget(PageContentFrame oldWidget) {
    super.didUpdateWidget(oldWidget);
    _content = _contentOf(widget);
  }

  @override
  Widget build(BuildContext context) {
    final responsive = Provider.of<ResponsiveProvider>(context);
    final chrome = pageChromeOf(context);
    final keptClear = keptClearOf(MediaQuery.viewPaddingOf(context));
    final keyboard = MediaQuery.viewInsetsOf(context).bottom;
    final clearsBottom = widget.bottom && chrome.controller > 0;
    // The keyboard comes up over the bottom inset and covers it.
    final bottom = math.max(keyboard, keptClear.bottom);

    final remaining = EdgeInsets.fromLTRB(
      keptClear.left,
      widget.top ? 0 : keptClear.top,
      keptClear.right + chrome.disk.right,
      clearsBottom ? 0 : chrome.disk.bottom + bottom,
    );

    return Container(
      padding: EdgeInsets.fromLTRB(
        widget.left && responsive.smallerOrEqualTo(DeviceType.band, false)
            ? 24
            : 0,
        widget.top ? keptClear.top + chrome.header : 0,
        0,
        clearsBottom ? chrome.controller + bottom : 0,
      ),
      child: _Keyboard(
        // A frame inside another one sees no keyboard, the outer one took it.
        height: PageContentFrame.keyboardOf(context),
        child: _Remaining(insets: remaining, child: _content),
      ),
    );
  }
}

/// The keyboard's height for [PageContentFrame.keyboardOf].
class _Keyboard extends InheritedWidget {
  const _Keyboard({required this.height, required super.child});

  final double height;

  @override
  bool updateShouldNotify(_Keyboard oldWidget) => height != oldWidget.height;
}

/// Gives the content [insets] as its padding and its view padding, and takes
/// the keyboard out of its view insets. Apart from the frame, which depends
/// only on the parts of the media query it reads, as this has to copy it all.
class _Remaining extends StatelessWidget {
  const _Remaining({required this.insets, required this.child});

  final EdgeInsets insets;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final media = MediaQuery.of(context);
    return MediaQuery(
      data: media.copyWith(
        padding: insets,
        viewPadding: insets,
        viewInsets: media.viewInsets.copyWith(bottom: 0),
      ),
      child: child,
    );
  }
}
