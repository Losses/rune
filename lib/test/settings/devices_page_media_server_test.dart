import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_devices/settings_devices.dart';
import 'package:rune/utils/api/set_sync_settings.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';
import 'package:rune/widgets/settings/settings_box_toggle.dart';
import '../settings_folder.dart';

const _mediaServer = 'Media Server';
const _broadcast = 'Enable Broadcast';
const _remoteControl = 'Allow Remote Control';
const _sync = 'Allow Paired Sync';
const _autoDeviceSync = 'Sync with Devices Automatically';

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

GetSyncSettingsResponse _settings({
  bool mediaServer = true,
  bool broadcast = true,
  bool acceptRemoteControl = false,
  bool acceptSync = true,
  bool autoDeviceSync = true,
}) => GetSyncSettingsResponse(
  requestId: 'loaded',
  mediaServer: mediaServer,
  broadcast: broadcast,
  acceptRemoteControl: acceptRemoteControl,
  acceptSync: acceptSync,
  autoDeviceSync: autoDeviceSync,
  autoShareSync: true,
  missingHostFile: MissingHostFilePolicy.keepAsRemote,
);

/// Every field a change can carry, by name. A null value is a field the save
/// leaves as the hub holds it.
Map<String, Object?> _fields(SyncSettingsChange change) => {
  'mediaServer': change.mediaServer,
  'broadcast': change.broadcast,
  'acceptRemoteControl': change.acceptRemoteControl,
  'acceptSync': change.acceptSync,
  'autoDeviceSync': change.autoDeviceSync,
  'autoShareSync': change.autoShareSync,
  'missingHostFile': change.missingHostFile,
};

/// The fields of a change that sets [field] to [value] and nothing else.
Map<String, Object?> _only(String field, Object value) => {
  for (final name in _fields(const SyncSettingsChange()).keys)
    name: name == field ? value : null,
};

const _titles = [_mediaServer, _broadcast, _remoteControl, _sync];

/// What the four switches show, in the order of [_titles].
List<bool> _shown(WidgetTester tester) =>
    _titles.map((title) => _switch(tester, title).checked).toList();

Future<void> _tap(WidgetTester tester, String title) async {
  await tester.ensureVisible(_switchFinder(title));
  await tester.tap(_switchFinder(title));
}

ToggleSwitch _switch(WidgetTester tester, String title) =>
    tester.widget<ToggleSwitch>(_switchFinder(title));

Finder _switchFinder(String title) => find.descendant(
  of: find.widgetWithText(SettingsBoxToggle, title),
  matching: find.byType(ToggleSwitch),
);

Future<void> _pumpPage(
  WidgetTester tester, {
  required Future<GetSyncSettingsResponse> Function() load,
  required Future<void> Function(SyncSettingsChange) save,
}) async {
  final registry = LibraryRegistryProvider(
    loadRecords: () async => const ListLibraryRecordsResponse(
      records: [],
      invalidRecords: [],
      lastOpenRecord: null,
    ),
  );
  final broadcast = _Broadcast();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(registry.dispose);
  addTearDown(broadcast.dispose);
  addTearDown(responsive.dispose);
  addTearDown(sizes.dispose);
  await registry.ready;

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: SettingsDevicesPage(
          loadSyncSettings: load,
          saveSyncSettings: save,
        ),
      ),
    ),
  );
  // Lets the screen size provider's debounce fire while the page is mounted.
  await tester.pump(const Duration(seconds: 1));
  await tester.pumpAndSettle();
}

void main() {
  late Directory storage;

  setUpAll(() async {
    storage = await emptySettingsFolder('rune-devices-page-test');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('devices-page-test');
  });

  testWidgets('the Media Server switches show the loaded settings', (
    tester,
  ) async {
    await _pumpPage(
      tester,
      load: () async => _settings(
        broadcast: false,
        acceptRemoteControl: true,
        acceptSync: false,
      ),
      save: (_) async {},
    );

    expect(_switch(tester, _mediaServer).checked, isTrue);
    expect(_switch(tester, _broadcast).checked, isFalse);
    expect(_switch(tester, _remoteControl).checked, isTrue);
    expect(_switch(tester, _sync).checked, isFalse);
    for (final title in [_mediaServer, _broadcast, _remoteControl, _sync]) {
      expect(_switch(tester, title).onChanged, isNotNull, reason: title);
    }
  });

  testWidgets('the permissions are disabled while the Media Server is off', (
    tester,
  ) async {
    final saved = <SyncSettingsChange>[];
    await _pumpPage(
      tester,
      load: () async =>
          _settings(mediaServer: false, acceptRemoteControl: true),
      save: (change) async => saved.add(change),
    );

    expect(_switch(tester, _mediaServer).checked, isFalse);
    expect(_switch(tester, _mediaServer).onChanged, isNotNull);
    for (final title in [_broadcast, _remoteControl, _sync]) {
      expect(_switch(tester, title).onChanged, isNull, reason: title);
      await tester.ensureVisible(_switchFinder(title));
      await tester.tap(_switchFinder(title));
      await tester.pumpAndSettle();
    }
    expect(saved, isEmpty);
    // A disabled permission still shows the value it will have once the
    // server is switched on again.
    expect(_switch(tester, _broadcast).checked, isTrue);
    expect(_switch(tester, _remoteControl).checked, isTrue);
    expect(_switch(tester, _sync).checked, isTrue);
  });

  testWidgets('each switch saves its own field and leaves the others', (
    tester,
  ) async {
    // For each switch: the change it sends, and what the four switches show
    // once it is saved.
    final changes = {
      _mediaServer: (_only('mediaServer', false), [false, true, false, true]),
      _broadcast: (_only('broadcast', false), [true, false, false, true]),
      _remoteControl: (
        _only('acceptRemoteControl', true),
        [true, true, true, true],
      ),
      _sync: (_only('acceptSync', false), [true, true, false, false]),
    };

    for (final MapEntry(key: title, value: (expected, shown))
        in changes.entries) {
      // What happens between the page and the hub, in order.
      final calls = <String>[];
      final saved = <SyncSettingsChange>[];
      await _pumpPage(
        tester,
        load: () async {
          calls.add('load');
          return _settings();
        },
        save: (change) async {
          calls.add('save');
          saved.add(change);
        },
      );
      expect(calls, ['load'], reason: title);

      await _tap(tester, title);
      await tester.pumpAndSettle();

      // The page shows what it saved without asking the hub before or after.
      expect(calls, ['load', 'save'], reason: title);
      expect(saved.map(_fields), [expected], reason: title);
      expect(_shown(tester), shown, reason: title);

      // The next round starts from a new page with the loaded settings.
      await tester.pumpWidget(const SizedBox());
    }
  });

  testWidgets('the switches are disabled while a save is pending', (
    tester,
  ) async {
    final pending = Completer<void>();
    final saved = <SyncSettingsChange>[];
    await _pumpPage(
      tester,
      load: () async => _settings(),
      save: (change) {
        saved.add(change);
        return pending.future;
      },
    );

    await _tap(tester, _remoteControl);
    await tester.pump();
    expect(saved, hasLength(1));
    for (final title in _titles) {
      expect(_switch(tester, title).onChanged, isNull, reason: title);
    }
    for (final title in _titles) {
      await tester.tap(_switchFinder(title), warnIfMissed: false);
      await tester.pump();
    }
    expect(saved, hasLength(1), reason: 'a tap during a save sent a request');
    // The value is shown only once the hub has accepted it.
    expect(_shown(tester), [true, true, false, true]);

    pending.complete();
    await tester.pumpAndSettle();
    for (final title in _titles) {
      expect(_switch(tester, title).onChanged, isNotNull, reason: title);
    }
    expect(_fields(saved.single), _only('acceptRemoteControl', true));
    expect(_shown(tester), [true, true, true, true]);
  });

  testWidgets(
    'tapping one switch sends a change in which every other field is null',
    (tester) async {
      // What the hub holds. It applies a save the way the hub does: only the
      // fields the change carries are replaced.
      var stored = _settings();
      final saved = <SyncSettingsChange>[];
      await _pumpPage(
        tester,
        load: () async => stored,
        save: (change) async {
          saved.add(change);
          stored = change.applyTo(stored);
        },
      );

      // Another writer changes every other setting after this page loaded.
      stored = GetSyncSettingsResponse(
        requestId: 'changed elsewhere',
        mediaServer: !stored.mediaServer,
        broadcast: !stored.broadcast,
        acceptRemoteControl: stored.acceptRemoteControl,
        acceptSync: !stored.acceptSync,
        autoDeviceSync: !stored.autoDeviceSync,
        autoShareSync: !stored.autoShareSync,
        missingHostFile: MissingHostFilePolicy.removeEverywhere,
      );
      final changedElsewhere = stored;
      await _tap(tester, _remoteControl);
      await tester.pumpAndSettle();

      expect(saved.map(_fields), [_only('acceptRemoteControl', true)]);
      expect(stored.acceptRemoteControl, isTrue);
      expect(stored.mediaServer, changedElsewhere.mediaServer);
      expect(stored.broadcast, changedElsewhere.broadcast);
      expect(stored.acceptSync, changedElsewhere.acceptSync);
      expect(stored.autoDeviceSync, changedElsewhere.autoDeviceSync);
      expect(stored.autoShareSync, changedElsewhere.autoShareSync);
      expect(stored.missingHostFile, changedElsewhere.missingHostFile);
    },
  );

  testWidgets('a failed save shows the error and keeps the shown values', (
    tester,
  ) async {
    var loads = 0;
    await _pumpPage(
      tester,
      load: () async {
        loads++;
        return _settings();
      },
      save: (_) async => throw Exception('settings file is read-only'),
    );
    expect(loads, 1);

    await _tap(tester, _remoteControl);
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byType(ContentDialog),
        matching: find.text(_remoteControl),
      ),
      findsOneWidget,
    );
    expect(find.textContaining('settings file is read-only'), findsOneWidget);
    expect(_shown(tester), [true, true, false, true]);
    for (final title in _titles) {
      expect(_switch(tester, title).onChanged, isNotNull, reason: title);
    }
    expect(loads, 1, reason: 'a save loaded the settings');
  });

  testWidgets('automatic device sync does not depend on the Media Server', (
    tester,
  ) async {
    final saved = <SyncSettingsChange>[];
    await _pumpPage(
      tester,
      load: () async => _settings(mediaServer: false),
      save: (change) async => saved.add(change),
    );
    expect(_switch(tester, _sync).onChanged, isNull);
    expect(_switch(tester, _autoDeviceSync).checked, isTrue);
    expect(_switch(tester, _autoDeviceSync).onChanged, isNotNull);

    await _tap(tester, _autoDeviceSync);
    await tester.pumpAndSettle();

    expect(saved.map(_fields), [_only('autoDeviceSync', false)]);
    expect(_switch(tester, _autoDeviceSync).checked, isFalse);
  });
}
