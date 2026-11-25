class RandomGridPlacement {
  final int coverIndex;
  final int col;
  final int row;
  final int size;

  const RandomGridPlacement({
    required this.coverIndex,
    required this.col,
    required this.row,
    required this.size,
  });

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other is! RandomGridPlacement) return false;
    return coverIndex == other.coverIndex &&
        col == other.col &&
        row == other.row &&
        size == other.size;
  }

  @override
  int get hashCode => Object.hash(coverIndex, col, row, size);

  @override
  String toString() =>
      'RandomGridPlacement(coverIndex: $coverIndex, col: $col, row: $row, size: $size)';
}
