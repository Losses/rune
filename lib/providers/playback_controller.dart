import 'dart:convert';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../widgets/playback_controller/constants/controller_items.dart';

final SettingsManager settingsManager = SettingsManager();

/// [entries] in the order [storedOrder] gives them, then the ones it does not
/// have, in the order of [entries]: an entry added since the order was saved
/// goes to the end, after the hidden ones, rather than to the front of the
/// bar.
List<ControllerEntry> orderControllerEntries(
  List<ControllerEntry> entries,
  List<String> storedOrder,
) {
  int rank(ControllerEntry entry) {
    final index = storedOrder.indexOf(entry.id);
    return index < 0 ? storedOrder.length + entries.indexOf(entry) : index;
  }

  return [...entries]..sort((a, b) => rank(a).compareTo(rank(b)));
}

class PlaybackControllerProvider extends ChangeNotifier {
  static const String storageKey = 'controller_order';

  final List<ControllerEntry> _entries = List.from(controllerItems);

  PlaybackControllerProvider() {
    _loadEntries();
  }

  List<ControllerEntry> get entries => List.unmodifiable(_entries);

  void _loadEntries() async {
    String? storedOrderJson =
        await settingsManager.getValue<String>(storageKey);

    if (storedOrderJson != null) {
      List<dynamic> storedOrderDynamic = jsonDecode(storedOrderJson);
      List<String> storedOrder = List<String>.from(storedOrderDynamic);

      // Use a Map to store the last occurrence of each entry
      Map<String, ControllerEntry> entryMap = {
        for (var entry in controllerItems) entry.id: entry
      };

      // Clear duplicates, keeping only the last occurrence
      _entries
        ..clear()
        ..addAll(entryMap.values);

      // Sort according to storedOrder
      final ordered = orderControllerEntries(_entries, storedOrder);
      _entries
        ..clear()
        ..addAll(ordered);

      // Ensure all new entries are added to the end
      for (var item in controllerItems) {
        if (!_entries.any((entry) => entry.id == item.id)) {
          _entries.add(item);
        }
      }

      // Remove entries that no longer exist
      _entries.removeWhere(
          (entry) => !controllerItems.any((item) => item.id == entry.id));
    }

    notifyListeners();
  }

  void reorder(int oldIndex, int newIndex) {
    final item = _entries.removeAt(oldIndex);
    _entries.insert(newIndex, item);

    _saveEntries();
    notifyListeners();
  }

  void _saveEntries() {
    List<String> order = _entries.map((e) => e.id).toList();
    String orderJson = jsonEncode(order);
    settingsManager.setValue(storageKey, orderJson);
  }
}
