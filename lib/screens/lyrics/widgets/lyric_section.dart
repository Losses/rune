import 'dart:ui';

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

const fontWeight = FontWeight.w600;

class LyricSection extends StatefulWidget {
  final LyricContentLineSection section;
  final ValueListenable<double> interpolatedTime;
  final bool isActive;
  final bool isPassed;

  /// The section is the whole line: the lyrics are timed by line, not by word,
  /// so the line lights up at once instead of being swept, which would also
  /// sweep every wrapped row at the same time.
  final bool wholeLine;

  const LyricSection({
    super.key,
    required this.section,
    required this.interpolatedTime,
    required this.isActive,
    required this.isPassed,
    this.wholeLine = false,
  });

  @override
  State<LyricSection> createState() => _LyricSectionState();
}

class _LyricSectionState extends State<LyricSection>
    with TickerProviderStateMixin {
  late AnimationController _blurAnimationController;
  late Animation<double> _blurAnimation;
  double _targetBlurValue = 0.0;

  @override
  void initState() {
    super.initState();
    _initBlurAnimation();
  }

  @override
  didChangeDependencies() {
    super.didChangeDependencies();
    if (widget.isActive) {
      _updateActiveBlur(5.0);
    }
  }

  void _initBlurAnimation() {
    _blurAnimationController = AnimationController(
      duration: const Duration(milliseconds: 500),
      vsync: this,
    );
    _blurAnimation = Tween<double>(
      begin: 0,
      end: 0,
    ).animate(CurvedAnimation(
      parent: _blurAnimationController,
      curve: Curves.linear,
    ));
  }

  void _updateActiveBlur(double targetBlur) {
    if (targetBlur == _targetBlurValue) {
      return;
    }
    if (targetBlur == _blurAnimation.value) {
      // Avoid unnecessary animation updates
      return;
    }

    _targetBlurValue = targetBlur;

    _blurAnimation = Tween<double>(
      begin: _blurAnimation.value,
      end: targetBlur,
    ).animate(CurvedAnimation(
      parent: _blurAnimationController,
      curve: Curves.linear,
    ));

    // Reset the animation controller to its initial state
    _blurAnimationController.reset();
    _blurAnimationController.forward(from: 0);
  }

  @override
  void didUpdateWidget(LyricSection oldWidget) {
    super.didUpdateWidget(oldWidget);

    if (widget.isActive != oldWidget.isActive) {
      if (!widget.isActive) {
        _updateActiveBlur(0.0);
      } else {
        _updateActiveBlur(5.0);
      }
    }
  }

  @override
  void dispose() {
    _blurAnimationController.dispose();
    super.dispose();
  }

  double calculateProgress(double currentTime) {
    if (widget.isPassed) return 1.0;
    if (!widget.isActive) return 0.0;
    if (widget.wholeLine) return 1.0;

    final section = widget.section;
    final duration = section.endTime - section.startTime;

    // A section without duration is fully lit as soon as it starts.
    if (duration <= 0) return currentTime >= section.startTime ? 1.0 : 0.0;

    return ((currentTime - section.startTime) / duration).clamp(0.0, 1.0);
  }

  @override
  Widget build(BuildContext context) {
    // Only the active section follows the interpolated time; the others
    // never repaint with it.
    return AnimatedBuilder(
      animation: Listenable.merge([
        _blurAnimation,
        if (widget.isActive && !widget.wholeLine) widget.interpolatedTime,
      ]),
      builder: (context, child) {
        return _LyricContent(
          section: widget.section,
          progress: calculateProgress(widget.interpolatedTime.value),
          isActive: widget.isActive,
          isPassed: widget.isPassed,
          blur: _blurAnimation.value,
        );
      },
    );
  }
}

class _LyricContent extends StatelessWidget {
  final LyricContentLineSection section;
  final double progress;
  final bool isActive;
  final bool isPassed;
  final double blur;

  const _LyricContent({
    required this.section,
    required this.progress,
    required this.isActive,
    required this.isPassed,
    required this.blur,
  });

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        _LyricText(
          content: section.content,
          isPassed: isPassed,
          alpha: isPassed ? 255 : 160,
        ),
        if (isActive)
          _ProgressHighlight(
            content: section.content,
            progress: progress,
          ),
        if (isActive || blur != 0.0)
          _BlurredProgress(
            content: section.content,
            progress: progress,
            blur: blur,
          ),
      ],
    );
  }
}

class _LyricText extends StatelessWidget {
  final String content;
  final bool isPassed;
  final int alpha;

  const _LyricText({
    required this.content,
    required this.isPassed,
    required this.alpha,
  });

  @override
  Widget build(BuildContext context) {
    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.zune, false);

    return Text(
      content,
      style: TextStyle(
        fontSize: isMini ? 24 : 32,
        fontWeight: fontWeight,
        color: Colors.white.withAlpha(alpha),
      ),
    );
  }
}

class _ProgressHighlight extends StatelessWidget {
  final String content;
  final double progress;

  const _ProgressHighlight({
    required this.content,
    required this.progress,
  });

  @override
  Widget build(BuildContext context) {
    return ClipRect(
      child: Align(
        alignment: Alignment.centerLeft,
        widthFactor: progress,
        child: _LyricText(
          content: content,
          isPassed: false,
          alpha: 255,
        ),
      ),
    );
  }
}

class _BlurredProgress extends StatelessWidget {
  final String content;
  final double progress;
  final double blur;

  const _BlurredProgress({
    required this.content,
    required this.progress,
    required this.blur,
  });

  @override
  Widget build(BuildContext context) {
    return ImageFiltered(
      imageFilter: ImageFilter.blur(
        sigmaX: blur,
        sigmaY: blur,
        tileMode: TileMode.decal,
      ),
      child: _ProgressHighlight(
        content: content,
        progress: progress,
      ),
    );
  }
}
