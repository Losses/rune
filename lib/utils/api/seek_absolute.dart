import '../../messages/all.dart';

void seekAbsolute(int positionMs) {
  SeekRequest(
    positionSeconds: (positionMs / 1000),
  ).sendSignalToRust();
}
