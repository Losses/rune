import 'dart:convert';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/rune_log.dart';
import '../utils/settings_manager.dart';
import '../screens/settings_library_home/constants/library_home_items.dart';

final SettingsManager settingsManager = SettingsManager();

class LibraryHomeEntryWithValue {
  final String id;
  final String? value;
  final LibraryHomeEntry definition;

  LibraryHomeEntryWithValue({
    required this.id,
    this.value,
    required this.definition,
  });

  factory LibraryHomeEntryWithValue.fromEntry(LibraryHomeEntry entry,
      [String? value]) {
    return LibraryHomeEntryWithValue(
      id: entry.id,
      value: value,
      definition: entry,
    );
  }

  String serialize() {
    if (value == null || value!.isEmpty) {
      return id;
    }
    return '$id::$value';
  }

  static LibraryHomeEntryWithValue? deserialize(
      String serialized, List<LibraryHomeEntry> availableEntries) {
    final parts = serialized.split('::');
    final id = parts[0];
    final value = parts.length > 1 ? parts[1] : null;

    final originalEntry = availableEntries.firstWhere(
      (entry) => entry.id == id,
      orElse: () => throw Exception('Entry not found: $id'),
    );

    return LibraryHomeEntryWithValue(
      id: id,
      value: value,
      definition: originalEntry,
    );
  }

  @override
  String toString() {
    return serialize();
  }
}

class LibraryHomeProvider extends ChangeNotifier {
  static const String storageKey = 'library_home';
  final List<LibraryHomeEntryWithValue> _entries = [];

  LibraryHomeProvider() {
    _loadEntries();
  }

  List<LibraryHomeEntryWithValue> get entries => List.unmodifiable(_entries);

  void _loadEntries() async {
    String? storedOrderJson =
        await settingsManager.getValue<String>(storageKey);

    if (storedOrderJson != null) {
      List<dynamic> storedOrderDynamic = jsonDecode(storedOrderJson);
      List<String> storedSerializedEntries =
          List<String>.from(storedOrderDynamic);

      Map<String, LibraryHomeEntryWithValue> entryMap = {};

      for (final serialized in storedSerializedEntries) {
        try {
          final entry = LibraryHomeEntryWithValue.deserialize(
            serialized,
            libraryHomeItems,
          );
          if (entry != null) {
            entryMap[entry.id] = entry;
          }
        } catch (e) {
          error$('Error deserializing entry: $serialized');
        }
      }
      _entries
        ..clear()
        ..addAll(entryMap.values);

      for (final item in libraryHomeItems) {
        if (!_entries.any((entry) => entry.id == item.id)) {
          _entries.add(LibraryHomeEntryWithValue.fromEntry(item));
        }
      }

      _entries.removeWhere(
        (entry) => !libraryHomeItems.any((item) => item.id == entry.id),
      );
    } else {
      _entries.addAll(
        libraryHomeItems
            .map((item) => LibraryHomeEntryWithValue.fromEntry(item)),
      );
    }

    notifyListeners();
  }

  void reorder(int oldIndex, int newIndex) {
    final item = _entries.removeAt(oldIndex);
    _entries.insert(newIndex, item);

    _saveEntries();
    notifyListeners();
  }

  void updateEntryValue(String id, String? value) {
    final index = _entries.indexWhere((entry) => entry.id == id);
    if (index != -1) {
      final originalEntry = _entries[index].definition;
      _entries[index] =
          LibraryHomeEntryWithValue.fromEntry(originalEntry, value);
      _saveEntries();
      notifyListeners();
    }
  }

  void _saveEntries() {
    List<String> serializedEntries =
        _entries.map((e) => e.serialize()).toList();
    String orderJson = jsonEncode(serializedEntries);
    settingsManager.setValue(storageKey, orderJson);
  }
}
