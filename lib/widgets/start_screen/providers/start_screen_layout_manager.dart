import 'package:fluent_ui/fluent_ui.dart';

class StartScreenLayoutManager with ChangeNotifier {
  final Map<String, StartGroupItemData> _items = {};

  void registerItem(StartGroupItemData data) {
    print('REGISTERED, ${data.groupId}, ${data.row} x ${data.column}');
    final key = _generateKey(data.groupId, data.row, data.column);
    _items[key] = data;
  }

  void unregisterItem(int groupId, int row, int column) {
    final key = _generateKey(groupId, row, column);
    _items.remove(key);
  }

  StartGroupItemData? getItem(int groupId, int row, int column) {
    final key = _generateKey(groupId, row, column);
    return _items[key];
  }

  void startAnimation(int groupId, int row, int column) {
    final item = getItem(groupId, row, column);
    item?.startAnimation();
  }

  String _generateKey(int groupId, int row, int column) {
    return 'g$groupId-$column:$row';
  }
}

class StartGroupItemData {
  final int groupId;
  final int row;
  final int column;
  final VoidCallback startAnimation;

  StartGroupItemData({
    required this.groupId,
    required this.row,
    required this.column,
    required this.startAnimation,
  });
}
