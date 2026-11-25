import '../../bindings/bindings.dart';

void playPrevious() async {
  PreviousRequest().sendSignalToRust();
}
