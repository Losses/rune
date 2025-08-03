import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

const fontWeight = FontWeight.w600;

class LyricSection extends StatefulWidget {
  final LyricContentLineSection section;
  final int currentTimeMilliseconds;
  final bool isActive;
  final bool isPassed;

  const LyricSection({
    super.key,
    required this.section,
    required this.currentTimeMilliseconds,
    required this.isActive,
    required this.isPassed,
  });

  @override
  State<LyricSection> createState() => _LyricSectionState();
}

class _LyricSectionState extends State<LyricSection>
    with TickerProviderStateMixin {
  late AnimationController _progressAnimationController;
  late Animation<double> _progressAnimation;
  late AnimationController _blurAnimationController;
  late Animation<double> _blurAnimation;
  double _previousProgress = 0.0;
  double _targetBlurValue = 0.0;

  @override
  void initState() {
    super.initState();
    _initProgressAnimation();
    _initBlurAnimation();
  }

  @override
  didChangeDependencies() {
    super.didChangeDependencies();
    if (widget.isActive) {
      _updateActiveBlur(5.0);
    }
  }

  void _initProgressAnimation() {
    _progressAnimationController = AnimationController(
      duration: const Duration(milliseconds: 16),
      vsync: this,
    );
    _previousProgress = calculateProgress();
    _progressAnimation = Tween<double>(
      begin: _previousProgress,
      end: _previousProgress,
    ).animate(_progressAnimationController);
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

    if (widget.currentTimeMilliseconds != oldWidget.currentTimeMilliseconds ||
        widget.isActive != oldWidget.isActive ||
        widget.isPassed != oldWidget.isPassed) {
      _updateProgressAnimation();
    }

    if (widget.isActive != oldWidget.isActive) {
      if (!widget.isActive) {
        _updateActiveBlur(0.0);
      } else {
        _updateActiveBlur(5.0);
      }
    }
  }

  void _updateProgressAnimation() {
    final newProgress = calculateProgress();
    if (newProgress != _previousProgress) {
      _progressAnimation = Tween<double>(
        begin: _previousProgress,
        end: newProgress,
      ).animate(
        CurvedAnimation(
          parent: _progressAnimationController,
          curve: Curves.linear,
        ),
      );
      _previousProgress = newProgress;
      _progressAnimationController.forward(from: 0);
    }
  }

  @override
  void dispose() {
    _progressAnimationController.dispose();
    _blurAnimationController.dispose();
    super.dispose();
  }

  double calculateProgress() {
    if (widget.isPassed) return 1.0;
    if (!widget.isActive) return 0.0;

    final duration = widget.section.endTime - widget.section.startTime;
    if (duration <= 0) return 0.0;

    if (widget.currentTimeMilliseconds >= widget.section.endTime) {
      return 1.0;
    } else if (widget.currentTimeMilliseconds > widget.section.startTime) {
      return (widget.currentTimeMilliseconds - widget.section.startTime) /
          duration;
    }
    return 0.0;
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: Listenable.merge([_progressAnimation, _blurAnimation]),
      builder: (context, child) {
        return _LyricContent(
          section: widget.section,
          progress: _progressAnimation.value,
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
