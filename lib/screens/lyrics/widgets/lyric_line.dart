import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import 'lyric_section.dart';

class LyricLine extends StatefulWidget {
  final List<LyricContentLineSection> sections;
  final int currentTimeMilliseconds;
  final bool isActive;
  final bool isPassed;

  const LyricLine({
    super.key,
    required this.sections,
    required this.currentTimeMilliseconds,
    required this.isActive,
    required this.isPassed,
  });

  @override
  State<LyricLine> createState() => _LyricLineState();
}

class _LyricLineState extends State<LyricLine>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  late Animation<double> _blurAnimation;
  double _targetBlur = 0.0;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      duration: const Duration(milliseconds: 300),
      vsync: this,
    );
    _blurAnimation = Tween<double>(begin: 0.0, end: 0.0).animate(
      CurvedAnimation(
        parent: _controller,
        curve: Curves.easeInOut,
      ),
    );
    _updateBlurAnimation();
  }

  @override
  void didUpdateWidget(LyricLine oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.currentTimeMilliseconds != widget.currentTimeMilliseconds ||
        oldWidget.isActive != widget.isActive ||
        oldWidget.isPassed != widget.isPassed) {
      _updateBlurAnimation();
    }
  }

  void _updateBlurAnimation() {
    _targetBlur = _calculateTargetBlur();

    _blurAnimation = Tween<double>(
      begin: _blurAnimation.value,
      end: _targetBlur,
    ).animate(
      CurvedAnimation(
        parent: _controller,
        curve: Curves.easeInOut,
      ),
    );

    _controller.forward(from: 0.0);
  }

  double _calculateTargetBlur() {
    if (widget.isActive) return 0.0;

    final startTime = widget.sections.first.startTime;
    final endTime = widget.sections.last.endTime;

    final timeDiff = widget.isPassed
        ? widget.currentTimeMilliseconds - endTime
        : startTime - widget.currentTimeMilliseconds;

    final maxTimeDiff = 5000.0;
    return (timeDiff.clamp(0, maxTimeDiff) / maxTimeDiff) * 2.0;
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _blurAnimation,
      builder: (context, child) {
        return ImageFiltered(
          imageFilter: ImageFilter.blur(
            sigmaX: _blurAnimation.value,
            sigmaY: _blurAnimation.value,
          ),
          child: child,
        );
      },
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 4.0, horizontal: 16.0),
        child: Wrap(
          children: widget.sections.map((section) {
            return LyricSection(
              section: section,
              currentTimeMilliseconds: widget.currentTimeMilliseconds,
              isActive: widget.isActive,
              isPassed: widget.isPassed,
            );
          }).toList(),
        ),
      ),
    );
  }
}
