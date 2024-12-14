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
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  late Animation<double> _animation;
  double _previousProgress = 0.0;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      duration: const Duration(milliseconds: 16),
      vsync: this,
    );
    _previousProgress = calculateProgress();
    _animation = Tween<double>(
      begin: _previousProgress,
      end: _previousProgress,
    ).animate(_controller);
  }

  @override
  void didUpdateWidget(LyricSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.currentTimeMilliseconds != oldWidget.currentTimeMilliseconds ||
        widget.isActive != oldWidget.isActive ||
        widget.isPassed != oldWidget.isPassed) {
      final newProgress = calculateProgress();
      if (newProgress != _previousProgress) {
        _animation = Tween<double>(
          begin: _previousProgress,
          end: newProgress,
        ).animate(
          CurvedAnimation(
            parent: _controller,
            curve: Curves.linear,
          ),
        );
        _previousProgress = newProgress;
        _controller.forward(from: 0);
      }
    }
  }

  @override
  void dispose() {
    _controller.dispose();
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
      animation: _animation,
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
                  widthFactor: _animation.value,
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
            if (widget.isActive)
              ImageFiltered(
                imageFilter: ImageFilter.blur(
                  sigmaX: 3,
                  sigmaY: 3,
                  tileMode: TileMode.decal,
                ),
                child: ClipRect(
                  child: Align(
                    alignment: Alignment.centerLeft,
                    widthFactor: _animation.value,
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
