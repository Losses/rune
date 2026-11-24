import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/measure_title.dart';

class StartGroupNormalLayout extends StatefulWidget {
  final String groupTitle;
  final Widget child;
  final VoidCallback? onTitleTap;

  const StartGroupNormalLayout({
    super.key,
    required this.groupTitle,
    required this.child,
    this.onTitleTap,
  });

  static const padding = EdgeInsets.only(left: 16, right: 16);
  static const titlePadding = EdgeInsets.only(bottom: 4);

  /// The width the layout takes around a child that is [childWidth] wide.
  static double widthFor(
    BuildContext context,
    String title,
    double childWidth,
  ) {
    final titleWidth = measureTitle(
      context,
      title,
      FluentTheme.of(context).typography.bodyLarge,
    ).width;
    return max(titleWidth, childWidth) + padding.horizontal;
  }

  @override
  StartGroupNormalLayoutState createState() => StartGroupNormalLayoutState();
}

class StartGroupNormalLayoutState extends State<StartGroupNormalLayout> {
  double _opacity = 1.0;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: StartGroupNormalLayout.padding,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          FocusableActionDetector(
            onShowFocusHighlight: (focus) {},
            onShowHoverHighlight: (hover) {},
            child: MouseRegion(
              onEnter: (_) => _changeOpacity(0.7),
              onExit: (_) => _changeOpacity(1.0),
              child: GestureDetector(
                onTap: widget.onTitleTap,
                child: AnimatedOpacity(
                  opacity: _opacity,
                  duration: const Duration(milliseconds: 100),
                  child: Padding(
                    padding: StartGroupNormalLayout.titlePadding,
                    child: Text(
                      widget.groupTitle,
                      style: theme.typography.bodyLarge,
                    ),
                  ),
                ),
              ),
            ),
          ),
          widget.child,
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
