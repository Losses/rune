import '../../bindings/bindings.dart';
import '../../providers/status.dart';

/// Queues an unthrottled seek when there is a ready playing or paused track.
bool seekAbsolute(double positionSeconds, PlaybackStatusState? status) {
  if (status?.canSeek != true) return false;
  SeekRequest(positionSeconds: positionSeconds).sendSignalToRust();
  return true;
}
