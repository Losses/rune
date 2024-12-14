import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

const double fontSize = 24;
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

  @override
  void initState() {
    super.initState();
    _progressAnimationController = AnimationController(
      duration: const Duration(milliseconds: 16),
      vsync: this,
    );
    _previousProgress = calculateProgress();
    _progressAnimation = Tween<double>(
      begin: _previousProgress,
      end: _previousProgress,
    ).animate(_progressAnimationController);

    // Initialize the blur animation controller
    _blurAnimationController = AnimationController(
      duration: const Duration(milliseconds: 500),
      vsync: this,
    );
    _blurAnimation = Tween<double>(
      begin: 0,
      end: 0,
    ).animate(CurvedAnimation(
      parent: _blurAnimationController,
      curve: Curves.slowMiddle,
    ));
  }

  void _updateActiveBlur(double targetBlur) {
    if (targetBlur == _blurAnimation.value) {
      // Avoid unnecessary animation updates
      return;
    }

    _blurAnimation = Tween<double>(
      begin: _blurAnimation.value,
      end: targetBlur,
    ).animate(CurvedAnimation(
      parent: _blurAnimationController,
      curve: Curves.easeInOut,
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
        _updateActiveBlur(3.0);
      }
    }

    if (widget.currentTimeMilliseconds != oldWidget.currentTimeMilliseconds ||
        widget.isActive != oldWidget.isActive ||
        widget.isPassed != oldWidget.isPassed) {
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
  }

  @override
  void dispose() {
    _progressAnimationController.dispose();
    _blurAnimationController.dispose();
    super.dispose();
  }

  double calculateProgress() {
    if (!widget.isActive) return 0.0;
    if (widget.isPassed) return 1.0;

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
    final theme = FluentTheme.of(context);

    return AnimatedBuilder(
      animation: _progressAnimation,
      builder: (context, child) {
        return Stack(
          children: [
            Text(
              widget.section.content,
              style: TextStyle(
                fontSize: fontSize,
                fontWeight: fontWeight,
                color: widget.isPassed
                    ? theme.resources.textFillColorPrimary
                    : theme.resources.textFillColorPrimary.withAlpha(160),
              ),
            ),
            if (widget.isActive)
              ClipRect(
                child: Align(
                  alignment: Alignment.centerLeft,
                  widthFactor: _progressAnimation.value,
                  child: Text(
                    widget.section.content,
                    style: TextStyle(
                      fontSize: fontSize,
                      fontWeight: fontWeight,
                      color: theme.resources.textFillColorPrimary,
                    ),
                  ),
                ),
              ),
            if (widget.isActive || !_blurAnimation.isCompleted)
              ImageFiltered(
                imageFilter: ImageFilter.blur(
                  sigmaX: _blurAnimation.value,
                  sigmaY: _blurAnimation.value,
                  tileMode: TileMode.decal,
                ),
                child: ClipRect(
                  child: Align(
                    alignment: Alignment.centerLeft,
                    widthFactor: _progressAnimation.value,
                    child: Text(
                      widget.section.content,
                      style: TextStyle(
                        fontSize: fontSize,
                        fontWeight: fontWeight,
                        color: theme.resources.textFillColorPrimary,
                      ),
                    ),
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}
