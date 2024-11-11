import '../../messages/all.dart';

void playNext() async {
  NextRequest().sendSignalToRust();
}
