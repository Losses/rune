import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/order_by_ids.dart';

void main() {
  test('items follow the order of the ids', () {
    expect(orderByIds([3, 1, 2], [1, 2, 3], (x) => x), [3, 1, 2]);
  });

  test('missing items are left out and repeated ids kept once', () {
    expect(orderByIds([4, 2, 2, 1], [1, 2], (x) => x), [2, 1]);
  });
}
