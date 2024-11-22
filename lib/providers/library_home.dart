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
      value: value ?? entry.defaultValue,
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

  /// Provides an immutable view of the list of entries
  List<LibraryHomeEntryWithValue> get entries => List.unmodifiable(_entries);

  /// Safely updates the list of entries
  void _safelyUpdateEntries(List<LibraryHomeEntryWithValue> newEntries) {
    // Create a map based on IDs to remove duplicates, keeping the last occurrence
    final uniqueEntries = <String, LibraryHomeEntryWithValue>{};

    // Add entries, overwriting duplicates to keep the last one
    for (final entry in newEntries) {
      uniqueEntries[entry.id] = entry;
    }

    // Ensure all required entries exist, filling missing ones with default values
    for (final item in libraryHomeItems) {
      if (!uniqueEntries.containsKey(item.id)) {
        uniqueEntries[item.id] = LibraryHomeEntryWithValue.fromEntry(item);
      }
    }

    // Remove entries not in libraryHomeItems
    uniqueEntries.removeWhere(
        (key, _) => !libraryHomeItems.any((item) => item.id == key));

    // Update the list of entries
    _entries
      ..clear()
      ..addAll(uniqueEntries.values);
  }

  /// Loads entries from storage
  Future<void> _loadEntries() async {
    try {
      final String? storedOrderJson =
          await settingsManager.getValue<String>(storageKey);

      if (storedOrderJson != null) {
        // Parse stored data
        final List<LibraryHomeEntryWithValue> loadedEntries =
            _parseStoredEntries(storedOrderJson);
        _safelyUpdateEntries(loadedEntries);
      } else {
        // Use default values if no stored data
        _safelyUpdateEntries(libraryHomeItems
            .map((item) => LibraryHomeEntryWithValue.fromEntry(item))
            .toList());
      }
    } catch (e, stackTrace) {
      error$('Error loading library home entries: $e\n$stackTrace');
      // Use default values on error
      _safelyUpdateEntries(libraryHomeItems
          .map((item) => LibraryHomeEntryWithValue.fromEntry(item))
          .toList());
    } finally {
      notifyListeners();
    }
  }

  /// Parses stored entries
  List<LibraryHomeEntryWithValue> _parseStoredEntries(String storedOrderJson) {
    final List<dynamic> storedOrderDynamic = jsonDecode(storedOrderJson);
    final List<String> storedSerializedEntries =
        List<String>.from(storedOrderDynamic);
    final List<LibraryHomeEntryWithValue> loadedEntries = [];

    for (final serialized in storedSerializedEntries) {
      try {
        final entry =
            LibraryHomeEntryWithValue.deserialize(serialized, libraryHomeItems);
        if (entry != null) {
          loadedEntries.add(entry);
        }
      } catch (e) {
        error$('Error deserializing entry: $serialized');
        // Continue processing other entries
      }
    }

    return loadedEntries;
  }

  /// Reorders entries
  void reorder(int oldIndex, int newIndex) {
    if (oldIndex < 0 ||
        oldIndex >= _entries.length ||
        newIndex < 0 ||
        newIndex >= _entries.length) {
      return;
    }

    final item = _entries.removeAt(oldIndex);
    _entries.insert(newIndex, item);

    _saveEntries();
    notifyListeners();
  }

  /// Updates entry value
  void updateEntryValue(String id, String? value) {
    final index = _entries.indexWhere((entry) => entry.id == id);
    if (index != -1) {
      final originalEntry = _entries[index].definition;
      final updatedEntry =
          LibraryHomeEntryWithValue.fromEntry(originalEntry, value);

      _entries[index] = updatedEntry;
      _saveEntries();
      notifyListeners();
    }
  }

  /// Saves entries to storage
  Future<void> _saveEntries() async {
    try {
      final List<String> serializedEntries =
          _entries.map((e) => e.serialize()).toList();
      final String orderJson = jsonEncode(serializedEntries);
      await settingsManager.setValue(storageKey, orderJson);
    } catch (e, stackTrace) {
      error$('Error saving library home entries: $e\n$stackTrace');
    }
  }
}
