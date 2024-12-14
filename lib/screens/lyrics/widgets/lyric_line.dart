import 'package:fluent_ui/fluent_ui.dart';

import 'lyric_section.dart';

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

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 4.0, horizontal: 16.0),
      child: Wrap(
        children: sections.map((section) {
          return LyricSection(
            section: section,
            currentTimeMilliseconds: currentTimeMilliseconds,
            isActive: isActive,
            isPassed: isPassed,
          );
        }).toList(),
      ),
    );
  }
}
