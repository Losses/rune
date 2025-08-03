import '../../bindings/bindings.dart';

void seekAbsolute(int positionMs) {
  SeekRequest(
    positionSeconds: (positionMs / 1000),
  ).sendSignalToRust();
}
