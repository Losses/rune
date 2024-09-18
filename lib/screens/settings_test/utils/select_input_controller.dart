import 'package:fluent_ui/fluent_ui.dart';

class SelectInputController extends ChangeNotifier {
  String? _selectedValue;

  SelectInputController(this._selectedValue);

  String? get selectedValue => _selectedValue;

  set selectedValue(String? value) {
    if (_selectedValue != value) {
      _selectedValue = value;
      notifyListeners();
    }
  }
}
