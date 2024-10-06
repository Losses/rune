import 'package:fluent_ui/fluent_ui.dart';

class DelayedDisplay extends StatefulWidget {
  final Widget child;
  final Duration delay;

  const DelayedDisplay({
    super.key,
    required this.child,
    this.delay = const Duration(milliseconds: 100),
  });

  @override
  DelayedDisplayState createState() => DelayedDisplayState();
}

class DelayedDisplayState extends State<DelayedDisplay> {
  bool _isVisible = false;

  @override
  void initState() {
    super.initState();
    Future.delayed(widget.delay, () {
      if (mounted) {
        setState(() {
          _isVisible = true;
        });
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedOpacity(
      opacity: _isVisible ? 1.0 : 0.0,
      duration: const Duration(milliseconds: 300),
      child: widget.child,
    );
  }
}
