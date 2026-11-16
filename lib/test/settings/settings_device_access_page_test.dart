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
  DeviceConnectionLevel? allConnection,
  DeviceSyncLevel? allSync,
  List<DeviceLibraryGrant> overrides = const [],
  bool administer = false,
}) => GetDeviceGrantsResponse(
  grants: DeviceGrantsInfo(
    control: true,
    allConnection: allConnection,
    allSync: allSync,
    libraries: overrides,
    administer: administer,
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
  final oneChoice = find.byWidgetPredicate((widget) => widget is ComboBox);

  Future<List<SetDeviceGrantsRequest>> pickChoice(
    WidgetTester tester, {
    required GetDeviceGrantsResponse stored,
    required String choice,
  }) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final saved = <SetDeviceGrantsRequest>[];
    await tester.pumpWidget(
      _app(
        SettingsDeviceAccessPage(
          client: _client,
          loadGrants: (_) async => stored,
          saveGrants: (request) async => saved.add(request),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(oneChoice, findsOneWidget);
    await tester.tap(oneChoice);
    await tester.pumpAndSettle();
    await tester.tap(find.text(choice).last);
    await tester.pumpAndSettle();
    return saved;
  }

  for (final (choice, from, connection, sync) in [
    (
      'Not Shared',
      (DeviceConnectionLevel.view, DeviceSyncLevel.pull),
      DeviceConnectionLevel.none,
      DeviceSyncLevel.none,
    ),
    (
      'Read Only',
      (DeviceConnectionLevel.none, DeviceSyncLevel.none),
      DeviceConnectionLevel.view,
      DeviceSyncLevel.pull,
    ),
    (
      'Read and Write',
      (DeviceConnectionLevel.view, DeviceSyncLevel.pull),
      DeviceConnectionLevel.viewEdit,
      DeviceSyncLevel.pullPush,
    ),
  ]) {
    testWidgets('choosing $choice sets both levels in one write', (
      tester,
    ) async {
      final saved = await pickChoice(
        tester,
        stored: _response(
          connection: from.$1,
          sync: from.$2,
          allConnection: DeviceConnectionLevel.view,
          allSync: DeviceSyncLevel.pullPush,
          overrides: const [
            DeviceLibraryGrant(
              libraryId: 'archive-id',
              connection: DeviceConnectionLevel.viewEdit,
              sync: DeviceSyncLevel.pullPush,
            ),
          ],
          administer: true,
        ),
        choice: choice,
      );

      expect(saved, hasLength(1));
      expect(
        saved.single.grants,
        DeviceGrantsChangeSet(
          value: DeviceGrantsInfo(
            control: true,
            allConnection: DeviceConnectionLevel.view,
            allSync: DeviceSyncLevel.pullPush,
            libraries: [
              const DeviceLibraryGrant(
                libraryId: 'archive-id',
                connection: DeviceConnectionLevel.viewEdit,
                sync: DeviceSyncLevel.pullPush,
              ),
              DeviceLibraryGrant(
                libraryId: 'music-id',
                connection: connection,
                sync: sync,
              ),
            ],
            administer: true,
          ),
        ),
      );
    });
  }

  testWidgets('a pair set through the API is shown and not written back', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final saved = <SetDeviceGrantsRequest>[];
    await tester.pumpWidget(
      _app(
        SettingsDeviceAccessPage(
          client: _client,
          loadGrants: (_) async => _response(
            connection: DeviceConnectionLevel.view,
            sync: DeviceSyncLevel.none,
          ),
          saveGrants: (request) async => saved.add(request),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Custom'), findsOneWidget);
    expect(saved, isEmpty);

    // Choosing the shown value again changes nothing.
    await tester.tap(oneChoice);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Custom').last);
    await tester.pumpAndSettle();
    expect(saved, isEmpty);

    await tester.tap(oneChoice);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Read Only').last);
    await tester.pumpAndSettle();
    expect(saved, hasLength(1));
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
