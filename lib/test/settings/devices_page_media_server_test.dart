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
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';
import 'package:rune/widgets/settings/settings_box_toggle.dart';

const _mediaServer = 'Media Server';
const _broadcast = 'Enable Broadcast';
const _remoteControl = 'Allow Remote Control';
const _sync = 'Allow Paired Sync';

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
}) => GetSyncSettingsResponse(
  requestId: 'loaded',
  mediaServer: mediaServer,
  broadcast: broadcast,
  acceptRemoteControl: acceptRemoteControl,
  acceptSync: acceptSync,
  missingHostFile: MissingHostFilePolicy.keepAsRemote,
);

/// The four values a settings message carries for the Media Server group, in
/// the order the page shows them.
List<Object> _values(GetSyncSettingsResponse settings) => [
  settings.mediaServer,
  settings.broadcast,
  settings.acceptRemoteControl,
  settings.acceptSync,
  settings.missingHostFile,
];

ToggleSwitch _switch(WidgetTester tester, String title) =>
    tester.widget<ToggleSwitch>(_switchFinder(title));

Finder _switchFinder(String title) => find.descendant(
  of: find.widgetWithText(SettingsBoxToggle, title),
  matching: find.byType(ToggleSwitch),
);

Future<void> _pumpPage(
  WidgetTester tester, {
  required Future<GetSyncSettingsResponse> Function() load,
  required Future<void> Function(GetSyncSettingsResponse) save,
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
    storage = await Directory.systemTemp.createTemp('rune-devices-page-test');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('devices-page-test');
  });
  tearDownAll(() async {
    if (!Platform.isWindows) await storage.delete(recursive: true);
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
    final saved = <GetSyncSettingsResponse>[];
    await _pumpPage(
      tester,
      load: () async =>
          _settings(mediaServer: false, acceptRemoteControl: true),
      save: (settings) async => saved.add(settings),
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
    final loaded = _settings();
    final changes = {
      _mediaServer: loaded.copyWith(mediaServer: false),
      _broadcast: loaded.copyWith(broadcast: false),
      _remoteControl: loaded.copyWith(acceptRemoteControl: true),
      _sync: loaded.copyWith(acceptSync: false),
    };

    for (final MapEntry(key: title, value: expected) in changes.entries) {
      var stored = loaded;
      final saved = <GetSyncSettingsResponse>[];
      await _pumpPage(
        tester,
        load: () async => stored,
        save: (settings) async {
          saved.add(settings);
          stored = settings;
        },
      );

      await tester.ensureVisible(_switchFinder(title));
      await tester.tap(_switchFinder(title));
      await tester.pumpAndSettle();

      expect(saved.map(_values), [_values(expected)], reason: title);
      expect(
        [
          _mediaServer,
          _broadcast,
          _remoteControl,
          _sync,
        ].map((title) => _switch(tester, title).checked),
        _values(expected).take(4),
        reason: title,
      );

      // The next round starts from a new page with the loaded settings.
      await tester.pumpWidget(const SizedBox());
    }
  });

  testWidgets('a failed save shows the error and keeps the loaded value', (
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

    await tester.ensureVisible(_switchFinder(_remoteControl));
    await tester.tap(_switchFinder(_remoteControl));
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byType(ContentDialog),
        matching: find.text(_remoteControl),
      ),
      findsOneWidget,
    );
    expect(find.textContaining('settings file is read-only'), findsOneWidget);
    expect(_switch(tester, _remoteControl).checked, isFalse);
    expect(_switch(tester, _mediaServer).checked, isTrue);
    expect(_switch(tester, _broadcast).checked, isTrue);
    expect(_switch(tester, _sync).checked, isTrue);
    expect(loads, 1);
  });
}
