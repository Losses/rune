import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';

import 'e2e_library_flow_test.dart' as lifecycle;
import 'runtime_subsonic_test.dart' as subsonic;

const remoteTitles = ['Remote A 440', 'Remote B 660', 'Remote C 880'];

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final fixtureDir = Platform.environment['RUNE_SUBSONIC_FIXTURE_DIR'];
  final localFolder = Platform.environment['E2E_LIBRARY_A'];
  if (fixtureDir == null ||
      fixtureDir.isEmpty ||
      localFolder == null ||
      localFolder.isEmpty) {
    throw StateError(
      'RUNE_SUBSONIC_FIXTURE_DIR and E2E_LIBRARY_A are required',
    );
  }

  testWidgets(
    'a Subsonic library opened after a local library offers its tracks from home',
    (tester) async {
      final fixture = subsonic.readSubsonicFixture(fixtureDir);

      app.main(const []);
      await lifecycle.waitFor(
        tester,
        () => find.byType(FluentApp).evaluate().isNotEmpty,
        description: 'the real app to finish startup',
      );
      await lifecycle.expectVisible(
        tester,
        find.byType(WelcomePage),
        description: 'Welcome for a fresh profile',
      );

      await lifecycle.addLibrary(tester, localFolder, expectLibraryHome: true);
      final openState = tester
          .element(find.byType(LibraryHomePage))
          .read<LibraryOpenStateProvider>();
      await lifecycle.openTracks(tester);
      await lifecycle.expectVisible(
        tester,
        find.textContaining('Startup A'),
        description: 'the track of the local library',
      );

      await lifecycle.openLibrarySettings(tester);
      await subsonic.addSubsonicLibrary(tester, fixture);
      await lifecycle.waitFor(
        tester,
        () =>
            openState.openRecord?.kind is LibraryRecordKindSummarySubsonic &&
            find.byType(LibraryHomePage).evaluate().isNotEmpty &&
            $router.path == '/library',
        description: 'the Subsonic library to open on its library home',
      );

      await lifecycle.openTracks(tester);
      for (final title in remoteTitles) {
        await lifecycle.expectVisible(
          tester,
          find.text(title),
          description: 'the Subsonic track $title',
        );
      }
    },
    timeout: const Timeout(Duration(minutes: 10)),
  );
}
