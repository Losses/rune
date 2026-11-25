import 'package:fluent_ui/fluent_ui.dart';

class ToggleSwitchController extends ChangeNotifier {
  bool _isChecked;

  ToggleSwitchController([bool initialChecked = false])
      : _isChecked = initialChecked;

  bool get isChecked => _isChecked;

  set isChecked(bool value) {
    if (_isChecked != value) {
      _isChecked = value;
      notifyListeners();
    }
  }
}
