import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/measure_title.dart';

class StartGroupStackedLayout extends StatefulWidget {
  final String groupTitle;
  final Widget child;
  final VoidCallback? onTitleTap;

  const StartGroupStackedLayout({
    super.key,
    required this.groupTitle,
    required this.child,
    this.onTitleTap,
  });

  static const padding = EdgeInsets.only(left: 36, right: 16);
  static const childPadding = EdgeInsets.only(left: 16, top: 32);

  /// The width the layout takes around a child that is [childWidth] wide. The
  /// title and the padded child share a Stack.
  static double widthFor(
    BuildContext context,
    String title,
    double childWidth,
  ) {
    final titleWidth = measureTitle(
      context,
      title,
      FluentTheme.of(context).typography.titleLarge,
    ).width;
    return max(titleWidth, childPadding.left + childWidth) + padding.horizontal;
  }

  @override
  StartGroupStackedLayoutState createState() => StartGroupStackedLayoutState();
}

class StartGroupStackedLayoutState extends State<StartGroupStackedLayout> {
  double _opacity = 0.5;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: StartGroupStackedLayout.padding,
      child: Stack(
        children: [
          // The title is not focusable, since its main purpose is for decoration
          // UX designers should always implement a navigation entry in the same
          // screen that has the same feature as onTitleTap here.
          MouseRegion(
            onEnter: (_) {
              if (widget.onTitleTap != null) {
                _changeOpacity(1.0);
              }
            },
            onExit: (_) {
              if (widget.onTitleTap != null) {
                _changeOpacity(0.5);
              }
            },
            child: Listener(
              onPointerUp: (_) {
                if (widget.onTitleTap == null) return;
                widget.onTitleTap!();
              },
              child: AnimatedOpacity(
                opacity: _opacity,
                duration: const Duration(milliseconds: 100),
                child:
                    Text(widget.groupTitle, style: theme.typography.titleLarge),
              ),
            ),
          ),
          Padding(
            padding: StartGroupStackedLayout.childPadding,
            child: widget.child,
          ),
        ],
      ),
    );
  }

  void _changeOpacity(double opacity) {
    setState(() {
      _opacity = opacity;
    });
  }
}
