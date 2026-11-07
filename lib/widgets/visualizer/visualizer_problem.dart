import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

import 'visualizer_controller.dart';

const _fade = Duration(milliseconds: 400);

/// A problem of the visualizer (the texture could not be set up, a scene
/// failed) in the top left corner, until it is gone. It keeps clear of the
/// macOS window buttons and the cover wall's back button.
class VisualizerProblem extends StatefulWidget {
  const VisualizerProblem({super.key, required this.controller});

  final VisualizerController controller;

  @override
  State<VisualizerProblem> createState() => _VisualizerProblemState();
}

class _VisualizerProblemState extends State<VisualizerProblem> {
  bool _visible = false;

  @override
  void initState() {
    super.initState();
    widget.controller.problem.addListener(_onProblemChanged);
  }

  @override
  void dispose() {
    widget.controller.problem.removeListener(_onProblemChanged);
    super.dispose();
  }

  void _onProblemChanged() {
    setState(() => _visible = widget.controller.problem.value != null);
  }

  @override
  Widget build(BuildContext context) {
    final problem = widget.controller.problem.value;
    final theme = FluentTheme.of(context);
    final r = Provider.of<ResponsiveProvider>(context);

    // Narrow windows show the back button in the corner, by the check the
    // button makes; the others show the macOS window buttons stacked on the
    // left.
    final small = r.smallerOrEqualTo(DeviceType.mobile);
    final left =
        16.0 + (small ? 40 : 0) + (Platform.isMacOS && !small ? 24 : 0);

    return Positioned(
      top: 0,
      left: 0,
      right: 0,
      child: IgnorePointer(
        child: SafeArea(
          bottom: false,
          child: Padding(
            padding: EdgeInsets.fromLTRB(left, 16, 16, 0),
            child: AnimatedOpacity(
              opacity: _visible ? 1 : 0,
              duration: _fade,
              child: Text(
                // Shown whole: the tail says why (a scene's file and line, the
                // graphics API a GPU was looked for in).
                problem ?? '',
                style: theme.typography.bodyStrong?.copyWith(
                  color: Colors.red,
                  shadows: [
                    Shadow(
                      color: theme.brightness.isDark
                          ? Colors.black
                          : Colors.white,
                      blurRadius: 8,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
