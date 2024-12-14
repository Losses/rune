import 'package:fluent_ui/fluent_ui.dart';
import '../../../messages/all.dart';

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
  void didUpdateWidget(LyricLine oldWidget) {
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

    double totalDuration = 0;
    double currentProgress = 0;

    for (final section in widget.sections) {
      final duration = section.endTime - section.startTime;
      totalDuration += duration;

      if (widget.currentTimeMilliseconds >= section.endTime) {
        currentProgress += duration;
      } else if (widget.currentTimeMilliseconds > section.startTime) {
        currentProgress += (widget.currentTimeMilliseconds - section.startTime);
      }
    }

    return totalDuration > 0 ? (currentProgress / totalDuration) : 0.0;
  }

  @override
  Widget build(BuildContext context) {
    final text = widget.sections.map((s) => s.content).join("");
    final theme = FluentTheme.of(context);

    return AnimatedBuilder(
      animation: _animation,
      builder: (context, child) {
        return Container(
          padding: const EdgeInsets.symmetric(vertical: 4.0, horizontal: 16.0),
          child: Stack(
            children: [
              Text(
                text,
                style: TextStyle(
                  fontSize: 20,
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
                      text,
                      style: TextStyle(
                        fontSize: 20,
                        color: theme.resources.textFillColorPrimary,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}
