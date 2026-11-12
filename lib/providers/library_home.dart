import 'dart:convert';

import 'package:fluent_ui/fluent_ui.dart';

import '../bindings/bindings.dart';
import '../utils/rune_log.dart';
import '../utils/settings_manager.dart';
import '../screens/settings_library_home/constants/library_home_items.dart';

final SettingsManager settingsManager = SettingsManager();

class LibraryHomeEntryWithValue {
  final LibraryHomeDomain domain;
  final LibraryHomeSetting setting;
  final LibraryHomeEntry definition;

  LibraryHomeEntryWithValue({
    required this.domain,
    required this.setting,
    required this.definition,
  });

  factory LibraryHomeEntryWithValue.fromEntry(
    LibraryHomeEntry entry, [
    LibraryHomeSetting? setting,
  ]) {
    return LibraryHomeEntryWithValue(
      domain: entry.domain,
      setting: setting ?? entry.defaultSetting,
      definition: entry,
    );
  }

  String serialize() {
    return '${domain.name}::${_storedValue(domain, setting)}';
  }

  static LibraryHomeEntryWithValue? deserialize(
    String serialized,
    List<LibraryHomeEntry> availableEntries,
  ) {
    final parts = serialized.split('::');
    final domain = LibraryHomeDomain.values.byName(parts[0]);
    if (parts.length != 2) {
      throw FormatException('Invalid home entry: $serialized');
    }

    final originalEntry = availableEntries.firstWhere(
      (entry) => entry.domain == domain,
      orElse: () => throw Exception('Entry not found: $domain'),
    );

    return LibraryHomeEntryWithValue(
      domain: domain,
      setting: _settingFromStoredValue(domain, parts[1]),
      definition: originalEntry,
    );
  }

  static LibraryHomeSetting _settingFromStoredValue(
    LibraryHomeDomain domain,
    String storedValue,
  ) {
    if (storedValue == 'disable') return libraryHomeDisabled;
    final mode = switch (storedValue) {
      'newest' => LibraryHomeListMode.reverse,
      'oldest' => LibraryHomeListMode.forward,
      'random' => LibraryHomeListMode.random,
      'name' => LibraryHomeListMode.name,
      'forward' => LibraryHomeListMode.forward,
      'reverse' => LibraryHomeListMode.reverse,
      _ => null,
    };
    if (mode != null) {
      final query = switch (domain) {
        LibraryHomeDomain.artists => LibraryHomeQueryArtists(value: mode),
        LibraryHomeDomain.albums => LibraryHomeQueryAlbums(value: mode),
        LibraryHomeDomain.playlists => LibraryHomeQueryPlaylists(value: mode),
        LibraryHomeDomain.tracks => LibraryHomeQueryTracks(value: mode),
        _ => throw FormatException('Mode is invalid for $domain'),
      };
      return LibraryHomeSettingQuery(query);
    }
    if (storedValue == 'enable') {
      return switch (domain) {
        LibraryHomeDomain.liked => const LibraryHomeSettingQuery(
          LibraryHomeQueryLiked(),
        ),
        LibraryHomeDomain.most => const LibraryHomeSettingQuery(
          LibraryHomeQueryMost(),
        ),
        _ => throw FormatException('Enable is invalid for $domain'),
      };
    }
    throw FormatException('Unknown home setting: $storedValue');
  }

  static String _storedValue(
    LibraryHomeDomain domain,
    LibraryHomeSetting setting,
  ) {
    if (setting is LibraryHomeSettingDisabled) return 'disable';
    final query = (setting as LibraryHomeSettingQuery).query;
    return switch ((domain, query)) {
      (LibraryHomeDomain.artists, LibraryHomeQueryArtists(:final value)) =>
        _storedMode(value),
      (LibraryHomeDomain.albums, LibraryHomeQueryAlbums(:final value)) =>
        _storedMode(value),
      (LibraryHomeDomain.playlists, LibraryHomeQueryPlaylists(:final value)) =>
        _storedMode(value),
      (LibraryHomeDomain.tracks, LibraryHomeQueryTracks(:final value)) =>
        _storedMode(value),
      (LibraryHomeDomain.liked, LibraryHomeQueryLiked()) => 'enable',
      (LibraryHomeDomain.most, LibraryHomeQueryMost()) => 'enable',
      _ => throw StateError('Query does not match $domain'),
    };
  }

  static String _storedMode(LibraryHomeListMode mode) => switch (mode) {
    LibraryHomeListMode.name => 'name',
    LibraryHomeListMode.forward => 'oldest',
    LibraryHomeListMode.reverse => 'newest',
    LibraryHomeListMode.random => 'random',
  };

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
    final uniqueEntries = <LibraryHomeDomain, LibraryHomeEntryWithValue>{};

    // Add entries, overwriting duplicates to keep the last one
    for (final entry in newEntries) {
      uniqueEntries[entry.domain] = entry;
    }

    // Ensure all required entries exist, filling missing ones with default values
    for (final item in libraryHomeItems) {
      if (!uniqueEntries.containsKey(item.domain)) {
        uniqueEntries[item.domain] = LibraryHomeEntryWithValue.fromEntry(item);
      }
    }

    // Remove entries not in libraryHomeItems
    uniqueEntries.removeWhere(
      (key, _) => !libraryHomeItems.any((item) => item.domain == key),
    );

    // Update the list of entries
    _entries
      ..clear()
      ..addAll(uniqueEntries.values);
  }

  /// Loads entries from storage
  Future<void> _loadEntries() async {
    try {
      final String? storedOrderJson = await settingsManager.getValue<String>(
        storageKey,
      );

      if (storedOrderJson != null) {
        // Parse stored data
        final List<LibraryHomeEntryWithValue> loadedEntries =
            _parseStoredEntries(storedOrderJson);
        _safelyUpdateEntries(loadedEntries);
      } else {
        // Use default values if no stored data
        _safelyUpdateEntries(
          libraryHomeItems
              .map((item) => LibraryHomeEntryWithValue.fromEntry(item))
              .toList(),
        );
      }
    } catch (e, stackTrace) {
      error$('Error loading library home entries: $e\n$stackTrace');
      // Use default values on error
      _safelyUpdateEntries(
        libraryHomeItems
            .map((item) => LibraryHomeEntryWithValue.fromEntry(item))
            .toList(),
      );
    } finally {
      notifyListeners();
    }
  }

  /// Parses stored entries
  List<LibraryHomeEntryWithValue> _parseStoredEntries(String storedOrderJson) {
    final List<dynamic> storedOrderDynamic = jsonDecode(storedOrderJson);
    final List<String> storedSerializedEntries = List<String>.from(
      storedOrderDynamic,
    );
    final List<LibraryHomeEntryWithValue> loadedEntries = [];

    for (final serialized in storedSerializedEntries) {
      try {
        final entry = LibraryHomeEntryWithValue.deserialize(
          serialized,
          libraryHomeItems,
        );
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
  void updateEntryValue(LibraryHomeDomain domain, LibraryHomeSetting setting) {
    final index = _entries.indexWhere((entry) => entry.domain == domain);
    if (index != -1) {
      final originalEntry = _entries[index].definition;
      final updatedEntry = LibraryHomeEntryWithValue.fromEntry(
        originalEntry,
        setting,
      );

      _entries[index] = updatedEntry;
      _saveEntries();
      notifyListeners();
    }
  }

  /// Saves entries to storage
  Future<void> _saveEntries() async {
    try {
      final List<String> serializedEntries = _entries
          .map((e) => e.serialize())
          .toList();
      final String orderJson = jsonEncode(serializedEntries);
      await settingsManager.setValue(storageKey, orderJson);
    } catch (e, stackTrace) {
      error$('Error saving library home entries: $e\n$stackTrace');
    }
  }
}
