import '../../bindings/bindings.dart';

void load(int index) async {
  LoadRequest(index: index).sendSignalToRust();
}
