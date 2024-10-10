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

  @override
  TurnTileGroupNormalLayoutState createState() =>
      TurnTileGroupNormalLayoutState();
}

class TurnTileGroupNormalLayoutState extends State<TurnTileGroupNormalLayout> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: const EdgeInsets.only(left: 16, right: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (widget.groupTitle != null)
            HoverOpacity(
              child: GestureDetector(
                onTap: widget.onTitleTap,
                child: Padding(
                  padding: const EdgeInsets.only(bottom: 4),
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
