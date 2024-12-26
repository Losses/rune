import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/api/seek_absolute.dart';
import '../../../messages/all.dart';

import 'lyric_section.dart';
import 'simple_lyric_section.dart';

class LyricLine extends StatefulWidget {
  final List<LyricContentLineSection> sections;
  final int currentTimeMilliseconds;
  final bool isActive;
  final bool isPassed;
  final bool isStatic;

  const LyricLine({
    super.key,
    required this.sections,
    required this.currentTimeMilliseconds,
    required this.isActive,
    required this.isPassed,
    required this.isStatic,
  });

  @override
  State<LyricLine> createState() => _LyricLineState();
}

class _LyricLineState extends State<LyricLine>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  late Animation<double> _blurAnimation;
  late Animation<double> _opacityAnimation;
  double _targetBlur = 0.0;
  double _targetOpacity = 1.0;
  bool _isHovered = false;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      duration: const Duration(milliseconds: 500),
      vsync: this,
    );
    _blurAnimation = Tween<double>(begin: 0.0, end: 0.0).animate(
      CurvedAnimation(
        parent: _controller,
        curve: Curves.easeInOut,
      ),
    );
    _opacityAnimation = Tween<double>(begin: 1.0, end: 1.0).animate(
      CurvedAnimation(
        parent: _controller,
        curve: Curves.easeInOut,
      ),
    );
    _updateAnimations();
  }

  @override
  void didUpdateWidget(LyricLine oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.currentTimeMilliseconds != widget.currentTimeMilliseconds ||
        oldWidget.isActive != widget.isActive ||
        oldWidget.isPassed != widget.isPassed) {
      _updateAnimations();
    }
  }

  void _updateAnimations() {
    _targetBlur = _isHovered ? 0 : _calculateTargetBlur();
    _targetOpacity = _isHovered ? 0.9 : 1.0;

    _blurAnimation = Tween<double>(
      begin: _blurAnimation.value,
      end: _targetBlur,
    ).animate(
      CurvedAnimation(
        parent: _controller,
        curve: Curves.easeInOut,
      ),
    );

    _opacityAnimation = Tween<double>(
      begin: _opacityAnimation.value,
      end: _targetOpacity,
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
    return (timeDiff.clamp(0, maxTimeDiff) / maxTimeDiff) * 3.0;
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      onEnter: (_) {
        if (mounted) {
          setState(() {
            _isHovered = true;
            _updateAnimations();
          });
        }
      },
      onExit: (_) {
        if (mounted) {
          setState(() {
            _isHovered = false;
            _updateAnimations();
          });
        }
      },
      child: GestureDetector(
        onTap: () {
          seekAbsolute(widget.sections.first.startTime);
        },
        child: AnimatedBuilder(
          animation: Listenable.merge([_blurAnimation, _opacityAnimation]),
          builder: (context, child) {
            return Opacity(
              opacity: _opacityAnimation.value,
              child: ImageFiltered(
                imageFilter: ImageFilter.blur(
                  sigmaX: _blurAnimation.value,
                  sigmaY: _blurAnimation.value,
                ),
                child: child,
              ),
            );
          },
          child: Container(
            padding:
                const EdgeInsets.symmetric(vertical: 10.0, horizontal: 16.0),
            child: widget.isStatic
                ? Wrap(
                    children: widget.sections.indexed.map((section) {
                      return SimpleLyricSection(
                        key: ValueKey(section.$1),
                        section: section.$2,
                        isPassed: widget.isPassed,
                      );
                    }).toList(),
                  )
                : Wrap(
                    children: widget.sections.indexed.map((section) {
                      return LyricSection(
                        key: ValueKey(section.$1),
                        section: section.$2,
                        currentTimeMilliseconds: widget.currentTimeMilliseconds,
                        isActive: widget.isActive,
                        isPassed: widget.isPassed,
                      );
                    }).toList(),
                  ),
          ),
        ),
      ),
    );
  }
}
