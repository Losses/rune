import '../../messages/all.dart';

void load(int index) async {
  LoadRequest(index: index).sendSignalToRust();
}
