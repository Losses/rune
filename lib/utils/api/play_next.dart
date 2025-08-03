import '../../bindings/bindings.dart';

void playNext() async {
  NextRequest().sendSignalToRust();
}
