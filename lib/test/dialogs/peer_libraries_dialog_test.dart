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
      final registry = _Libraries();
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider(
              create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
            ),
          ],
          child: FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: const _Harness(),
          ),
        ),
      );

      await tester.pumpAndSettle();
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
}

class _Harness extends StatelessWidget {
  const _Harness();

  @override
  Widget build(BuildContext context) => PeerLibrariesDialog(
    client: const ClientSummary(
      alias: 'Bedroom',
      fingerprint: 'peer-fingerprint',
      deviceModel: 'Desktop',
      status: ClientStatus.approved,
    ),
    close: () {},
    loadLibraries: (_) async => const ListPeerLibrariesResponse(
      libraries: [
        PeerLibrary(
          libraryId: 'shared-library',
          alias: 'Shared Music',
          added: false,
          connection: DeviceConnectionLevel.view,
          sync: DeviceSyncLevel.pull,
        ),
      ],
    ),
    addLibrary: (_, _) async => throw const RequestFailure(
      'AddRemoteLibraryRequest',
      RequestErrorKind.invalidInput,
      'Invalid library id',
    ),
  );
}

class _Libraries extends ChangeNotifier implements LibraryRegistryProvider {
  @override
  List<LibraryRecordSummary> get records => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
