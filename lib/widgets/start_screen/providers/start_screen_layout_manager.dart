import 'dart:math';
import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

class StartGroupItemData {
  final String key;
  final int groupId;
  final int row;
  final int column;
  final double distance;
  final VoidCallback startAnimation;
  bool played;

  StartGroupItemData({
    required this.key,
    required this.groupId,
    required this.row,
    required this.column,
    required this.distance,
    required this.startAnimation,
    this.played = false,
  });
}

class StartItemRegisterResult {
  final bool skipAnimation;
  final StartGroupItemData? data;

  StartItemRegisterResult({
    required this.skipAnimation,
    required this.data,
  });
}

class StartScreenLayoutManager with ChangeNotifier {
  final Map<String, StartGroupItemData> _items = {};
  final Map<int, Size> _groupSizes = {};
  double _currentAnimationDistance = 0;
  bool _animationFinished = false;
  Timer? _animationTimer;

  StartItemRegisterResult registerItem(
      int groupId, int row, int column, VoidCallback startAnimation,
      [String? prefix]) {
    if (_animationFinished) {
      return StartItemRegisterResult(skipAnimation: true, data: null);
    }

    final key = _generateKey(groupId, row, column, prefix);

    // Update group size
    _updateGroupSize(groupId, row, column);

    // Calculate distance from the top-left corner of the group
    final distance = _calculateDistance(groupId, row, column);

    _items[key] = StartGroupItemData(
        key: key,
        groupId: groupId,
        row: row,
        column: column,
        distance: distance,
        startAnimation: startAnimation);

    // If the current animation index has already exceeded this element's index, it has missed the animation
    return StartItemRegisterResult(
        skipAnimation:
            (_currentAnimationDistance > distance || _animationFinished),
        data: _items[key]);
  }

  void unregisterItem(StartGroupItemData data) {
    if (_animationFinished) return;

    _items.remove(data.key);

    // Recalculate group size when an item is removed
    _recalculateGroupSize(data.groupId);
  }

  StartGroupItemData? getItem(int groupId, int row, int column) {
    final key = _generateKey(groupId, row, column);
    return _items[key];
  }

  String _generateKey(int groupId, int row, int column, [String? prefix]) {
    return '${prefix ?? "g"}$groupId-$column:$row';
  }

  void playAnimations([double speed = 0.3]) {
    print("_animationFinished: $_animationFinished, _animationTimer: $_animationTimer");
    if (_animationFinished) return;
    // This means the animation is already playing
    if (_animationTimer != null) return;

    // Calculate the maximum distance
    double maxDistance = 0;
    _items.forEach((key, item) {
      if (item.distance > maxDistance) {
        maxDistance = item.distance;
      }
    });

    // Increase the animation distance by speed every frame
    _animationTimer = Timer.periodic(const Duration(milliseconds: 16), (timer) {
      _currentAnimationDistance += speed;

      // Find elements whose distance is less than or equal to _currentAnimationDistance and have not played their animation
      _items.forEach((key, item) {
        if ((item.distance < _currentAnimationDistance ||
                _currentAnimationDistance > maxDistance) &&
            !item.played) {
          item.startAnimation();
          item.played = true;
        }
      });

      // Continue playing animations until all elements have played
      if (_currentAnimationDistance > maxDistance) {
        // Set _animationFinished to true
        _animationFinished = true;
        timer.cancel();
      }
    });
  }

  void resetAnimations() {
    _currentAnimationDistance = 0;
    _animationFinished = false;
    _animationTimer?.cancel();
    _animationTimer = null;
    _items.forEach((_, item) {
      item.played = false;
    });
  }

  void _updateGroupSize(int groupId, int row, int column) {
    final currentSize = _groupSizes[groupId] ?? const Size(0, 0);
    final newWidth = max(currentSize.width, column.toDouble());
    final newHeight = max(currentSize.height, row.toDouble());
    _groupSizes[groupId] = Size(newWidth, newHeight);
  }

  void _recalculateGroupSize(int groupId) {
    double maxWidth = 0;
    double maxHeight = 0;

    _items.forEach((key, item) {
      if (item.groupId == groupId) {
        maxWidth = max(maxWidth, item.column.toDouble());
        maxHeight = max(maxHeight, item.row.toDouble());
      }
    });

    _groupSizes[groupId] = Size(maxWidth, maxHeight);
  }

  double _calculateDistance(int groupId, int row, int column) {
    double offsetX = 0;
    double offsetY = 0;

    _groupSizes.forEach((id, size) {
      if (id < groupId) {
        offsetX += size.width;
        offsetY += size.height;
      }
    });

    final adjustedRow = row + offsetY;
    final adjustedColumn = column + offsetX;

    return sqrt(adjustedRow * adjustedRow + adjustedColumn * adjustedColumn);
  }

  @override
  void dispose() {
    super.dispose();
    cleanup();
  }

  void cleanup() {
    _animationTimer?.cancel();
    _items.clear();
    _groupSizes.clear();
  }
}
