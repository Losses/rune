import 'package:flutter/scheduler.dart';
import 'package:flutter/foundation.dart' as foundation;
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';

import 'lyric_line.dart';
import 'gradient_mask.dart';

class LyricsDisplay extends StatefulWidget {
  final List<LyricContentLine> lyrics;
  final int currentTimeMilliseconds;
  final List<int> activeLines;
  final foundation.ValueListenable<double> interpolatedTime;

  const LyricsDisplay({
    super.key,
    required this.lyrics,
    required this.currentTimeMilliseconds,
    required this.activeLines,
    required this.interpolatedTime,
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
  BoxConstraints? _lastConstraints;

  // Lines (indices into `lyrics`) the view is currently centred on.
  List<int> _targetLines = [];

  // Set while the view still has to be placed without animation (first
  // layout, size changes). It is consumed once the list reports positions.
  bool _snapPending = true;

  // The scroll offset the item positions were measured at. The list does not
  // report new positions after every jump, so they can be older than the
  // current offset.
  double? _measuredScrollOffset;

  @override
  void initState() {
    super.initState();
    _targetLines = _computeTargetLines();
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

    final lyricsChanged = !identical(oldWidget.lyrics, widget.lyrics);
    final targetLines = _computeTargetLines();

    // Nothing active and nothing upcoming (after the last line): stay put.
    // Lines of other lyrics are no place to stay, though.
    if (targetLines.isEmpty) {
      if (lyricsChanged) _targetLines = const [];
      return;
    }

    if (lyricsChanged || !foundation.listEquals(_targetLines, targetLines)) {
      _targetLines = targetLines;
      _scheduleScroll();
    }
  }

  /// The lines to centre: the active ones, or, when nothing is active, the
  /// next upcoming line (the first line before the song starts, the next line
  /// in a gap). Empty after the last line.
  List<int> _computeTargetLines() {
    if (widget.activeLines.isNotEmpty) return widget.activeLines;

    final cT = widget.currentTimeMilliseconds;
    for (final (index, line) in widget.lyrics.indexed) {
      if (line.startTime > cT) return [index];
    }
    return const [];
  }

  void _scheduleScroll() {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _scrollToTargetLines(animate: true);
    });
  }

  void _offsetListener() {
    final scrollController = _itemScrollController.scrollController;
    if (scrollController != null && scrollController.hasClients) {
      _measuredScrollOffset = scrollController.offset;
    }

    if (!_snapPending || !mounted) return;
    if (_itemPositionsListener.itemPositions.value.isEmpty) return;

    if (_scrollToTargetLines(animate: false)) {
      _snapPending = false;
    }
  }

  /// Centres the target lines in the viewport. Returns true when the exact
  /// position could be applied from measured item positions.
  bool _scrollToTargetLines({required bool animate}) {
    if (_targetLines.isEmpty || !_itemScrollController.isAttached) {
      return false;
    }

    final scrollController = _itemScrollController.scrollController;
    if (scrollController == null || !scrollController.hasClients) {
      return false;
    }

    // Item 0 of the list is padding, so line `i` is list item `i + 1`.
    final visible = {
      for (final item in _itemPositionsListener.itemPositions.value)
        item.index: item,
    };

    final allVisible = _targetLines.every((i) => visible.containsKey(i + 1));

    if (!allVisible) {
      // Far away: only fresh positions are reliable, so jump by index.
      final index = _centreIndex();
      if (animate) {
        _itemScrollController
            .scrollTo(
              index: index,
              duration: const Duration(milliseconds: 300),
              curve: Curves.easeInOut,
              viewportAlignment: 0.5,
              edgeAlignment: 0.5,
            )
            .ignore();
      } else {
        // Leading edge at the middle; refined once the line is measured.
        _itemScrollController.jumpTo(index: index, alignment: 0.5);
      }
      return false;
    }

    // `offset` is the item's top-left relative to the viewport, as it was at
    // `_measuredScrollOffset`.
    double totalCentre = 0;
    for (final i in _targetLines) {
      final item = visible[i + 1]!;
      totalCentre += item.offset.dy + item.itemSize / 2;
    }

    final position = scrollController.position;
    final averageCentre = totalCentre / _targetLines.length;
    final scrollOffset =
        (_measuredScrollOffset ?? scrollController.offset) +
            (averageCentre - position.viewportDimension / 2);
    final clamped = scrollOffset.clamp(0.0, position.maxScrollExtent);

    if (animate) {
      scrollController.animateTo(
        clamped,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );
    } else {
      scrollController.jumpTo(clamped);
    }
    return true;
  }

  /// The list item in the middle of the target lines.
  int _centreIndex() {
    final average = _targetLines.reduce((a, b) => a + b) / _targetLines.length;
    return average.round() + 1;
  }

  Widget _buildLyricsList(BoxConstraints constraints, bool isMini) {
    if (_lastConstraints != constraints) {
      _lastConstraints = constraints;
      _snapPending = true;
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
        initialScrollIndex:
            _targetLines.isEmpty ? 0 : _centreIndex(),
        initialAlignment: _targetLines.isEmpty ? 0 : 0.5,
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
              interpolatedTime: widget.interpolatedTime,
              isActive: widget.activeLines.contains(actualIndex),
              isPassed: eT <= cT,
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
        // A minimized window on Windows is 0 x 0. The list divides by its
        // viewport size and is left with non-finite offsets, so drop it and
        // build a fresh one, placed on the current line, once the window is
        // back.
        if (constraints.maxWidth == 0 || constraints.maxHeight == 0) {
          _measuredScrollOffset = null;
          _lastConstraints = null;
          return const SizedBox.shrink();
        }

        return GradientMask(
          child: _buildLyricsList(constraints, isMini),
        );
      },
    );
  }
}
