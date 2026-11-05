import 'dart:convert';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../widgets/playback_controller/constants/controller_items.dart';

final SettingsManager settingsManager = SettingsManager();

class PlaybackControllerProvider extends ChangeNotifier {
  static const String storageKey = 'controller_order';

  final List<ControllerEntry> _entries = List.from(controllerItems);

  PlaybackControllerProvider() {
    _loadEntries();
  }

  List<ControllerEntry> get entries => List.unmodifiable(_entries);

  /// The entries that are on offer now, in order.
  List<ControllerEntry> availableEntries(BuildContext context) =>
      _entries.where((entry) => entry.isAvailable(context)).toList();

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
      _entries.sort((a, b) {
        int indexA = storedOrder.indexOf(a.id);
        int indexB = storedOrder.indexOf(b.id);
        return indexA.compareTo(indexB);
      });

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

  /// Moves an entry of [shown], a list of the available entries as shown,
  /// from [oldIndex] to [newIndex] in it, and keeps the others where they are.
  void reorderShown(List<ControllerEntry> shown, int oldIndex, int newIndex) {
    final item = shown[oldIndex];
    final rest = List.of(shown)..removeAt(oldIndex);
    _entries.remove(item);
    if (newIndex >= rest.length) {
      _entries.insert(
        rest.isEmpty ? 0 : _entries.indexOf(rest.last) + 1,
        item,
      );
    } else {
      _entries.insert(_entries.indexOf(rest[newIndex]), item);
    }

    _saveEntries();
    notifyListeners();
  }

  void _saveEntries() {
    List<String> order = _entries.map((e) => e.id).toList();
    String orderJson = jsonEncode(order);
    settingsManager.setValue(storageKey, orderJson);
  }
}
