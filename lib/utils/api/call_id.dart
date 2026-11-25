import '../../bindings/bindings.dart';

var _nextCallIdValue = DateTime.now().microsecondsSinceEpoch;

Uint64 nextCallId() => Uint64.fromBigInt(BigInt.from(_nextCallIdValue++));
