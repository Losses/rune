import 'dart:math';
import 'package:flutter/foundation.dart' as foundation;
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../../utils/rune_log.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

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
  final ItemScrollController _itemScrollController = ItemScrollController();
  final ScrollOffsetController _scrollOffsetController =
      ScrollOffsetController();
  final ItemPositionsListener _itemPositionsListener =
      ItemPositionsListener.create();
  final Map<int, (double, double)> _lineOffsets = {};
  BoxConstraints? _lastConstraints;

  @override
  void initState() {
    super.initState();
    _itemPositionsListener.itemPositions.addListener(_offsetListener);
  }

  @override
  void dispose() {
    _itemPositionsListener.itemPositions.removeListener(_offsetListener);
    super.dispose();
  }

  @override
  void didUpdateWidget(LyricsDisplay oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!foundation.listEquals(oldWidget.activeLines, widget.activeLines)) {
      _scrollToActiveLines();
    }
  }

  void _offsetListener() {
    for (final item in _itemPositionsListener.itemPositions.value) {
      _lineOffsets[item.index] =
          (item.offset.dy + item.itemSize, item.itemSize);
    }
  }

  void _scrollToActiveLinesById() async {
    if (widget.activeLines.isEmpty) return;

    final double averageId =
        widget.activeLines.reduce((a, b) => a + b) / widget.activeLines.length;

    await _itemScrollController.scrollTo(
      index: averageId.round(),
      duration: const Duration(milliseconds: 1),
      curve: Curves.easeInOut,
      edgeAlignment: 0.5,
      viewportAlignment: 0.5,
    );

    await Future.delayed(Duration(milliseconds: 100));

    _scrollToActiveLines(true);
  }

  void _scrollToActiveLines([bool disableScrollToLine = false]) {
    if (widget.activeLines.isEmpty) return;

    final allOffsetsAvailable =
        widget.activeLines.every((index) => _lineOffsets.containsKey(index));

    if (!allOffsetsAvailable) {
      if (!disableScrollToLine) {
        _scrollToActiveLinesById();
      }
      warn$("No active line found!");
      return;
    }

    double totalOffset = 0;
    for (int index in widget.activeLines) {
      final renderBox = _lineOffsets[index]!;
      totalOffset += renderBox.$1 + renderBox.$2 / 2;
    }

    final scrollController = _itemScrollController.scrollController;

    if (scrollController == null) {
      _scrollToActiveLinesById();
      return;
    }

    if (widget.activeLines.length == 1 && widget.activeLines[0] == 0) {
      scrollController.animateTo(
        0,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );

      return;
    }

    final viewportDimension = scrollController.position.viewportDimension;
    final maxScrollExtent = scrollController.position.maxScrollExtent;

    final averageOffset = totalOffset / widget.activeLines.length;
    final containerMiddle = viewportDimension / 2;
    final scrollOffset =
        scrollController.offset + (averageOffset - containerMiddle);

    scrollController.animateTo(
      max(0, min(scrollOffset, maxScrollExtent)),
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeInOut,
    );
  }

  Widget _buildLyricsList(BoxConstraints constraints, bool isMini) {
    if (_lastConstraints != constraints) {
      _lastConstraints = constraints;
      _scrollToActiveLines();
    }

    final double paddingSize =
        (isMini ? constraints.maxWidth : constraints.maxHeight) / 2;

    return ScrollConfiguration(
      behavior: ScrollConfiguration.of(context).copyWith(scrollbars: false),
      child: ScrollablePositionedList.builder(
        physics: const NeverScrollableScrollPhysics(),
        itemScrollController: _itemScrollController,
        itemPositionsListener: _itemPositionsListener,
        scrollOffsetController: _scrollOffsetController,
        itemCount: widget.lyrics.length + 2,
        itemBuilder: (context, index) {
          if (index == 0 || index == widget.lyrics.length + 1) {
            return SizedBox(height: paddingSize);
          }

          final actualIndex = index - 1;
          final line = widget.lyrics[actualIndex];
          final eT = line.endTime;
          final sT = line.startTime;
          final cT = widget.currentTimeMilliseconds;

          return RepaintBoundary(
            child: LyricLine(
              sections: line.sections,
              currentTimeMilliseconds: cT,
              isActive: widget.activeLines.contains(actualIndex),
              isPassed: eT < cT,
              isStatic: eT + 500 < cT || sT - 500 > cT,
            ),
          );
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.dock, false);

    return LayoutBuilder(
      builder: (context, constraints) {
        return GradientMask(
          child: _buildLyricsList(constraints, isMini),
        );
      },
    );
  }
}
