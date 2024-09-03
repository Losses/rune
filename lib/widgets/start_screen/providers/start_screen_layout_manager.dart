import 'dart:math';
import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

class StartGroupItemData {
  final int groupId;
  final int row;
  final int column;
  final double distance;
  final VoidCallback startAnimation;
  bool played;

  StartGroupItemData({
    required this.groupId,
    required this.row,
    required this.column,
    required this.distance,
    required this.startAnimation,
    this.played = false,
  });
}

class StartScreenLayoutManager with ChangeNotifier {
  final Map<String, StartGroupItemData> _items = {};
  final Map<int, Size> _groupSizes = {};
  double _currentAnimationDistance = 0;
  bool _animationFinished = false;

  bool registerItem(
      int groupId, int row, int column, VoidCallback startAnimation) {
    final key = _generateKey(groupId, row, column);

    // Update group size
    _updateGroupSize(groupId, row, column);

    // Calculate distance from the top-left corner of the group
    final distance = _calculateDistance(row, column);

    _items[key] = StartGroupItemData(
        groupId: groupId,
        row: row,
        column: column,
        distance: distance,
        startAnimation: startAnimation);

    // If the current animation index has already exceeded this element's index, it has missed the animation
    return _currentAnimationDistance > distance || _animationFinished;
  }

  void unregisterItem(int groupId, int row, int column) {
    final key = _generateKey(groupId, row, column);
    _items.remove(key);
    _recalculateGroupSize(
        groupId); // Recalculate group size when an item is removed
  }

  StartGroupItemData? getItem(int groupId, int row, int column) {
    final key = _generateKey(groupId, row, column);
    return _items[key];
  }

  String _generateKey(int groupId, int row, int column) {
    return 'g$groupId-$column:$row';
  }

  Timer? _animationTimer;

  void playAnimations(double speed) {
    // Calculate the maximum distance
    double maxDistance = 0;
    _items.forEach((key, item) {
      if (item.distance > maxDistance) {
        maxDistance = item.distance;
      }
    });

    if (_animationTimer != null) return;

    // Increase the animation distance by speed every frame
    _animationTimer = Timer.periodic(const Duration(milliseconds: 16), (timer) {
      _currentAnimationDistance += speed;

      // Find elements whose distance is less than or equal to _currentAnimationDistance and have not played their animation
      _items.forEach((key, item) {
        if (item.distance <= _currentAnimationDistance && !item.played) {
          item.startAnimation();
          item.played = true;
        }
      });

      // Continue playing animations until all elements have played
      if (_currentAnimationDistance >= maxDistance) {
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

  double _calculateDistance(int row, int column) {
    return sqrt(row * row + column * column);
  }
}
