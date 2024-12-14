import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

class LyricLine extends StatelessWidget {
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

  double calculateProgress() {
    if (!isActive) return 0.0;
    if (isPassed) return 1.0;

    double totalDuration = 0;
    double currentProgress = 0;

    for (final section in sections) {
      final duration = section.endTime - section.startTime;
      totalDuration += duration;

      if (currentTimeMilliseconds >= section.endTime) {
        currentProgress += duration;
      } else if (currentTimeMilliseconds > section.startTime) {
        currentProgress += (currentTimeMilliseconds - section.startTime);
      }
    }

    return totalDuration > 0 ? (currentProgress / totalDuration) : 0.0;
  }

  @override
  Widget build(BuildContext context) {
    final progress = calculateProgress();
    final text = sections.map((s) => s.content).join("");
    final theme = FluentTheme.of(context);

    return Container(
      padding: const EdgeInsets.symmetric(vertical: 4.0, horizontal: 16.0),
      child: Stack(
        children: [
          Text(
            text,
            style: TextStyle(
              fontSize: 20,
              color: isPassed
                  ? theme.resources.textFillColorPrimary
                  : theme.resources.textFillColorPrimary.withAlpha(160),
            ),
          ),
          if (isActive)
            ClipRect(
              child: Align(
                alignment: Alignment.centerLeft,
                widthFactor: progress,
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
  }
}
