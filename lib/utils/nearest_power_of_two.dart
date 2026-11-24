import 'dart:math';

int nearestPowerOfTwo(int value) {
  if (value <= 0) return 1;
  return pow(2, (log(value) / log(2)).round()).toInt();
}
