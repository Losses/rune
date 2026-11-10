import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_devices/device_access.dart';
import 'package:rune/screens/settings_devices/settings_device_access.dart';

const _client = ClientSummary(
  alias: 'Living Room Speaker',
  fingerprint: 'fingerprint',
  deviceModel: 'Speaker',
  status: ClientStatus.approved,
);

DeviceLibraryAccess _library({
  DeviceConnectionLevel connection = DeviceConnectionLevel.view,
  DeviceSyncLevel sync = DeviceSyncLevel.none,
}) => DeviceLibraryAccess(
  libraryId: 'music-id',
  alias: 'My Music',
  connection: connection,
  sync: sync,
);

GetDeviceGrantsResponse _response({
  DeviceConnectionLevel connection = DeviceConnectionLevel.view,
  DeviceSyncLevel sync = DeviceSyncLevel.none,
}) => GetDeviceGrantsResponse(
  grants: const DeviceGrantsInfo(
    control: true,
    allConnection: null,
    allSync: null,
    libraries: [],
    administer: false,
  ),
  libraries: [_library(connection: connection, sync: sync)],
);

Widget _app(Widget child) {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  addTearDown(sizes.dispose);
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
      ChangeNotifierProvider<ResponsiveProvider>(
        create: (_) => ResponsiveProvider(sizes, () async => null),
      ),
    ],
    child: FluentApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: child,
    ),
  );
}

void main() {
  testWidgets('changing Connection preserves Sync in the saved grant', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    SetDeviceGrantsRequest? saved;
    await tester.pumpWidget(
      _app(
        SettingsDeviceAccessPage(
          client: _client,
          loadGrants: (_) async => _response(sync: DeviceSyncLevel.pull),
          saveGrants: (request) async => saved = request,
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byType(ComboBox<DeviceConnectionLevel>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('View & Edit').last);
    await tester.pumpAndSettle();

    expect(
      saved!.libraryGrants.single,
      const DeviceLibraryGrant(
        libraryId: 'music-id',
        connection: DeviceConnectionLevel.viewEdit,
        sync: DeviceSyncLevel.pull,
      ),
    );
  });

  testWidgets('changing Sync preserves Connection in the saved grant', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    SetDeviceGrantsRequest? saved;
    await tester.pumpWidget(
      _app(
        SettingsDeviceAccessPage(
          client: _client,
          loadGrants: (_) async =>
              _response(connection: DeviceConnectionLevel.viewEdit),
          saveGrants: (request) async => saved = request,
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byType(ComboBox<DeviceSyncLevel>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Pull & Push').last);
    await tester.pumpAndSettle();

    expect(
      saved!.libraryGrants.single,
      const DeviceLibraryGrant(
        libraryId: 'music-id',
        connection: DeviceConnectionLevel.viewEdit,
        sync: DeviceSyncLevel.pullPush,
      ),
    );
  });

  testWidgets('device summaries identify Guest and My Device presets', (
    tester,
  ) async {
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) {
            final strings = AppLocalizations.of(context)!;
            expect(
              deviceAccessSummary(
                const DeviceGrantsInfo(
                  control: true,
                  allConnection: DeviceConnectionLevel.view,
                  allSync: DeviceSyncLevel.none,
                  libraries: [],
                  administer: false,
                ),
                strings,
              ),
              'Guest',
            );
            expect(
              deviceAccessSummary(
                const DeviceGrantsInfo(
                  control: true,
                  allConnection: DeviceConnectionLevel.viewEdit,
                  allSync: DeviceSyncLevel.pullPush,
                  libraries: [],
                  administer: false,
                ),
                strings,
              ),
              'My Device',
            );
            return const SizedBox();
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
  });
}
