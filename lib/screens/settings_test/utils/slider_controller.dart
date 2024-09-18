import 'package:fluent_ui/fluent_ui.dart';

class SliderController extends ChangeNotifier {
  double _value;

  SliderController(this._value);

  double get value => _value;

  set value(double newValue) {
    if (_value != newValue) {
      _value = newValue;
      notifyListeners();
    }
  }
}
