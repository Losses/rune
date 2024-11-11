import '../../messages/all.dart';

void playPrevious() async {
  PreviousRequest().sendSignalToRust();
}
