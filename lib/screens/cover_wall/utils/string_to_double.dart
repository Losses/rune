import 'package:hashlib/hashlib.dart';

final maxHashValue = BigInt.from(1) << 64;

double stringToDouble(String input) {
  var hash = xxh3.string(input).bigInt();

  return hash / maxHashValue;
}
