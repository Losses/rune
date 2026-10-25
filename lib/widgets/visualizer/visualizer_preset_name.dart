import 'dart:io';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

import 'visualizer_controller.dart';

const _shownFor = Duration(seconds: 4);
const _fade = Duration(milliseconds: 400);

/// The name of the scene that just started, in the top left corner of the
/// visualizer for a few seconds, the way Winamp shows a preset's name. It
/// keeps clear of the macOS window buttons and the cover wall's back button.
/// A problem (the texture could not be set up, a scene failed) stays until
/// it is gone.
class VisualizerPresetName extends StatefulWidget {
  const VisualizerPresetName({super.key, required this.controller});

  final VisualizerController controller;

  @override
  State<VisualizerPresetName> createState() => _VisualizerPresetNameState();
}

class _VisualizerPresetNameState extends State<VisualizerPresetName> {
  Timer? _hideTimer;
  bool _visible = false;

  @override
  void initState() {
    super.initState();
    widget.controller.label.addListener(_onLabelChanged);
  }

  @override
  void dispose() {
    widget.controller.label.removeListener(_onLabelChanged);
    _hideTimer?.cancel();
    super.dispose();
  }

  void _onLabelChanged() {
    final label = widget.controller.label.value;
    _hideTimer?.cancel();
    setState(() => _visible = label != null);
    if (label != null && !label.isError) {
      _hideTimer = Timer(_shownFor, () {
        if (mounted) setState(() => _visible = false);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final label = widget.controller.label.value;
    final theme = FluentTheme.of(context);
    final viewPadding = MediaQuery.viewPaddingOf(context);
    final r = Provider.of<ResponsiveProvider>(context);

    // Small layouts show the back button in the corner and hide the macOS
    // window buttons; the others show the buttons stacked on the left.
    final small = r.smallerOrEqualTo(DeviceType.mobile, false);
    final left = 16 +
        viewPadding.left +
        (small ? 40 : 0) +
        (Platform.isMacOS && !small ? 24 : 0);

    return Positioned(
      top: 16 + viewPadding.top,
      left: left,
      right: 16 + viewPadding.right,
      child: IgnorePointer(
        child: AnimatedOpacity(
          opacity: _visible ? 1 : 0,
          duration: _fade,
          child: Text(
            label?.text ?? '',
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: theme.typography.bodyStrong?.copyWith(
              color: label?.isError == true ? Colors.red : null,
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
    );
  }
}
