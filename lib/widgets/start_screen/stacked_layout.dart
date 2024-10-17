import 'package:fluent_ui/fluent_ui.dart';

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

  @override
  StartGroupStackedLayoutState createState() => StartGroupStackedLayoutState();
}

class StartGroupStackedLayoutState extends State<StartGroupStackedLayout> {
  double _opacity = 0.5;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: const EdgeInsets.only(left: 36, right: 16),
      child: Stack(
        children: [
          // The title is not focusable, since its main purpose is for decoration
          // UX designers should always implement a navigation entry in the same
          // screen that has the same feature as onTitleTap here.
          MouseRegion(
            onEnter: (_) => _changeOpacity(1.0),
            onExit: (_) => _changeOpacity(0.5),
            child: GestureDetector(
              onTap: widget.onTitleTap,
              child: AnimatedOpacity(
                opacity: _opacity,
                duration: const Duration(milliseconds: 100),
                child:
                    Text(widget.groupTitle, style: theme.typography.titleLarge),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.only(left: 16, top: 32),
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
