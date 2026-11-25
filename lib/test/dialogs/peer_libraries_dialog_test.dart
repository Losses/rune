import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/library_registry.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/settings_server/widgets/peer_libraries_dialog.dart';
import '../../utils/api/request_failure.dart';

void main() {
  testWidgets(
    'selected peer library exposes Add and reports errors in a dialog',
    (tester) async {
      await _pumpDialog(
        tester,
        _Harness(
          library: const PeerLibrary(
            libraryId: 'shared-library',
            alias: 'Shared Music',
            added: false,
            connection: DeviceConnectionLevel.view,
            sync: DeviceSyncLevel.pull,
          ),
          addLibrary: (_, _) async => throw const RequestFailure(
            'AddRemoteLibraryRequest',
            RequestErrorKind.invalidInput,
            'Invalid library id',
          ),
        ),
      );

      await tester.tap(find.text('Shared Music'));
      await tester.pumpAndSettle();
      expect(find.text('Add'), findsOneWidget);

      await tester.tap(find.text('Add'));
      await tester.pumpAndSettle();
      expect(find.byType(ContentDialog), findsNWidgets(2));
      expect(find.text('The request is invalid.'), findsOneWidget);
      expect(find.byType(InfoBar), findsNothing);
    },
  );

  testWidgets(
    'a peer library without sync access is not shared and cannot be added',
    (tester) async {
      var addCalls = 0;
      await _pumpDialog(
        tester,
        _Harness(
          library: const PeerLibrary(
            libraryId: 'unshared-library',
            alias: 'Unshared Music',
            added: false,
            connection: DeviceConnectionLevel.view,
            sync: DeviceSyncLevel.none,
          ),
          addLibrary: (_, _) async {
            addCalls++;
            throw const RequestFailure(
              'AddRemoteLibraryRequest',
              RequestErrorKind.permissionDenied,
              'Not shared',
            );
          },
        ),
      );

      await tester.tap(find.text('Unshared Music'));
      await tester.pumpAndSettle();
      expect(find.text('Not Shared'), findsOneWidget);

      final addButton = find.widgetWithText(Button, 'Add');
      expect(tester.widget<Button>(addButton).onPressed, isNull);
      await tester.tap(addButton, warnIfMissed: false);
      await tester.pumpAndSettle();
      expect(addCalls, 0);
    },
  );
}

Future<void> _pumpDialog(WidgetTester tester, Widget harness) async {
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(
          value: _Libraries(),
        ),
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: harness,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

class _Harness extends StatelessWidget {
  const _Harness({required this.library, required this.addLibrary});

  final PeerLibrary library;
  final PeerLibraryAdder addLibrary;

  @override
  Widget build(BuildContext context) => PeerLibrariesDialog(
    client: const ClientSummary(
      alias: 'Bedroom',
      fingerprint: 'peer-fingerprint',
      deviceModel: 'Desktop',
      status: ClientStatus.approved,
    ),
    close: () {},
    loadLibraries: (_) async => ListPeerLibrariesResponse(libraries: [library]),
    addLibrary: addLibrary,
  );
}

class _Libraries extends ChangeNotifier implements LibraryRegistryProvider {
  @override
  List<LibraryRecordSummary> get records => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
