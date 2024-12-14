import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import 'lyric_line.dart';

class LyricsDisplay extends StatelessWidget {
  final List<LyricContentLine> lyrics;
  final int currentTimeMilliseconds;
  final List<int> activeLines;

  const LyricsDisplay({
    super.key,
    required this.lyrics,
    required this.currentTimeMilliseconds,
    required this.activeLines,
  });

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      itemCount: lyrics.length,
      itemBuilder: (context, index) {
        final line = lyrics[index];
        return LyricLine(
          key: ValueKey(index),
          sections: line.sections,
          currentTimeMilliseconds: currentTimeMilliseconds,
          isActive: activeLines.contains(index),
          isPassed: line.endTime < currentTimeMilliseconds,
        );
      },
    );
  }
}
