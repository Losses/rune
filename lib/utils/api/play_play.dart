import '../../bindings/bindings.dart';

void playPlay() async {
  PlayRequest().sendSignalToRust();
}
