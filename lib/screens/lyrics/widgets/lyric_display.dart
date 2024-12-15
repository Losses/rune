import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import 'lyric_line.dart';
import 'gradient_mask.dart';

class LyricsDisplay extends StatefulWidget {
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
  State<LyricsDisplay> createState() => _LyricsDisplayState();
}

class _LyricsDisplayState extends State<LyricsDisplay> {
  final ScrollController _scrollController = ScrollController();
  final Map<int, GlobalKey> _lineKeys = {};

  @override
  void initState() {
    super.initState();
    _initLineKeys();
  }

  void _initLineKeys() {
    for (int i = 0; i < widget.lyrics.length; i++) {
      _lineKeys[i] = GlobalKey();
    }
  }

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  @override
  void didUpdateWidget(LyricsDisplay oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!listEquals(oldWidget.activeLines, widget.activeLines)) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _scrollToActiveLines();
      });
    }
  }

  void _scrollToActiveLines() {
    if (widget.activeLines.isEmpty || !_scrollController.hasClients) return;

    // Calculate the average position of active lines
    double totalOffset = 0;
    int count = 0;

    for (int index in widget.activeLines) {
      final RenderBox? renderBox =
          _lineKeys[index]?.currentContext?.findRenderObject() as RenderBox?;
      if (renderBox == null) continue;

      final RenderBox? listRenderBox = context.findRenderObject() as RenderBox?;
      if (listRenderBox == null) continue;

      final Offset offset =
          renderBox.localToGlobal(Offset.zero, ancestor: listRenderBox);
      totalOffset += offset.dy + renderBox.size.height / 2;
      count++;
    }

    if (count == 0) return;

    final double averagePosition = totalOffset / count;
    final double containerMiddle =
        _scrollController.position.viewportDimension / 2;
    final double scrollOffset =
        _scrollController.offset + (averagePosition - containerMiddle);

    _scrollController.animateTo(
      max(0, min(scrollOffset, _scrollController.position.maxScrollExtent)),
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeInOut,
    );
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final double paddingSize = constraints.maxHeight / 2;
        return GradientMask(
          child: ListView.builder(
            controller: _scrollController,
            itemCount: widget.lyrics.length + 2, // Add 2 for padding items
            itemBuilder: (context, index) {
              if (index == 0 || index == widget.lyrics.length + 1) {
                return SizedBox(height: paddingSize);
              }

              final actualIndex = index - 1;
              final line = widget.lyrics[actualIndex];
              return RepaintBoundary(
                key: _lineKeys[actualIndex],
                child: LyricLine(
                  sections: line.sections,
                  currentTimeMilliseconds: widget.currentTimeMilliseconds,
                  isActive: widget.activeLines.contains(actualIndex),
                  isPassed: line.endTime < widget.currentTimeMilliseconds,
                ),
              );
            },
          ),
        );
      },
    );
  }
}
