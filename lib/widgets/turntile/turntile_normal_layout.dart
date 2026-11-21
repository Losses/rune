import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/hover_opacity.dart';

class TurnTileGroupNormalLayout extends StatefulWidget {
  final String? groupTitle;
  final Widget child;
  final VoidCallback? onTitleTap;

  const TurnTileGroupNormalLayout({
    super.key,
    required this.groupTitle,
    required this.child,
    this.onTitleTap,
  });

  static const padding = EdgeInsets.only(left: 16, right: 16, bottom: 16);
  static const titlePadding = EdgeInsets.only(bottom: 4);

  /// The height the layout takes around a child that is [childHeight] high in
  /// a list [width] wide.
  static double heightFor(
    BuildContext context,
    String? title,
    double width,
    double childHeight,
  ) {
    var height = padding.vertical + childHeight;
    if (title != null) {
      final painter = TextPainter(
        text: TextSpan(
          text: title,
          style: DefaultTextStyle.of(context).style.merge(
                FluentTheme.of(context).typography.bodyLarge,
              ),
        ),
        textScaler:
            MediaQuery.maybeTextScalerOf(context) ?? TextScaler.noScaling,
        textDirection: Directionality.of(context),
      )..layout(maxWidth: max(0, width - padding.horizontal));
      height += painter.height + titlePadding.bottom;
      painter.dispose();
    }
    return height;
  }

  @override
  TurnTileGroupNormalLayoutState createState() =>
      TurnTileGroupNormalLayoutState();
}

class TurnTileGroupNormalLayoutState extends State<TurnTileGroupNormalLayout> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: TurnTileGroupNormalLayout.padding,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (widget.groupTitle != null)
            HoverOpacity(
              child: GestureDetector(
                onTap: widget.onTitleTap,
                child: Padding(
                  padding: TurnTileGroupNormalLayout.titlePadding,
                  child: Text(
                    widget.groupTitle!,
                    style: theme.typography.bodyLarge,
                  ),
                ),
              ),
            ),
          widget.child,
        ],
      ),
    );
  }
}
