import 'package:fluent_ui/fluent_ui.dart';

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

  @override
  StartGroupNormalLayoutState createState() => StartGroupNormalLayoutState();
}

class StartGroupNormalLayoutState extends State<StartGroupNormalLayout> {
  double _opacity = 1.0;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: const EdgeInsets.only(left: 16, right: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
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
                    padding: const EdgeInsets.only(bottom: 4),
                    child: Text(widget.groupTitle,
                        style: theme.typography.bodyLarge),
                  ),
                ),
              ),
            ),
          ),
          Expanded(
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
